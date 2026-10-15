//! The cursors drive any `ByteStepper`, not only image decoders: a toy
//! record format (a length byte, then that many bytes) yields the same
//! records through the slice, `ReadAt` and `Read` cursors at every window.

use std::convert::Infallible;

use enough::Unstoppable;
use zencodec::decode::{Input, Request};
use zencodec::source::{ReadAt, SourceError};
use zencodec_io::{
    ByteStepper, DriveOptions, ReadAtCursor, ReadCursor, SliceCursor, Step, Stepped, Yield,
};

/// Yields each record's payload; `Finished(None)` at the end.
struct Records {
    at: u64,
    request: [Request; 1],
}

impl Records {
    fn new() -> Self {
        Self {
            at: 0,
            request: [Request::at_least(0, 1)],
        }
    }
}

impl ByteStepper for Records {
    type Output = Option<Vec<u8>>;
    type Error = Infallible;

    fn step(
        &mut self,
        input: Input<'_>,
        _: &dyn enough::Stop,
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
                self.request = [Request::at_least(self.at, need)];
                Ok(Stepped {
                    consumed: skip.unwrap_or(0).min(input.bytes.len()),
                    step: Step::Need,
                })
            }
        }
    }

    fn requested(&self) -> &[Request] {
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
        let got = collect(|s| slice.next(&bytes, s, &Unstoppable).unwrap());
        assert_eq!(got, records, "slice window {window}");

        let mut remote = Remote(bytes.clone());
        let mut at = ReadAtCursor::new(DriveOptions::new(0, window));
        let got = collect(|s| at.next(&mut remote, s, &Unstoppable).unwrap());
        assert_eq!(got, records, "read_at window {window}");
        assert!(at.stats().reads > 0);

        let mut reader = &bytes[..];
        let mut read = ReadCursor::new(window, 1 << 20);
        let got = collect(|s| read.next(&mut reader, s, &Unstoppable).unwrap());
        assert_eq!(got, records, "read window {window}");
    }
}

#[test]
fn an_in_memory_source_is_driven_as_a_slice() {
    let (bytes, records) = file();
    let mut src = zencodec::source::SliceReadAt::new(&bytes);
    let mut at = ReadAtCursor::new(DriveOptions::new(0, 5));
    let got = collect(|s| at.next(&mut src, s, &Unstoppable).unwrap());
    assert_eq!(got, records);
}

#[cfg(feature = "tokio")]
mod async_cursor {
    use super::*;
    use zencodec::encode::BoxedError;
    use zencodec_io::tokio_io::{AsyncCursor, AsyncSource};

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
                match cursor
                    .next(&mut src, &mut stage, &Unstoppable)
                    .await
                    .unwrap()
                {
                    Yield::Output(Some(r)) => got.push(r),
                    Yield::Finished(None) => break,
                    other => panic!("unexpected {other:?}"),
                }
            }
            assert_eq!(got, records, "window {window}");
        }
    }
}
