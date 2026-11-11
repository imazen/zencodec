//! Paired synthetic driver mechanisms, not native codec or semantic parity.
#![forbid(unsafe_code)]

#[path = "../../../../zencodec-io/benches/fixture.rs"]
pub mod older;

use next_codec::decode::{DecodeEvent, DecodePlan, DecodeStep, Decoder, Delivery, Sink};
use next_codec::{FrameHeader, ImageInfo, Input, InputRequest, Layout, Raster, ResourceEstimate};
use std::sync::Arc;

pub const ARMS: [&str; 14] = [
    "direct",
    "direct_AA",
    "older_slice_dyn_stack",
    "older_slice_typed",
    "older_slice_dyn",
    "next_slice_boxed",
    "older_read_at",
    "next_read_at",
    "older_coroutine",
    "older_tokio_inline",
    "next_tokio_inline",
    "older_tokio_pool",
    "next_tokio_pool",
    "next_buffered_one_hop",
];

/// SYN1 wire framing reused from the existing benchmark; samples are noise and patches.
pub fn fixture(width: u32, height: u32) -> Arc<[u8]> {
    let mut bytes = older::synthetic_file(width, height);
    let mut state = 0x1379_ace5u32;
    for (i, pixel) in bytes[older::HEADER..]
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .enumerate()
    {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        let noise = state.to_le_bytes();
        let patch = ((i / width as usize / 8) ^ (i % width as usize / 8)) as u8;
        pixel.copy_from_slice(&[noise[0] ^ patch, noise[1], noise[2], 255]);
    }
    bytes.into()
}

/// One identical backing allocation and row-copy destination for both API families.
#[derive(Default)]
pub struct BufferSink {
    pub bytes: Vec<u8>,
    width: u32,
    height: u32,
    stride: usize,
    padding: usize,
    pub rows: u32,
    pub ended: bool,
}
impl BufferSink {
    pub fn new(padding: usize) -> Self {
        Self {
            padding,
            ..Self::default()
        }
    }
    fn start(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        self.stride = width as usize * 4 + self.padding;
        self.bytes.resize(self.stride * height as usize, 0x95);
    }
    pub fn active(&self) -> Vec<u8> {
        (0..self.height as usize)
            .flat_map(|y| {
                self.bytes[y * self.stride..y * self.stride + self.width as usize * 4]
                    .iter()
                    .copied()
            })
            .collect()
    }
}
impl zencodec::decode::DecodeRowSink for BufferSink {
    fn begin(
        &mut self,
        width: u32,
        height: u32,
        descriptor: zenpixels::PixelDescriptor,
    ) -> Result<(), zencodec::decode::SinkError> {
        assert_eq!(descriptor, zenpixels::PixelDescriptor::RGBA8_SRGB);
        self.start(width, height);
        Ok(())
    }
    fn provide_next_buffer(
        &mut self,
        y: u32,
        rows: u32,
        width: u32,
        descriptor: zenpixels::PixelDescriptor,
    ) -> Result<zenpixels::PixelSliceMut<'_>, zencodec::decode::SinkError> {
        assert_eq!(width, self.width);
        self.rows += rows;
        let from = y as usize * self.stride;
        let end = from + (rows as usize - 1) * self.stride + width as usize * 4;
        Ok(zenpixels::PixelSliceMut::new(
            &mut self.bytes[from..end],
            width,
            rows,
            self.stride,
            descriptor,
        )
        .unwrap())
    }
    fn finish(&mut self) -> Result<(), zencodec::decode::SinkError> {
        self.ended = true;
        Ok(())
    }
}
impl Sink for BufferSink {
    fn begin(&mut self, frame: &FrameHeader) -> next_codec::Result<()> {
        assert_eq!(
            frame.raster.descriptor(),
            Some(next_pixels::PixelDescriptor::RGBA8_SRGB)
        );
        self.start(frame.raster.width, frame.raster.height);
        Ok(())
    }
    fn lend(
        &mut self,
        y: u32,
        rows: u32,
        _pass: next_codec::Pass,
    ) -> next_codec::Result<next_pixels::PixelSliceMut<'_>> {
        self.rows += rows;
        let from = y as usize * self.stride;
        let end = from + (rows as usize - 1) * self.stride + self.width as usize * 4;
        Ok(next_pixels::PixelSliceMut::new(
            &mut self.bytes[from..end],
            self.width,
            rows,
            self.stride,
            next_pixels::PixelDescriptor::RGBA8_SRGB,
        )
        .unwrap())
    }
    fn end(&mut self) -> next_codec::Result<()> {
        self.ended = true;
        Ok(())
    }
}

