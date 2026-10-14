//! Integration tests for compiler-synthesized Sans-I/O coroutine decoders.
//!
//! Verifies zero conflict with:
//! 1. Tokio async event loops and network range fetchers (zero blocking threads, zero block_on).
//! 2. C# / C FFI callers with synchronous push loops and caller-owned row buffers.
//! 3. Backpressure and multi-batch yield pausing.
//! 4. Cooperative cancellation (`StopToken`, `Pulse`).
//! 5. `ReadAt` source integration via `drive_ranges`.
//! 6. Sequential streaming fallback (`read_exact`).

#![cfg(feature = "std")]
#![allow(clippy::chunks_exact_to_as_chunks)]

use almost_enough::{Cancellable, Stopper};
use zenpixels::{PixelDescriptor, PixelSlice, PixelSliceMut};

use zencodec::decode::{
    ByteRange, CoroutineContext, CoroutineDecoder, DecodeRowSink, IncrementalDecode, OutputInfo,
    PushStatus, SinkError, SliceReadAt, SourceError, drive_ranges,
};
use zencodec::{ImageFormat, ImageInfo};

/// Builds a test MiniTile image file:
/// - 0..16: Header (Magic "MTIL", width 4, height 4, index_offset 80)
/// - 16..32: Padding
/// - 32..64: Tile 0 data (4x2 pixels RGBA8, red)
/// - 64..80: Padding
/// - 80..96: Index Table: [tile0_off: 32, tile0_len: 32, tile1_off: 96, tile1_len: 32]
/// - 96..128: Tile 1 data (4x2 pixels RGBA8, blue)
fn build_minitile_file() -> Vec<u8> {
    let mut file = vec![0u8; 128];

    // Header: offset 0..16
    file[0..4].copy_from_slice(b"MTIL");
    file[4..8].copy_from_slice(&4u32.to_le_bytes()); // width = 4
    file[8..12].copy_from_slice(&4u32.to_le_bytes()); // height = 4
    file[12..16].copy_from_slice(&80u32.to_le_bytes()); // index_offset = 80

    // Tile 0: offset 32..64 (4x2 pixels RGBA8, filled with red 255, 0, 0, 255)
    for p in file[32..64].chunks_exact_mut(4) {
        p.copy_from_slice(&[255, 0, 0, 255]);
    }

    // Index Table: offset 80..96
    file[80..84].copy_from_slice(&32u32.to_le_bytes()); // tile 0 offset
    file[84..88].copy_from_slice(&32u32.to_le_bytes()); // tile 0 len
    file[88..92].copy_from_slice(&96u32.to_le_bytes()); // tile 1 offset
    file[92..96].copy_from_slice(&32u32.to_le_bytes()); // tile 1 len

    // Tile 1: offset 96..128 (4x2 pixels RGBA8, filled with blue 0, 0, 255, 255)
    for p in file[96..128].chunks_exact_mut(4) {
        p.copy_from_slice(&[0, 0, 255, 255]);
    }

    file
}

fn create_minitile_coroutine() -> CoroutineDecoder<
    impl core::future::Future<Output = Result<OutputInfo, SourceError>> + Send + 'static,
    SourceError,
