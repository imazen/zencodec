//! A pull consumer must be able to drain one bounded sink loan per step.
use std::{future::Future, num::NonZeroU32, task::Poll};
use zencodec::decode::*;
use zencodec::{ImageFormat, ImageInfo, NoPulse};
use zencodec_coroutine::{CoroutineDecoder, CoroutineError};
use zenpixels::{PixelFormat, PixelView, PixelViewMut};

const DESC: PixelFormat = PixelFormat::RGB8_SRGB;
const HEIGHT: u32 = 129;

struct OneSlot {
    bytes: [u8; 24],
    pending: bool,
    meta: Option<BatchMeta>,
    rows: u32,
    finished: bool,
}
impl OneSlot {
    fn new() -> Self {
        Self {
            bytes: [0xa5; 24],
            pending: false,
            meta: None,
            rows: 0,
            finished: false,
        }
    }
    fn loan(
        &mut self,
        meta: BatchMeta,
        rows: u32,
        width: u32,
        desc: PixelFormat,
    ) -> Result<PixelViewMut<'_>, SinkError> {
        if self.pending {
            return Err("the consumer has not drained its only slot".into());
        }
        assert!(rows <= 2);
        self.pending = true;
        self.meta = Some(meta);
        self.rows = rows;
        Ok(PixelViewMut::new_with_format_cow(
            &mut self.bytes,
            width,
            rows,
            12,
            zenpixels::FormatCow::from_owned(desc),
        )?)
    }
}
impl DecodeRowSink for OneSlot {
    fn max_buffer_rows(&self) -> NonZeroU32 {
        NonZeroU32::new(2).unwrap()
    }
    fn accepts_provisional(&self) -> bool {
        true
    }
    fn provide_next_buffer(
        &mut self,
        y: u32,
        rows: u32,
        width: u32,
        desc: PixelFormat,
    ) -> Result<PixelViewMut<'_>, SinkError> {
        self.loan(BatchMeta::final_rows(y), rows, width, desc)
    }
    fn provide_pass_buffer(
        &mut self,
        meta: BatchMeta,
        rows: u32,
        width: u32,
        desc: PixelFormat,
    ) -> Result<PixelViewMut<'_>, SinkError> {
        self.loan(meta, rows, width, desc)
    }
    fn finish(&mut self) -> Result<(), SinkError> {
        assert!(!self.pending);
        self.finished = true;
        Ok(())
    }
}

fn decoder(
    provisional: bool,
    returns_with_pending: bool,
) -> impl IncrementalDecode<Error = CoroutineError> {
    CoroutineDecoder::with_options(
        1,
        zencodec::source::AccessPattern::RandomAccess,
        move |mut ctx| async move {
            ctx.set_info(ImageInfo::new(2, HEIGHT, ImageFormat::Png));
            ctx.set_output_descriptor(DESC);
            let mut bytes = vec![0xdd; HEIGHT as usize * 9];
            for y in 0..HEIGHT as usize {
                bytes[y * 9..y * 9 + 6].fill(y as u8);
            }
            let pixels = PixelView::new_with_format_cow(
                &bytes,
                2,
                HEIGHT,
                9,
                zenpixels::FormatCow::from_owned(DESC),
            )
            .unwrap();
            let meta = if provisional {
                BatchMeta::new(0, false).with_pass(7)
            } else {
                BatchMeta::final_rows(0)
            };
            // Exercise the finishing-state path too: a producer can deposit its
            // last rows and return in the same poll, without awaiting delivery.
            if returns_with_pending {
                let mut write = std::pin::pin!(ctx.write_rows(meta, &pixels));
                std::future::poll_fn(|cx| {
                    assert!(write.as_mut().poll(cx).is_pending());
                    Poll::Ready(())
                })
                .await;
            } else {
                ctx.write_rows(meta, &pixels).await?;
            }
            Ok::<_, CoroutineError>(OutputInfo::full_decode(2, HEIGHT, DESC))
        },
    )
}