/// The existing hand-written SYN1 decoder translated to the new protocol.
/// It performs the same row copies; the new protocol adds its frame-done event.
struct NextRows {
    strip: u32,
    next: u32,
    begun: bool,
    ended: bool,
    info: Option<ImageInfo>,
    plan: Option<DecodePlan>,
    need: [InputRequest; 1],
}
impl NextRows {
    fn new(strip: u32) -> Self {
        assert!(strip > 0);
        Self {
            strip,
            next: 0,
            begun: false,
            ended: false,
            info: None,
            plan: None,
            need: [InputRequest::AtLeast {
                offset: 0,
                min_len: older::HEADER,
            }],
        }
    }
}
static FORMAT: next_codec::FormatInfo =
    next_codec::FormatInfo::new("syn1", "image/x-syn1", "syn1", 4, |b| {
        b.starts_with(b"SYN1")
    });
impl Decoder for NextRows {
    fn requested(&self) -> &[InputRequest] {
        if self
            .info
            .as_ref()
            .is_some_and(|info| self.next >= info.height)
        {
            &[]
        } else {
            &self.need
        }
    }
    fn info(&self) -> Option<&ImageInfo> {
        self.info.as_ref()
    }
    fn plan(&self) -> Option<&DecodePlan> {
        self.plan.as_ref()
    }
    fn step(&mut self, input: Input<'_>, sink: &mut dyn Sink) -> next_codec::Result<DecodeStep> {
        let Some(info) = &self.info else {
            assert!(input.offset == 0 && input.bytes.len() >= older::HEADER);
            let width = u32::from_le_bytes(input.bytes[4..8].try_into().unwrap());
            let height = u32::from_le_bytes(input.bytes[8..12].try_into().unwrap());
            let raster = Raster::packed(width, height, next_pixels::PixelDescriptor::RGBA8_SRGB);
            self.info = Some(ImageInfo::new(
                next_codec::ImageFormat::new(&FORMAT),
                width,
                height,
                Layout::Packed(next_pixels::PixelDescriptor::RGBA8_SRGB),
            ));
            self.info.as_mut().unwrap().settled = next_codec::MetadataKinds::ALL;
            self.plan = Some(DecodePlan::new(
                raster,
                Delivery::Strips,
                ResourceEstimate::default(),
            ));
            self.need = [InputRequest::AtLeast {
                offset: older::HEADER as u64,
                min_len: self.strip.min(height) as usize * width as usize * 4,
            }];
            return Ok(DecodeStep::new(older::HEADER, DecodeEvent::Info));
        };
        let (width, height) = (info.width, info.height);
        if !self.begun {
            sink.begin(&FrameHeader::still(
                self.plan.as_ref().unwrap().raster.clone(),
            ))?;
            self.begun = true;
        }
        if self.next >= height {
            if !self.ended {
                sink.end()?;
                self.ended = true;
                return Ok(DecodeStep::new(0, DecodeEvent::FrameDone));
            }
            return Ok(DecodeStep::new(0, DecodeEvent::Done));
        }
        let row = width as usize * 4;
        let rows = (height - self.next).min(self.strip);
        let len = rows as usize * row;
        let at = older::HEADER as u64 + self.next as u64 * row as u64;
        if input.offset == at && input.bytes.len() >= len {
            let mut dst = sink.lend(self.next, rows, next_codec::Pass::Final)?;
            for r in 0..rows {
                let from = r as usize * row;
                dst.row_mut(r)
                    .copy_from_slice(&input.bytes[from..from + row]);
            }
            self.next += rows;
            self.need = [InputRequest::AtLeast {
                offset: older::HEADER as u64 + self.next as u64 * row as u64,
                min_len: self.strip.min(height - self.next) as usize * row,
            }];
            return Ok(DecodeStep::new(len, DecodeEvent::Rows));
        }
        self.need = [InputRequest::AtLeast {
            offset: at,
            min_len: len,
        }];
        Ok(DecodeStep::new(0, DecodeEvent::NeedInput))
    }
}