> {
    CoroutineDecoder::new(|mut ctx: CoroutineContext| async move {
        // 1. Read header
        let mut header = [0u8; 16];
        ctx.read_exact_at(0, &mut header).await?;
        assert_eq!(&header[0..4], b"MTIL");
        let width = u32::from_le_bytes(header[4..8].try_into().unwrap());
        let height = u32::from_le_bytes(header[8..12].try_into().unwrap());
        let index_offset = u32::from_le_bytes(header[12..16].try_into().unwrap()) as u64;

        ctx.set_info(ImageInfo::new(width, height, ImageFormat::Png));
        ctx.set_output_descriptor(PixelDescriptor::RGBA8_SRGB);

        // 2. Read index table from end of file (seekable read)
        let mut index = [0u8; 16];
        ctx.read_exact_at(index_offset, &mut index).await?;
        let t0_off = u32::from_le_bytes(index[0..4].try_into().unwrap()) as u64;
        let t0_len = u32::from_le_bytes(index[4..8].try_into().unwrap()) as usize;
        let t1_off = u32::from_le_bytes(index[8..12].try_into().unwrap()) as u64;
        let t1_len = u32::from_le_bytes(index[12..16].try_into().unwrap()) as usize;

        // 3. Read both tiles in parallel using read_many_at
        let mut t0_data = vec![0u8; t0_len];
        let mut t1_data = vec![0u8; t1_len];
        let mut reqs = [
            (t0_off, t0_data.as_mut_slice()),
            (t1_off, t1_data.as_mut_slice()),
        ];
        ctx.read_many_at(&mut reqs).await?;

        // 4. Yield Tile 0 (rows 0..2)
        let slice0 = PixelSlice::new(
            &t0_data,
            width,
            2,
            width as usize * 4,
            PixelDescriptor::RGBA8_SRGB,
        )
        .unwrap();
        ctx.yield_scanlines(0, &slice0).await?;

        // 5. Yield Tile 1 (rows 2..4)
        let slice1 = PixelSlice::new(
            &t1_data,
            width,
            2,
            width as usize * 4,
            PixelDescriptor::RGBA8_SRGB,
        )
        .unwrap();
        ctx.yield_scanlines(2, &slice1).await?;

        let res: Result<OutputInfo, SourceError> = Ok(OutputInfo::full_decode(
            width,
            height,
            PixelDescriptor::RGBA8_SRGB,
        ));
        res
    })
}

// ===========================================================================
// Test 1: Tokio Async Event Loop & Network Range Multiplexing
// ===========================================================================

#[tokio::test]
async fn test_tokio_async_network_loop() {
    let file = build_minitile_file();
    let mut decoder = create_minitile_coroutine();

    let mut decoded_rows = [0u8; 4 * 4 * 4]; // 4x4 RGBA8
    let mut row_count = 0;

    // Start by pushing initial chunk or probing
    let mut outcome = decoder.push_range(0, &[], false, None).unwrap();

    // Async driver loop: runs directly on Tokio worker thread without spawn_blocking
    while !decoder.is_complete() {
        match outcome.status {
            PushStatus::NeedRanges => {
                let ranges: Vec<ByteRange> = decoder.requested_ranges().to_vec();
                assert!(
                    !ranges.is_empty(),
                    "NeedRanges must specify requested spans"
                );

                // Fetch all requested ranges concurrently via Tokio JoinSet
                let mut set = tokio::task::JoinSet::new();
                for range in ranges {
                    let file_copy = file.clone();
                    set.spawn(async move {
                        let start = range.offset as usize;
                        let end = (range.offset + range.length) as usize;
                        (range.offset, file_copy[start..end].to_vec())
                    });
                }

                let mut fetched_chunks = Vec::new();
                while let Some(res) = set.join_next().await {
                    fetched_chunks.push(res.unwrap());
                }

                // Push each fetched chunk into the decoder synchronously
                for (offset, chunk) in fetched_chunks {
                    outcome = decoder.push_range(offset, &chunk, false, None).unwrap();
                }
            }
            PushStatus::OutputReady | PushStatus::BlockedOnOutput => {
                while let Some((y, slice)) = decoder.pull_batch().unwrap() {
                    let rows = slice.rows();
                    let stride = 4 * 4;
                    for r in 0..rows {
                        let dst_start = (y + r) as usize * stride;
                        decoded_rows[dst_start..dst_start + stride].copy_from_slice(slice.row(r));
                        row_count += 1;
                    }
                }
                // Refresh status after draining
                outcome = decoder.push_range(0, &[], false, None).unwrap();
            }
            PushStatus::NeedMoreInput => {
                panic!("MiniTile uses explicit range requests, should not need sequential input");
            }
            PushStatus::Complete => break,
            _ => break,
        }
    }

    assert_eq!(row_count, 4, "Decoded all 4 rows");
    assert!(decoder.is_complete());

    // Verify Tile 0 (rows 0..2) are red
    for p in decoded_rows[0..32].chunks_exact(4) {
        assert_eq!(p, &[255, 0, 0, 255]);
    }
    // Verify Tile 1 (rows 2..4) are blue
    for p in decoded_rows[32..64].chunks_exact(4) {
        assert_eq!(p, &[0, 0, 255, 255]);
    }
}

// ===========================================================================
// Test 2: C# / C FFI Caller-Owned Buffer & Zero Async Runtime
// ===========================================================================

