//! Real forward-only sources must not be asked to refetch held bytes.
#![cfg(feature = "std")]

use zencodec::decode::{Input, InputRequest};
use zencodec::how_far::NoPulse;
use zencodec::source::{ReadAt, SourceError};
use zencodec_io::{ByteStepper, DriveOptions, ReadAtCursor, Step, Stepped};

struct Plan {
    rounds: Vec<Vec<InputRequest>>,
    at: usize,
}

impl ByteStepper for Plan {
    type Output = Vec<u8>;
    type Error = core::convert::Infallible;

    fn step(
        &mut self,
        input: Input<'_>,
        _: Option<&dyn zencodec::Pulse>,
    ) -> Result<Stepped<Vec<u8>>, Self::Error> {
        let request = self.rounds[self.at][0];
        if !request.is_satisfied_by(&input) {
            return Ok(Stepped {
                consumed: 0,
                step: Step::Need,
            });
        }
        let (offset, n) = match request {
            InputRequest::AtLeast { offset, min_len } => (offset, min_len),
            InputRequest::Range(r) => (r.offset, r.length as usize),
            _ => unreachable!(),
        };
        assert_eq!(input.offset, offset);
        let bytes = input.bytes[..n].to_vec();
        assert_eq!(
            bytes,
            (offset..offset + n as u64)
                .map(|v| v as u8)
                .collect::<Vec<_>>()
        );
        self.at += 1;
        Ok(Stepped {
            consumed: n,
            step: if self.at == self.rounds.len() {
                Step::Finished(bytes)
            } else {
                Step::Output(bytes)
            },
        })
    }

    fn requested(&self) -> &[InputRequest] {
        &self.rounds[self.at]
    }
}

#[derive(Default)]
struct Forward {
    position: u64,
    reads: Vec<(u64, usize)>,
}

impl ReadAt for Forward {
    type Error = SourceError;
    fn len(&mut self) -> Result<u64, Self::Error> {
        Ok(128)
    }
    fn len_hint(&self) -> Option<u64> {
        Some(128)
    }
    fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), Self::Error> {
        if offset < self.position {
            return Err(SourceError::BeyondWindow {
                offset,
                oldest_retained: self.position,
            });
        }
        if offset + out.len() as u64 > 128 {
            return Err(SourceError::UnexpectedEof {
                offset,
                len: out.len() as u64,
                total: Some(128),
            });
        }
        self.reads.push((offset, out.len()));
        for (i, b) in out.iter_mut().enumerate() {
            *b = (offset + i as u64) as u8;
        }
        self.position = offset + out.len() as u64;
        Ok(())
    }
}

fn opts() -> DriveOptions {
    DriveOptions::new(0, 16)
}

fn overlap_plan() -> Plan {
    Plan {
        at: 0,
        rounds: vec![
            vec![InputRequest::at_least(0, 2)],
            vec![InputRequest::range(4, 4)],
            vec![InputRequest::range(10, 10)],
            vec![InputRequest::at_least(20, 3)],
            vec![InputRequest::range(25, 3)],
        ],
    }
}

fn batch_plan() -> Plan {
    Plan {
        at: 0,
        rounds: vec![
            vec![InputRequest::range(0, 4), InputRequest::range(16, 16)],
            vec![InputRequest::at_least(16, 4)],
            vec![InputRequest::range(24, 4)],
            vec![InputRequest::range(30, 6)],
        ],
    }
}

#[test]
fn read_at_reuses_overlap_and_retains_the_unused_suffix() {
    let mut source = Forward::default();
    let mut cursor = ReadAtCursor::new(opts());
    let mut plan = overlap_plan();
    while !cursor
        .next(&mut source, &mut plan, Some(&NoPulse))
        .unwrap()
        .is_finished()
    {}
    assert_eq!(source.reads, [(0, 16), (16, 4), (20, 18)]);
}

#[test]
fn read_at_reuses_a_batched_range_for_a_sequential_request() {
    let mut source = Forward::default();
    let mut cursor = ReadAtCursor::new(opts());
    let mut plan = batch_plan();
    while !cursor
        .next(&mut source, &mut plan, Some(&NoPulse))
        .unwrap()
        .is_finished()
    {}
    assert_eq!(source.reads, [(0, 4), (16, 16), (32, 4)]);
}

#[test]
fn true_backtracking_still_reaches_the_forward_source_refusal() {
    let mut plan = Plan {
        at: 0,
        rounds: vec![
            vec![InputRequest::at_least(0, 16)],
            vec![InputRequest::range(0, 1)],
        ],
    };
    let mut source = Forward::default();
    let mut cursor = ReadAtCursor::new(opts());
    cursor.next(&mut source, &mut plan, Some(&NoPulse)).unwrap();
    assert!(matches!(
        cursor.next(&mut source, &mut plan, Some(&NoPulse)),
        Err(zencodec_io::DriveFailure::Source(
            SourceError::BeyondWindow { .. }
        ))
    ));
}

#[cfg(feature = "tokio")]
mod asynchronous {
    use super::*;
    use zencodec::decode::BoxedError;
    use zencodec_io::tokio_io::{AsyncCursor, AsyncSource};

    struct Fragmented {
        source: Forward,
        chunk: usize,
    }
    impl AsyncSource for Fragmented {
        async fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<usize, BoxedError> {
            tokio::task::yield_now().await;
            let n = out
                .len()
                .min(self.chunk)
                .min(128usize.saturating_sub(offset as usize));
            self.source.read_exact_at(offset, &mut out[..n])?;
            Ok(n)
        }
        fn len_hint(&self) -> Option<u64> {
            Some(128)
        }
        async fn len(&mut self) -> Result<u64, BoxedError> {
            Ok(128)
        }
    }

    #[tokio::test]
    async fn fragmented_async_sources_reuse_overlaps_and_batched_ranges() {
        for chunk in [1, 3, 7, 16, 64] {
            for mut plan in [overlap_plan(), batch_plan()] {
                let mut source = Fragmented {
                    source: Forward::default(),
                    chunk,
                };
                let mut cursor = AsyncCursor::new(opts());
                while !cursor
                    .next(&mut source, &mut plan, Some(&NoPulse))
                    .await
                    .unwrap()
                    .is_finished()
                {}
                if chunk >= 16 {
                    let expected = if plan.rounds.len() == 5 {
                        vec![(0, 16), (16, 4), (20, chunk.min(18))]
                    } else {
                        vec![(0, 4), (16, 16), (32, 4)]
                    };
                    assert_eq!(source.source.reads, expected);
                }
            }
        }
    }
}
