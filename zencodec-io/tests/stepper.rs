//! The cursors drive any `ByteStepper`, not only image decoders: a toy
//! record format (a length byte, then that many bytes) yields the same
//! records through the slice, `ReadAt` and `Read` cursors at every window.

use std::convert::Infallible;

use zencodec::decode::{Input, InputRequest};
use zencodec::how_far::NoPulse;
use zencodec::source::{ReadAt, SourceError};
use zencodec_io::{
    ByteStepper, DriveOptions, ReadAtCursor, ReadCursor, SliceCursor, Step, Stepped, Yield,
};

/// Yields each record's payload; `Finished(None)` at the end.
struct Records {
    at: u64,
    request: [InputRequest; 1],
}

impl Records {
    fn new() -> Self {
        Self {
            at: 0,
            request: [InputRequest::at_least(0, 1)],
        }
    }
}

impl ByteStepper for Records {
    type Output = Option<Vec<u8>>;
    type Error = Infallible;

    fn step(
        &mut self,
        input: Input<'_>,
        _: &dyn zencodec::Pulse,
    ) -> Result<Stepped<Self::Output>, Infallible> {
        if input.total_len == Some(self.at) {
            return Ok(Stepped {
                consumed: 0,
                step: Step::Finished(None),
            });
        }
        // Use what lies at our position, if the input reaches it.
        let skip = self.at.checked_sub(input.offset).map(|s| s as usize);
        let here = skip.and_then(|s| input.bytes.get(s..)).unwrap_or(&[]);
        match here.first() {
            Some(&len) if here.len() > len as usize => {
                let payload = here[1..=len as usize].to_vec();
                self.at += 1 + len as u64;
                Ok(Stepped {
                    consumed: skip.unwrap() + 1 + len as usize,
                    step: Step::Output(Some(payload)),
                })
            }
            first => {
                let need = first.map_or(1, |&len| 1 + len as usize);
                self.request = [InputRequest::at_least(self.at, need)];
                Ok(Stepped {
                    consumed: skip.unwrap_or(0).min(input.bytes.len()),
                    step: Step::Need,
                })
            }
        }
    }

    fn requested(&self) -> &[InputRequest] {
        &self.request
    }
}

fn file() -> (Vec<u8>, Vec<Vec<u8>>) {
    let records: Vec<Vec<u8>> = (0..40u8)
        .map(|i| (0..i % 23).map(|j| i ^ j).collect())
        .collect();
    let mut bytes = Vec::new();
    for r in &records {
        bytes.push(r.len() as u8);
        bytes.extend_from_slice(r);
    }
    (bytes, records)
}

/// A source that is not in memory, so the windowed path runs.
struct Remote(Vec<u8>);

impl ReadAt for Remote {
    type Error = SourceError;
    fn len(&mut self) -> Result<u64, SourceError> {
        Ok(self.0.len() as u64)
    }
    fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), SourceError> {
        let n = self.read_at(offset, out)?;
        assert_eq!(n, out.len(), "exact read past the end");
        Ok(())
    }
    fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<usize, SourceError> {
        let start = (offset as usize).min(self.0.len());
        let n = out.len().min(self.0.len() - start);
        out[..n].copy_from_slice(&self.0[start..start + n]);
        Ok(n)
    }
}

fn collect(mut next: impl FnMut(&mut Records) -> Yield<Option<Vec<u8>>>) -> Vec<Vec<u8>> {
    let mut stage = Records::new();
    let mut out = Vec::new();
    loop {
        match next(&mut stage) {
            Yield::Output(Some(r)) => out.push(r),
            Yield::Finished(None) => return out,
            other => panic!("unexpected {other:?}"),
        }
    }
}

#[test]
fn every_cursor_and_window_yields_the_same_records() {
    let (bytes, records) = file();
    for window in [1, 3, 7, 64, 4096] {
        let mut slice = SliceCursor::new(window);
        let got = collect(|s| slice.next(&bytes, s, &NoPulse).unwrap());
        assert_eq!(got, records, "slice window {window}");

        let mut remote = Remote(bytes.clone());
        let mut at = ReadAtCursor::new(DriveOptions::new(0, window));
        let got = collect(|s| at.next(&mut remote, s, &NoPulse).unwrap());
        assert_eq!(got, records, "read_at window {window}");
        assert!(at.stats().reads > 0);

        let mut reader = &bytes[..];
        let mut read = ReadCursor::new(window, 1 << 20);
        let got = collect(|s| read.next(&mut reader, s, &NoPulse).unwrap());
        assert_eq!(got, records, "read window {window}");
    }
}

#[test]
fn an_in_memory_source_is_driven_as_a_slice() {
    let (bytes, records) = file();
    let mut src = zencodec::source::SliceReadAt::new(&bytes);
    let mut at = ReadAtCursor::new(DriveOptions::new(0, 5));
    let got = collect(|s| at.next(&mut src, s, &NoPulse).unwrap());
    assert_eq!(got, records);
}

#[cfg(feature = "tokio")]
mod async_cursor {
    use super::*;
    use zencodec::encode::BoxedError;
    use zencodec_io::tokio_io::{AsyncCursor, AsyncSource};

    #[tokio::test]
    async fn a_controlled_pause_allows_destination_preparation_without_more_io() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        use zencodec::decode::*;
        use zencodec::source::AccessPattern;
        use zencodec_coroutine::{CoroutineDecoder, CoroutineError};
        use zencodec_io::{CollectSink, ControlledEvent, ControlledStepper};
        use zenpixels::{PixelDescriptor, PixelSlice, PixelSliceMut};