struct NativeFfiRowSink<'a> {
    buffer: &'a mut [u8],
    width: u32,
    stride: usize,
    rows_written: u32,
}

impl<'a> NativeFfiRowSink<'a> {
    fn new(buffer: &'a mut [u8], width: u32, stride: usize) -> Self {
        Self {
            buffer,
            width,
            stride,
            rows_written: 0,
        }
    }
}

impl DecodeRowSink for NativeFfiRowSink<'_> {
    fn provide_next_buffer(
        &mut self,
        y: u32,
        height: u32,
        width: u32,
        descriptor: PixelDescriptor,
    ) -> Result<PixelSliceMut<'_>, SinkError> {
        assert_eq!(width, self.width);
        let start = y as usize * self.stride;
        let len = height as usize * self.stride;
        self.rows_written += height;
        PixelSliceMut::new(
            &mut self.buffer[start..start + len],
            width,
            height,
            self.stride,
            descriptor,
        )
        .map_err(|e| Box::new(e) as SinkError)
    }
}

#[test]
fn test_csharp_ffi_synchronous_push_and_caller_buffer() {
    let file = build_minitile_file();
    let mut decoder = create_minitile_coroutine();

    // Caller allocates destination memory in unmanaged memory or pinned C# buffer
    let mut csharp_managed_buffer = vec![0u8; 4 * 16]; // 4 rows * 16 bytes stride
    let mut sink = NativeFfiRowSink::new(&mut csharp_managed_buffer, 4, 16);

    // Synchronous FFI push loop: zero Tokio, zero async runtime
    let outcome = decoder.push_range(0, &file[0..16], false, None).unwrap();
    assert!(outcome.metadata.is_header_ready());

    let info = decoder.info().expect("info available after header");
    assert_eq!(info.width, 4);
    assert_eq!(info.height, 4);

    while !decoder.is_complete() {
        let ranges = decoder.requested_ranges();
        if !ranges.is_empty() {
            let req = ranges[0];
            let start = req.offset as usize;
            let end = (req.offset + req.length) as usize;
            let _ = decoder
                .push_range(req.offset, &file[start..end], false, None)
                .unwrap();
        }

        while decoder.pull_batch_into(&mut sink).unwrap().is_some() {}
    }

    assert_eq!(sink.rows_written, 4);
    assert!(decoder.is_complete());

    // Verify buffer written directly by FFI sink
    for p in csharp_managed_buffer[0..32].chunks_exact(4) {
        assert_eq!(p, &[255, 0, 0, 255]);
    }
    for p in csharp_managed_buffer[32..64].chunks_exact(4) {
        assert_eq!(p, &[0, 0, 255, 255]);
    }
}

// ===========================================================================
// Test 3: Backpressure & Multi-Batch Pausing
// ===========================================================================

#[test]
fn test_backpressure_pauses_coroutine_until_pulled() {
    // Coroutine configured with max_pending_batches = 1
    let mut decoder = CoroutineDecoder::with_max_pending_batches(1, |mut ctx| async move {
        let dummy = vec![42u8; 16]; // 4 pixels RGBA8
        let slice = PixelSlice::new(&dummy, 4, 1, 16, PixelDescriptor::RGBA8_SRGB).unwrap();

        // Yield batch 0
        ctx.yield_scanlines(0, &slice).await?;
        // Yield batch 1 (should pause on backpressure if batch 0 not drained)
        ctx.yield_scanlines(1, &slice).await?;
        // Yield batch 2
        ctx.yield_scanlines(2, &slice).await?;

        let res: Result<OutputInfo, SourceError> =
            Ok(OutputInfo::full_decode(4, 3, PixelDescriptor::RGBA8_SRGB));
        res
    });

    // Initial pump
    let outcome = decoder.push_range(0, &[], false, None).unwrap();
    // Batch 0 filled the single pending slot; batch 1 triggered backpressure
    assert_eq!(outcome.status, PushStatus::BlockedOnOutput);

    // Pull batch 0
    let batch0 = decoder.pull_batch().unwrap().expect("batch 0 available");
    assert_eq!(batch0.0, 0);

    // Draining batch 0 allowed the coroutine to advance and yield batch 1!
    let batch1 = decoder.pull_batch().unwrap().expect("batch 1 available");
    assert_eq!(batch1.0, 1);

    // Pull batch 2
    let batch2 = decoder.pull_batch().unwrap().expect("batch 2 available");
    assert_eq!(batch2.0, 2);

    assert!(decoder.is_complete());
}