fn next_decoder(strip: u32) -> Box<dyn Decoder> {
    std::hint::black_box(Box::new(NextRows::new(strip)) as Box<dyn Decoder>)
}

pub fn execute(
    arm: &str,
    file: &Arc<[u8]>,
    strip: u32,
    padding: usize,
    quota: usize,
    runtime: &tokio::runtime::Runtime,
) -> BufferSink {
    let mut sink = BufferSink::new(padding);
    let old_opts = zencodec_io::DriveOptions::default()
        .with_work_quota(zencodec_io::WorkQuota::new(quota).unwrap());
    let new_opts = next_io::DriveOptions::default().with_steps_per_batch(quota);
    match arm {
        "direct" | "direct_AA" => older::direct_decode(file, strip, &mut sink),
        "older_slice_typed" => {
            zencodec_io::drive_from_slice(
                &mut older::HandWritten::new(strip),
                file,
                &mut sink,
                None,
                file.len(),
            )
            .unwrap();
        }
        "older_slice_dyn_stack" => {
            let mut decoder = older::HandWritten::new(strip);
            let erased: &mut (
                     dyn zencodec::decode::IncrementalDecode<Error = older::SynError> + Send
                 ) = std::hint::black_box(&mut decoder);
            zencodec_io::drive_from_slice(erased, file, &mut sink, None, file.len()).unwrap();
        }
        "older_slice_dyn" => {
            let mut decoder: Box<
                dyn zencodec::decode::IncrementalDecode<Error = older::SynError> + Send,
            > = Box::new(older::HandWritten::new(strip));
            zencodec_io::drive_from_slice(
                std::hint::black_box(&mut *decoder),
                file,
                &mut sink,
                None,
                file.len(),
            )
            .unwrap();
        }
        "next_slice_boxed" => {
            next_io::decode_slice(next_decoder(strip), file, &mut sink).unwrap();
        }
        "older_read_at" => {
            let mut source = zencodec::source::SliceReadAt::new(file);
            zencodec_io::drive_from_read_at(
                &mut older::HandWritten::new(strip),
                &mut source,
                &mut sink,
                None,
                old_opts,
            )
            .unwrap();
        }
        "next_read_at" => {
            let mut source: &[u8] = file;
            next_io::decode_read_at(next_decoder(strip), &mut source, &mut sink, &new_opts)
                .unwrap();
        }
        "older_coroutine" => {
            zencodec_io::drive_from_slice(
                &mut older::coroutine_sequential(strip),
                file,
                &mut sink,
                None,
                file.len(),
            )
            .unwrap();
        }
        "older_tokio_inline" => {
            sink = runtime.block_on(async {
                let mut source = zencodec_io::tokio_io::BytesSource(file.clone());
                zencodec_io::tokio_io::decode_async(
                    older::HandWritten::new(strip),
                    sink,
                    &mut source,
                    zencodec_io::tokio_io::Inline,
                    old_opts,
                    None,
                )
                .await
                .unwrap()
                .1
            });
        }
        "next_tokio_inline" => {
            sink = runtime.block_on(async {
                let mut source = next_io::tokio_io::InMemory(file.clone());
                next_io::tokio_io::decode(
                    next_decoder(strip),
                    &mut source,
                    sink,
                    next_io::tokio_io::Offload::Inline,
                    new_opts,
                    next_codec::SharedPulse::default(),
                )
                .await
                .unwrap()
                .1
            });
        }
        "older_tokio_pool" => {
            sink = runtime.block_on(async {
                let mut source = zencodec_io::tokio_io::BytesSource(file.clone());
                zencodec_io::tokio_io::decode_async(
                    older::HandWritten::new(strip),
                    sink,
                    &mut source,
                    |job: zencodec_io::tokio_io::Job| {
                        tokio::task::spawn_blocking(job);
                    },
                    old_opts,
                    None,
                )
                .await
                .unwrap()
                .1
            });
        }
        "next_tokio_pool" => {
            sink = runtime.block_on(async {
                let mut source = next_io::tokio_io::InMemory(file.clone());
                next_io::tokio_io::decode(
                    next_decoder(strip),
                    &mut source,
                    sink,
                    next_io::tokio_io::Offload::Blocking,
                    new_opts,
                    next_codec::SharedPulse::default(),
                )
                .await
                .unwrap()
                .1
            });
        }
        "next_buffered_one_hop" => {
            sink = runtime
                .block_on(next_io::tokio_io::decode_buffered(
                    next_decoder(strip),
                    file.clone(),
                    sink,
                ))
                .unwrap()
                .1;
        }
        _ => panic!("unknown benchmark arm {arm}"),
    }
    sink
}