#[test]
fn large_native_batch_yields_one_strided_slot_per_step_even_while_finishing() {
    for provisional in [false, true] {
        for finishing in [false, true] {
            let mut decoder = decoder(provisional, finishing);
            let mut sink = OneSlot::new();
            let mut next_y = 0;
            while next_y < HEIGHT {
                let progress = decoder
                    .step(
                        Input::new(0, if next_y == 0 { &[] } else { &[1, 2] }),
                        &mut sink,
                        Some(&NoPulse),
                    )
                    .unwrap();
                let Event::Rows { meta, rows } = progress.event else {
                    panic!("{progress:?}")
                };
                assert_eq!(progress.consumed, 0);
                assert_eq!(meta.y, next_y);
                assert_eq!(meta.is_final, !provisional);
                assert_eq!(meta.pass, if provisional { 7 } else { 0 });
                assert_eq!(sink.meta, Some(meta));
                assert_eq!(rows, 2.min(HEIGHT - next_y));
                for row in 0..rows as usize {
                    assert_eq!(
                        &sink.bytes[row * 12..row * 12 + 6],
                        &[(next_y + row as u32) as u8; 6]
                    );
                    assert_eq!(&sink.bytes[row * 12 + 6..row * 12 + 12], &[0xa5; 6]);
                }
                assert!(!sink.finished);
                sink.pending = false; // The pull consumer takes the completed strip.
                next_y += rows;
            }
            assert!(matches!(
                decoder
                    .step(Input::new(0, &[]), &mut sink, Some(&NoPulse))
                    .unwrap()
                    .event,
                Event::Done(_)
            ));
            assert!(sink.finished);
        }
    }
}

#[test]
fn cancellation_between_partial_deliveries_is_terminal() {
    struct Timeout;
    impl enough::Stop for Timeout {
        fn check(&self) -> Result<(), enough::StopReason> {
            Err(enough::StopReason::TimedOut)
        }
    }
    for finishing in [false, true] {
        let mut decoder = decoder(false, finishing);
        let mut sink = OneSlot::new();
        decoder
            .step(Input::new(0, &[]), &mut sink, Some(&NoPulse))
            .unwrap();
        sink.pending = false;
        assert!(matches!(
            decoder.step(
                Input::new(0, &[]),
                &mut sink,
                Some(&zencodec::how_far::StopOnly::new(Timeout))
            ),
            Err(StepError::Cancelled(enough::StopReason::TimedOut))
        ));
        assert!(!sink.pending && !sink.finished);
        assert!(matches!(
            decoder.step(Input::new(0, &[]), &mut sink, Some(&NoPulse)),
            Err(StepError::Finished)
        ));
    }
}