        struct Counted {
            source: Trickle,
            reads: Arc<AtomicUsize>,
        }
        impl AsyncSource for Counted {
            async fn read_at(&mut self, at: u64, out: &mut [u8]) -> Result<usize, BoxedError> {
                self.reads.fetch_add(1, Ordering::SeqCst);
                self.source.read_at(at, out).await
            }
            async fn len(&mut self) -> Result<u64, BoxedError> {
                self.source.len().await
            }
        }
        // Preparation happens only after the final boundary becomes available.
        struct Destination(Option<CollectSink>);
        impl DecodeRowSink for Destination {
            fn begin(&mut self, w: u32, h: u32, d: PixelDescriptor) -> Result<(), SinkError> {
                self.0
                    .as_mut()
                    .expect("destination is not prepared")
                    .begin(w, h, d)
            }
            fn provide_next_buffer(
                &mut self,
                y: u32,
                h: u32,
                w: u32,
                d: PixelDescriptor,
            ) -> Result<PixelSliceMut<'_>, SinkError> {
                self.0.as_mut().unwrap().provide_next_buffer(y, h, w, d)
            }
            fn finish(&mut self) -> Result<(), SinkError> {
                self.0.as_mut().unwrap().finish()
            }
        }
        let d = PixelDescriptor::RGBA8_SRGB;
        let native = CoroutineDecoder::controlled(
            32,
            AccessPattern::RandomAccess,
            move |mut ctx| async move {
                let mut header = [0];
                ctx.read_exact_at(0, &mut header).await?;
                ctx.set_info(zencodec::ImageInfo::new(2, 2, zencodec::ImageFormat::Qoi));
                ctx.set_output_descriptor(d);
                ctx.interpretation(InterpretationFacts::new(
                    2,
                    2,
                    Some(d),
                    InterpretationState::Final,
                ))
                .await?;
                let mut rows = [0; 24];
                ctx.read_exact_at(1, &mut rows).await?;
                ctx.write_rows(
                    BatchMeta::final_rows(0),
                    &PixelSlice::new(&rows, 2, 2, 12, d).unwrap(),
                )
                .await?;
                Ok::<_, CoroutineError>(OutputInfo::full_decode(2, 2, d))
            },
        );
        let bytes: Vec<u8> = (0..25).collect();
        let reads = Arc::new(AtomicUsize::new(0));
        let mut source = Counted {
            source: Trickle(bytes.clone()),
            reads: reads.clone(),
        };
        let mut cursor = AsyncCursor::new(DriveOptions::new(0, 2));
        let mut sink = Destination(None);
        let mut stage = ControlledStepper::new(native, &mut sink);
        let Yield::Output(ControlledEvent::Suspended(boundary)) = cursor
            .next(&mut source, &mut stage, &NoPulse)
            .await
            .unwrap()
        else {
            panic!("expected pause")
        };
        let before = reads.load(Ordering::SeqCst);
        assert!(stage.sink().0.is_none());
        tokio::task::yield_now().await;
        assert!(
            matches!(cursor.next(&mut source, &mut stage, &NoPulse).await.unwrap(),
            Yield::Output(ControlledEvent::Suspended(again)) if again == boundary)
        );
        assert_eq!(reads.load(Ordering::SeqCst), before);
        stage.sink_mut().0 = Some(CollectSink::new());
        stage
            .resume(boundary.id(), BoundaryAction::AdoptInterpretation, &NoPulse)
            .unwrap();
        assert_eq!(reads.load(Ordering::SeqCst), before);
        loop {
            match cursor
                .next(&mut source, &mut stage, &NoPulse)
                .await
                .unwrap()
            {
                Yield::Finished(ControlledEvent::Decode(Event::Done(_))) => break,
                Yield::Output(ControlledEvent::Decode(_)) => {}
                other => panic!("unexpected {other:?}"),
            }
        }
        assert_eq!(
            stage.sink().0.as_ref().unwrap().bytes(),
            [&bytes[1..9], &bytes[13..21]].concat()
        );
    }

    /// Returns at most three bytes per read, like a slow socket.
    struct Trickle(Vec<u8>);

    impl AsyncSource for Trickle {
        async fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<usize, BoxedError> {
            let start = (offset as usize).min(self.0.len());
            let n = out.len().min(3).min(self.0.len() - start);
            out[..n].copy_from_slice(&self.0[start..start + n]);
            tokio::task::yield_now().await;
            Ok(n)
        }
        async fn len(&mut self) -> Result<u64, BoxedError> {
            Ok(self.0.len() as u64)
        }
    }

    #[tokio::test]
    async fn the_async_cursor_yields_the_same_records_from_short_reads() {
        let (bytes, records) = file();
        for window in [1, 7, 4096] {
            let mut src = Trickle(bytes.clone());
            let mut cursor = AsyncCursor::new(DriveOptions::new(0, window));
            let mut stage = Records::new();
            let mut got = Vec::new();
            loop {
                match cursor.next(&mut src, &mut stage, &NoPulse).await.unwrap() {
                    Yield::Output(Some(r)) => got.push(r),
                    Yield::Finished(None) => break,
                    other => panic!("unexpected {other:?}"),
                }
            }
            assert_eq!(got, records, "window {window}");
        }
    }
}
