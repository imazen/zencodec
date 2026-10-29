//! A pull consumer must be able to drain one bounded sink loan per step.
use std::{future::Future, num::NonZeroU32, task::Poll};
use zencodec::decode::*;
use zencodec::{ImageFormat, ImageInfo, NoPulse};
use zencodec_coroutine::{CoroutineDecoder, CoroutineError};
use zenpixels::{PixelDescriptor, PixelSlice, PixelSliceMut};

const DESC: PixelDescriptor = PixelDescriptor::RGB8_SRGB;
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
        desc: PixelDescriptor,
    ) -> Result<PixelSliceMut<'_>, SinkError> {
        if self.pending {
            return Err("the consumer has not drained its only slot".into());
        }
        assert!(rows <= 2);
        self.pending = true;
        self.meta = Some(meta);
        self.rows = rows;
        Ok(PixelSliceMut::new(&mut self.bytes, width, rows, 12, desc)?)
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
        desc: PixelDescriptor,
    ) -> Result<PixelSliceMut<'_>, SinkError> {
        self.loan(BatchMeta::final_rows(y), rows, width, desc)
    }
    fn provide_pass_buffer(
        &mut self,
        meta: BatchMeta,
        rows: u32,
        width: u32,
        desc: PixelDescriptor,
    ) -> Result<PixelSliceMut<'_>, SinkError> {
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
            let pixels = PixelSlice::new(&bytes, 2, HEIGHT, 9, DESC).unwrap();
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