/// Validate the exact operation used for timing, outside the timed region.
pub fn qualify(runtime: &tokio::runtime::Runtime) {
    for (w, h, strip) in [
        (31, 19, 4),
        (256, 256, 4),
        (1024, 1024, 16),
        (1024, 1024, 1),
    ] {
        let file = fixture(w, h);
        for padding in [0, 16] {
            for arm in ARMS {
                let sink = execute(arm, &file, strip, padding, 64, runtime);
                assert_eq!(
                    sink.active(),
                    file[older::HEADER..],
                    "{arm} {w}x{h} padding {padding}"
                );
                assert_eq!(sink.rows, h, "{arm}");
                assert!(sink.ended, "{arm}");
                if padding > 0 {
                    for row in sink.bytes.chunks_exact(sink.stride) {
                        assert!(
                            row[w as usize * 4..].iter().all(|b| *b == 0x95),
                            "{arm} padding changed"
                        );
                    }
                }
            }
        }
    }
    let file = fixture(256, 256);
    for quota in [1usize, 64, usize::MAX] {
        for arm in [
            "older_tokio_inline",
            "next_tokio_inline",
            "older_tokio_pool",
            "next_tokio_pool",
            "next_buffered_one_hop",
        ] {
            let sink = execute(arm, &file, 1, 16, quota, runtime);
            assert_eq!(sink.active(), file[older::HEADER..], "{arm} quota={quota}");
            assert_eq!(sink.rows, 256);
            assert!(sink.ended);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn translated_decoder_keeps_requests_current_after_consuming_input() {
        use next_codec::decode::Decoder;
        let file = super::fixture(31, 19);
        let mut decoder = super::NextRows::new(4);
        let mut sink = super::BufferSink::new(16);
        let step = decoder
            .step(next_codec::Input::whole(&file), &mut sink)
            .unwrap();
        assert_eq!(step.event, next_codec::decode::DecodeEvent::Info);
        let mut offset = super::older::HEADER;
        for rows in [4usize, 4, 4, 4, 3] {
            assert_eq!(
                decoder.requested(),
                [next_codec::InputRequest::AtLeast {
                    offset: offset as u64,
                    min_len: rows * 31 * 4
                }]
            );
            let step = decoder
                .step(
                    next_codec::Input::new(offset as u64, &file[offset..]),
                    &mut sink,
                )
                .unwrap();
            assert_eq!(step.consumed, rows * 31 * 4);
            offset += step.consumed;
        }
        assert!(decoder.requested().is_empty());
        assert_eq!(
            decoder
                .step(next_codec::Input::whole(&[]), &mut sink)
                .unwrap()
                .event,
            next_codec::decode::DecodeEvent::FrameDone
        );
        assert_eq!(
            decoder
                .step(next_codec::Input::whole(&[]), &mut sink)
                .unwrap()
                .event,
            next_codec::decode::DecodeEvent::Done
        );
    }

    #[test]
    fn every_timed_arm_keeps_exact_strided_samples_and_completion() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        super::qualify(&rt);
    }
}