// ===========================================================================
// Test 4: Cooperative Cancellation
// ===========================================================================

#[test]
fn test_cooperative_cancellation() {
    let stopper = Stopper::new();
    let mut decoder = CoroutineDecoder::new(|mut ctx| async move {
        let mut buf = [0u8; 16];
        ctx.read_exact_at(0, &mut buf).await?;
        // Should stop when cancelled
        ctx.read_exact_at(100, &mut buf).await?;
        let res: Result<OutputInfo, SourceError> =
            Ok(OutputInfo::full_decode(4, 4, PixelDescriptor::RGBA8_SRGB));
        res
    });

    // First push succeeds
    let outcome = decoder
        .push_range(0, &[0u8; 16], false, Some(&stopper))
        .unwrap();
    assert_eq!(outcome.status, PushStatus::NeedRanges);

    // Cancel via Stop token
    stopper.stop();

    // Next push fails with cancellation
    let err = decoder
        .push_range(100, &[0u8; 16], false, Some(&stopper))
        .unwrap_err();
    assert!(
        err.to_string().contains("cancelled"),
        "error should indicate cancellation: {err}"
    );
}

// ===========================================================================
// Test 5: ReadAt Source Integration with drive_ranges
// ===========================================================================

#[test]
fn test_read_at_source_drive_ranges() {
    let file = build_minitile_file();
    let mut source = SliceReadAt::new(&file);
    let mut decoder = create_minitile_coroutine();

    let mut output_buf = vec![0u8; 4 * 16];
    let mut sink = NativeFfiRowSink::new(&mut output_buf, 4, 16);

    let mut outcome = decoder.push_range(0, &[], false, None).unwrap();
    while !decoder.is_complete() {
        if matches!(outcome.status, PushStatus::NeedRanges) {
            let stats =
                drive_ranges(&mut decoder, &mut source, 0, None).expect("drive_ranges succeeds");
            assert!(stats.bytes_fetched > 0);
        }

        while decoder.pull_batch_into(&mut sink).unwrap().is_some() {}
        outcome = decoder.push_range(0, &[], false, None).unwrap();
    }

    assert!(decoder.is_complete());
    assert_eq!(sink.rows_written, 4);

    // Verify output
    for p in output_buf[0..32].chunks_exact(4) {
        assert_eq!(p, &[255, 0, 0, 255]);
    }
    for p in output_buf[32..64].chunks_exact(4) {
        assert_eq!(p, &[0, 0, 255, 255]);
    }
}

// ===========================================================================
// Test 6: Sequential Streaming Fallback (read_exact)
// ===========================================================================

#[test]
fn test_sequential_streaming_fallback() {
    let mut decoder = CoroutineDecoder::new(|mut ctx| async move {
        let mut header = [0u8; 4];
        ctx.read_exact(&mut header).await?;
        assert_eq!(&header, b"SEQ1");

        let mut data = [0u8; 8];
        ctx.read_exact(&mut data).await?;

        let slice = PixelSlice::new(&data, 2, 1, 8, PixelDescriptor::RGBA8_SRGB).unwrap();
        ctx.yield_scanlines(0, &slice).await?;

        let res: Result<OutputInfo, SourceError> =
            Ok(OutputInfo::full_decode(2, 1, PixelDescriptor::RGBA8_SRGB));
        res
    });

    // Push sequential chunks in two separate calls
    let outcome1 = decoder.push_chunk(b"SEQ1", false, None).unwrap();
    assert_eq!(outcome1.status, PushStatus::NeedRanges); // requests next sequential bytes

    let outcome2 = decoder.push_chunk(&[7u8; 8], true, None).unwrap();
    assert!(outcome2.status == PushStatus::OutputReady || outcome2.status == PushStatus::Complete);

    let batch = decoder.pull_batch().unwrap().expect("batch yielded");
    assert_eq!(batch.0, 0);
    assert_eq!(batch.1.row(0), &[7u8; 8]);
    assert!(decoder.is_complete());
}