#[test]
fn split_wide_rows_keep_bits_metadata_and_destination_padding() {
    use zenpixels::{ColorOrigin, FloatFacts, HdrMetadata, PixelBuf, SampleFacts, SampleLayout};
    #[repr(align(64))]
    struct Aligned([u8; 64]);
    struct Sink {
        bytes: Aligned,
        metas: Vec<(BatchMeta, RowMetadata)>,
    }
    impl DecodeRowSink for Sink {
        fn max_buffer_rows(&self) -> NonZeroU32 {
            NonZeroU32::MIN
        }
        fn accepts_provisional(&self) -> bool {
            true
        }
        fn provide_next_buffer(
            &mut self,
            _: u32,
            _: u32,
            _: u32,
            _: PixelFormat,
        ) -> Result<PixelViewMut<'_>, SinkError> {
            Err("delivery lost explicit row metadata".into())
        }
        fn provide_buffer_with_metadata(
            &mut self,
            meta: BatchMeta,
            height: u32,
            width: u32,
            desc: PixelFormat,
            facts: &RowMetadata,
        ) -> Result<PixelViewMut<'_>, SinkError> {
            self.bytes.0.fill(0xD6);
            self.metas.push((meta, facts.clone()));
            Ok(PixelViewMut::new_with_format_cow(
                &mut self.bytes.0,
                width,
                height,
                16,
                zenpixels::FormatCow::from_owned(desc),
            )?
            .with_hdr(facts.hdr))
        }
    }
    for is_float in [false, true] {
        let (mut buffer, expected) = if is_float {
            let words = [0x7fc0_0017u32, 0x8000_0000, 0x0000_0001, 0x3f80_0000];
            let spec = zencodec::helpers::codec_spec(SampleLayout::GRAYF32);
            (
                PixelBuf::from_typed(words.map(f32::from_bits).to_vec(), 2, 2, spec).unwrap(),
                words
                    .into_iter()
                    .flat_map(u32::to_ne_bytes)
                    .collect::<Vec<_>>(),
            )
        } else {
            let words = [2114u16, 65535, 4228, 0];
            (
                PixelBuf::from_typed(words.to_vec(), 2, 2, PixelFormat::GRAY16_SRGB).unwrap(),
                words
                    .into_iter()
                    .flat_map(u16::to_ne_bytes)
                    .collect::<Vec<_>>(),
            )
        };
        let facts = if is_float {
            SampleFacts::NONE.with_float(FloatFacts::NO_INFINITE)
        } else {
            SampleFacts::NONE.with_depth(
                zencodec::sample::ExactDepth::new(
                    zenpixels::SampleType::U16,
                    zenpixels::SignalRange::Full,
                    5,
                    16,
                )
                .unwrap(),
            )
        };
        let hdr = HdrMetadata {
            cll: Some(zencodec::ContentLightLevel::new(900, 200)),
            ..HdrMetadata::default()
        };
        let origin = ColorOrigin::from_cicp(zencodec::Cicp::DISPLAY_P3);
        buffer.assume_facts(facts);
        let buffer = buffer.with_hdr(hdr).with_origin(origin.clone());
        let mut decoder = CoroutineDecoder::new(move |mut ctx| async move {
            ctx.set_info(ImageInfo::new(2, 2, ImageFormat::Png));
            ctx.set_output_descriptor(buffer.format().clone());
            ctx.write_rows(BatchMeta::new(0, false).with_pass(2), &buffer.as_view())
                .await?;
            ctx.write_rows(BatchMeta::final_rows(0), &buffer.as_view())
                .await?;
            Ok::<_, CoroutineError>(OutputInfo::full_decode(2, 2, buffer.format().clone()))
        });
        let mut sink = Sink {
            bytes: Aligned([0; 64]),
            metas: Vec::new(),
        };
        let row_bytes = expected.len() / 2;
        for index in 0..4 {
            let progress = decoder
                .step(Input::new(0, &[]), &mut sink, Some(&NoPulse))
                .unwrap();
            let y = index % 2;
            let Event::Rows { meta, rows: 1 } = progress.event else {
                panic!("expected one row")
            };
            assert_eq!(meta.y, y as u32);
            assert_eq!(meta.is_final, index >= 2);
            assert_eq!(meta.pass, if index < 2 { 2 } else { 0 });
            assert_eq!(
                &sink.bytes.0[..row_bytes],
                &expected[y * row_bytes..(y + 1) * row_bytes]
            );
            assert_eq!(&sink.bytes.0[row_bytes..16], &[0xD6; 16][row_bytes..]);
            assert_eq!(sink.metas[index].1.facts, facts);
            assert_eq!(sink.metas[index].1.origin.as_ref(), Some(&origin));
            assert_eq!(sink.metas[index].1.hdr, hdr);
        }
        assert!(matches!(
            decoder
                .step(Input::new(0, &[]), &mut sink, Some(&NoPulse))
                .unwrap()
                .event,
            Event::Done(_)
        ));
    }
}
