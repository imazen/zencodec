use core::num::NonZeroU32;
use zencodec::decode::{BatchMeta, DecodeRowSink, SinkError};
use zenpixels::{PixelDescriptor, PixelSlice, PixelSliceMut};

struct Bounded {
    bytes: Vec<u8>,
    calls: Vec<(BatchMeta, u32)>,
    cap: NonZeroU32,
    fail_at: Option<u32>,
}
impl Bounded {
    fn new(cap: u32) -> Self {
        Self {
            bytes: vec![0xa5; 11 * 12],
            calls: Vec::new(),
            cap: NonZeroU32::new(cap).unwrap(),
            fail_at: None,
        }
    }
}
impl DecodeRowSink for Bounded {
    fn max_buffer_rows(&self) -> NonZeroU32 {
        self.cap
    }
    fn provide_next_buffer(
        &mut self,
        y: u32,
        h: u32,
        w: u32,
        d: PixelDescriptor,
    ) -> Result<PixelSliceMut<'_>, SinkError> {
        self.provide_pass_buffer(BatchMeta::final_rows(y), h, w, d)
    }
    fn accepts_provisional(&self) -> bool {
        true
    }
    fn provide_pass_buffer(
        &mut self,
        meta: BatchMeta,
        h: u32,
        w: u32,
        d: PixelDescriptor,
    ) -> Result<PixelSliceMut<'_>, SinkError> {
        assert!(h <= self.cap.get(), "copy exceeded admitted row capacity");
        if self.fail_at.is_some_and(|y| meta.y >= y) {
            return Err("deliberate sink failure".into());
        }
        self.calls.push((meta, h));
        PixelSliceMut::new(&mut self.bytes[meta.y as usize * 12..], w, h, 12, d).map_err(Into::into)
    }
}

#[test]
fn native_batches_split_without_losing_stride_pass_or_row_coordinates() {
    let mut raw = vec![0xee; 7 * 15 - 6];
    for y in 0..7 {
        for x in 0..9 {
            raw[y * 15 + x] = (y * 9 + x) as u8;
        }
    }
    let pixels = PixelSlice::new(&raw, 3, 7, 15, PixelDescriptor::RGB8_SRGB).unwrap();
    for cap in [1, 2, 3, 7, 9, u32::MAX] {
        for is_final in [false, true] {
            let mut sink = Bounded::new(cap);
            let meta = BatchMeta::new(2, is_final).with_pass(4);
            zencodec::helpers::copy_rows_to_sink(&mut sink, meta, &pixels).unwrap();
            let mut next = 2;
            for (actual, count) in &sink.calls {
                assert_eq!(actual.y, next);
                assert_eq!(actual.is_final, is_final);
                // Final rows deliberately use the ordinary sink entry point.
                if !is_final {
                    assert_eq!(actual.pass, 4);
                }
                next += count;
            }
            assert_eq!(next, 9);
            for y in 0..11 {
                if (2..9).contains(&y) {
                    assert_eq!(&sink.bytes[y * 12..y * 12 + 9], pixels.row((y - 2) as u32));
                    assert_eq!(&sink.bytes[y * 12 + 9..(y + 1) * 12], &[0xa5; 3]);
                } else {
                    assert_eq!(&sink.bytes[y * 12..(y + 1) * 12], &[0xa5; 12]);
                }
            }
        }
    }
}

#[test]
fn row_overflow_fails_before_effects_and_sink_failure_stops_remaining_strips() {
    let raw = [0; 7 * 9];
    let pixels = PixelSlice::new(&raw, 3, 7, 9, PixelDescriptor::RGB8_SRGB).unwrap();
    let mut sink = Bounded::new(2);
    assert!(
        zencodec::helpers::copy_rows_to_sink(
            &mut sink,
            BatchMeta::final_rows(u32::MAX - 3),
            &pixels
        )
        .is_err()
    );
    assert!(sink.calls.is_empty());
    sink.fail_at = Some(4);
    let error = zencodec::helpers::copy_rows_to_sink(&mut sink, BatchMeta::final_rows(0), &pixels)
        .unwrap_err();
    assert_eq!(error.to_string(), "deliberate sink failure");
    assert_eq!(
        sink.calls
            .iter()
            .map(|(m, n)| (m.y, *n))
            .collect::<Vec<_>>(),
        [(0, 2), (2, 2)]
    );
    assert_eq!(&sink.bytes[4 * 12..], &[0xa5; 7 * 12]);
}
