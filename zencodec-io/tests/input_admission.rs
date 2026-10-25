//! Independent byte and source-call oracles for driver admission.
use core::convert::Infallible;
use enough::{Stop, Unstoppable};
use zencodec::decode::{Input, Request};
use zencodec::source::{ReadAt, SliceReadAt, SourceError};
use zencodec_io::{
    ByteStepper, DriveFailure, DriveOptions, InputError, IoLimits, ProtocolViolation, ReadAtCursor,
    Step, Stepped,
};

#[derive(Default)]
struct Source {
    reads: Vec<(u64, usize)>,
    batches: Vec<usize>,
}

fn bytes_at(offset: u64, len: usize) -> Vec<u8> {
    (offset..offset + len as u64)
        .map(|n| (n.wrapping_mul(17) % 251) as u8)
        .collect()
}

impl ReadAt for Source {
    type Error = SourceError;
    fn len(&mut self) -> Result<u64, Self::Error> {
        Ok(4096)
    }
    fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), Self::Error> {
        self.reads.push((offset, out.len()));
        out.copy_from_slice(&bytes_at(offset, out.len()));
        Ok(())
    }
    fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<usize, Self::Error> {
        self.read_exact_at(offset, out)?;
        Ok(out.len())
    }
    fn read_many_at(&mut self, requests: &mut [(u64, &mut [u8])]) -> Result<(), Self::Error> {
        self.batches.push(requests.len());
        for (offset, out) in requests {
            self.read_exact_at(*offset, out)?;
        }
        Ok(())
    }
}

struct Wants(Vec<Request>);
impl ByteStepper for Wants {
    type Error = Infallible;
    type Output = (u64, Vec<u8>);
    fn requested(&self) -> &[Request] {
        &self.0
    }
    fn step(
        &mut self,
        input: Input<'_>,
        _: &dyn Stop,
    ) -> Result<Stepped<Self::Output>, Self::Error> {
        Ok(Stepped {
            consumed: 0,
            step: if self.0.iter().any(|r| r.is_satisfied_by(&input)) {
                Step::Finished((input.offset, input.bytes.to_vec()))
            } else {
                Step::Need
            },
        })
    }
}

fn options(bytes: usize, ranges: usize, concurrent: usize) -> DriveOptions {
    DriveOptions::new(32, 512).with_limits(IoLimits::new(bytes, ranges, concurrent))
}

#[test]
fn oversized_contiguous_requests_and_overflow_do_not_read() {
    for request in [Request::range(32, 17), Request::at_least(32, 17)] {
        let mut source = Source::default();
        let mut cursor = ReadAtCursor::new(options(16, 8, 8));
        let e = cursor
            .next(&mut source, &mut Wants(vec![request]), &Unstoppable)
            .unwrap_err();
        assert!(matches!(
            e,
            DriveFailure::Input(InputError::Limit {
                needed: 17,
                limit: 16
            })
        ));
        assert!(source.reads.is_empty());
        assert_eq!(cursor.retained_input_capacity(), 0);
    }
    for request in [Request::range(u64::MAX, 2), Request::at_least(u64::MAX, 2)] {
        let mut source = Source::default();
        let e = ReadAtCursor::new(DriveOptions::default())
            .next(&mut source, &mut Wants(vec![request]), &Unstoppable)
            .unwrap_err();
        assert!(matches!(
            e,
            DriveFailure::Protocol(ProtocolViolation::AddressOverflow(_))
        ));
        assert!(source.reads.is_empty());
    }
}

#[test]
fn affordable_alternative_and_prefetch_obey_every_cap() {
    for cap in [8, 16, 24, 32, 40, 48, 64, 128] {
        for descriptors in [1, 2, 3, 4] {
            for concurrent in [1, 2, 4] {
                let requests = vec![
                    Request::range(0, 256),
                    Request::range(32, 8),
                    Request::range(48, 8),
                    Request::range(80, 8),
                ];
                let mut source = Source::default();
                let mut cursor = ReadAtCursor::new(options(cap, descriptors, concurrent));
                let (offset, bytes) = cursor
                    .next(&mut source, &mut Wants(requests), &Unstoppable)
                    .unwrap()
                    .into_inner();
                assert_eq!(offset, 32);
                assert_eq!(bytes, bytes_at(32, bytes.len()));
                assert!(bytes.len() >= 8);
                assert!(cursor.retained_input_capacity() <= cap);
                assert!(source.reads.len() <= descriptors);
                assert!(source.reads.iter().map(|(_, n)| n).sum::<usize>() <= cap);
                assert!(source.batches.iter().all(|n| *n <= concurrent));
            }
        }
    }
}

#[test]
fn sequential_prefetch_and_forward_reuse_stay_inside_capacity() {
    let mut source = Source::default();
    let mut cursor = ReadAtCursor::new(options(16, 4, 4));
    for (offset, len) in [(0, 1), (1, 15), (4, 16), (20, 1), (34, 16)] {
        let (_, bytes) = cursor
            .next(
                &mut source,
                &mut Wants(vec![Request::at_least(offset, len)]),
                &Unstoppable,
            )
            .unwrap()
            .into_inner();
        assert_eq!(bytes, bytes_at(offset, bytes.len()));
        assert!(bytes.len() >= len);
        assert!(cursor.retained_input_capacity() <= 16);
    }
    assert!(source.reads.iter().all(|(_, n)| *n <= 16));
    assert_eq!(source.reads[0], (0, 16));
    assert_eq!(
        source.reads[1],
        (16, 4),
        "overlapping prefix must be reused"
    );
}

#[test]
fn retained_ranges_and_new_sequential_window_share_the_allowance() {
    let mut source = Source::default();
    let mut cursor =
        ReadAtCursor::new(DriveOptions::new(0, 16).with_limits(IoLimits::new(24, 3, 3)));
    cursor
        .next(
            &mut source,
            &mut Wants(vec![
                Request::range(0, 8),
                Request::range(32, 8),
                Request::range(64, 8),
            ]),
            &Unstoppable,
        )
        .unwrap();
    assert_eq!(cursor.retained_input_capacity(), 24);
    for request in [
        Request::at_least(33, 16),
        Request::range(96, 24),
        Request::at_least(120, 16),
    ] {
        let (offset, bytes) = cursor
            .next(&mut source, &mut Wants(vec![request]), &Unstoppable)
            .unwrap()
            .into_inner();
        assert_eq!(bytes, bytes_at(offset, bytes.len()));
        assert!(cursor.retained_input_capacity() <= 24);
    }
}

#[test]
fn borrowed_slices_need_no_owned_payload_or_io_allowance() {
    let bytes = bytes_at(0, 128);
    let mut source = SliceReadAt::new(&bytes);
    let mut cursor = ReadAtCursor::new(options(0, 0, 0));
    let (offset, got) = cursor
        .next(
            &mut source,
            &mut Wants(vec![Request::range(32, 64)]),
            &Unstoppable,
        )
        .unwrap()
        .into_inner();
    assert_eq!((offset, got), (32, bytes[32..96].to_vec()));
    assert_eq!(cursor.retained_input_capacity(), 0);
}

#[test]
fn sequential_prefetch_cannot_wrap_the_address_space() {
    let mut source = Source::default();
    let mut cursor = ReadAtCursor::new(options(16, 2, 2));
    let (offset, bytes) = cursor
        .next(
            &mut source,
            &mut Wants(vec![Request::at_least(u64::MAX - 8, 8)]),
            &Unstoppable,
        )
        .unwrap()
        .into_inner();
    assert_eq!((offset, bytes), (u64::MAX - 8, bytes_at(u64::MAX - 8, 8)));
    assert_eq!(source.reads, [(u64::MAX - 8, 8)]);
}

#[test]
fn zero_read_and_descriptor_allowances_refuse_without_io() {
    for (limits, range_error) in [
        (IoLimits::new(16, 0, 1), true),
        (IoLimits::new(16, 1, 0), false),
    ] {
        let mut source = Source::default();
        let error = ReadAtCursor::new(DriveOptions::default().with_limits(limits))
            .next(
                &mut source,
                &mut Wants(vec![Request::range(32, 8)]),
                &Unstoppable,
            )
            .unwrap_err();
        assert!(if range_error {
            matches!(error, DriveFailure::Input(InputError::RangesDisabled))
        } else {
            matches!(error, DriveFailure::Input(InputError::ReadsDisabled))
        });
        assert!(source.reads.is_empty());
    }
}

#[test]
fn failed_sync_batches_never_publish_unfilled_bytes() {
    struct InterruptedBatch {
        source: Source,
        started: usize,
    }
    impl ReadAt for InterruptedBatch {
        type Error = SourceError;
        fn len(&mut self) -> Result<u64, Self::Error> {
            Ok(4096)
        }
        fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), Self::Error> {
            self.source.read_exact_at(offset, out)
        }
        fn read_many_at(&mut self, requests: &mut [(u64, &mut [u8])]) -> Result<(), Self::Error> {
            self.started += 1;
            if self.started == 1 {
                requests[0].1.fill(0xEE);
                return Err(SourceError::UnexpectedEof {
                    offset: requests[0].0,
                    len: requests[0].1.len() as u64,
                    total: None,
                });
            }
            self.source.read_many_at(requests)
        }
    }
    let mut source = InterruptedBatch {
        source: Source::default(),
        started: 0,
    };
    let mut cursor = ReadAtCursor::new(options(16, 2, 2));
    let mut stage = Wants(vec![Request::range(32, 8), Request::range(80, 8)]);
    assert!(matches!(
        cursor.next(&mut source, &mut stage, &Unstoppable),
        Err(DriveFailure::Source(_))
    ));
    assert_eq!(cursor.retained_input_capacity(), 0);
    let (offset, bytes) = cursor
        .next(&mut source, &mut stage, &Unstoppable)
        .unwrap()
        .into_inner();
    assert_eq!(source.started, 2);
    assert_eq!(bytes, bytes_at(offset, bytes.len()));
}

#[cfg(feature = "std")]
#[test]
fn sequential_read_cursor_limits_prefetch_and_skipping_too() {
    let bytes = bytes_at(0, 128);
    let mut source = std::io::Cursor::new(&bytes);
    let mut cursor = zencodec_io::ReadCursor::new(4096, 7);
    for offset in [0, 5, 65, 100] {
        let (offset, got) = cursor
            .next(
                &mut source,
                &mut Wants(vec![Request::at_least(offset, 7)]),
                &Unstoppable,
            )
            .unwrap()
            .into_inner();
        assert_eq!(got, bytes_at(offset, 7));
        assert!(cursor.retained_input_capacity() <= 7);
    }
}

#[cfg(feature = "tokio")]
mod asynchronous {
    use super::*;
    use zencodec::decode::BoxedError;
    use zencodec_io::tokio_io::{AsyncCursor, AsyncSource};

    impl AsyncSource for Source {
        async fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<usize, BoxedError> {
            Ok(ReadAt::read_at(self, offset, out)?)
        }
        async fn read_ranges(
            &mut self,
            requests: &mut [(u64, &mut [u8])],
        ) -> Result<(), BoxedError> {
            Ok(self.read_many_at(requests)?)
        }
        async fn len(&mut self) -> Result<u64, BoxedError> {
            Ok(4096)
        }
    }

    #[tokio::test]
    async fn ready_cursor_outputs_still_yield_after_the_work_quota() {
        use core::{
            future::{Future, poll_fn},
            task::Poll,
        };
        struct Ready;
        impl ByteStepper for Ready {
            type Output = ();
            type Error = Infallible;
            fn requested(&self) -> &[Request] {
                &[]
            }
            fn step(&mut self, _: Input<'_>, _: &dyn Stop) -> Result<Stepped<()>, Infallible> {
                Ok(Stepped {
                    consumed: 0,
                    step: Step::Output(()),
                })
            }
        }
        let mut source = Source::default();
        let mut cursor = AsyncCursor::new(
            options(16, 2, 2).with_work_quota(zencodec_io::WorkQuota::new(3).unwrap()),
        );
        let mut stage = Ready;
        for _ in 0..3 {
            cursor
                .next(&mut source, &mut stage, &Unstoppable)
                .await
                .unwrap();
        }
        let future = cursor.next(&mut source, &mut stage, &Unstoppable);
        let mut future = core::pin::pin!(future);
        poll_fn(|cx| {
            assert!(future.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        future.await.unwrap();
    }

    struct InterruptedBatch {
        source: Source,
        started: usize,
    }
    impl AsyncSource for InterruptedBatch {
        async fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<usize, BoxedError> {
            Ok(ReadAt::read_at(&mut self.source, offset, out)?)
        }
        async fn read_ranges(
            &mut self,
            requests: &mut [(u64, &mut [u8])],
        ) -> Result<(), BoxedError> {
            self.started += 1;
            if self.started == 1 {
                requests[0].1.fill(0xEE);
                core::future::pending::<()>().await;
            }
            Ok(self.source.read_many_at(requests)?)
        }
        async fn len(&mut self) -> Result<u64, BoxedError> {
            Ok(4096)
        }
    }

    #[tokio::test]
    async fn cancellation_never_publishes_partly_filled_ranges() {
        use core::{
            future::{Future, poll_fn},
            task::Poll,
        };
        let mut source = InterruptedBatch {
            source: Source::default(),
            started: 0,
        };
        let mut cursor = AsyncCursor::new(options(16, 2, 2));
        let mut stage = Wants(vec![Request::range(32, 8), Request::range(80, 8)]);
        {
            let future = cursor.next(&mut source, &mut stage, &Unstoppable);
            let mut future = core::pin::pin!(future);
            poll_fn(|cx| {
                assert!(future.as_mut().poll(cx).is_pending());
                Poll::Ready(())
            })
            .await;
        }
        assert_eq!(source.started, 1);
        assert_eq!(cursor.retained_input_capacity(), 0);
        let (offset, bytes) = cursor
            .next(&mut source, &mut stage, &Unstoppable)
            .await
            .unwrap()
            .into_inner();
        assert_eq!(source.started, 2);
        assert_eq!(bytes, bytes_at(offset, bytes.len()));
    }

    #[tokio::test]
    async fn async_and_sync_have_the_same_admission_and_byte_results() {
        for cap in [8, 16, 24, 32, 48, 64] {
            for concurrent in [1, 2, 4] {
                let mut source = Source::default();
                let mut cursor = AsyncCursor::new(options(cap, 3, concurrent));
                for requests in [
                    vec![
                        Request::range(0, 256),
                        Request::range(32, 8),
                        Request::range(48, 8),
                        Request::range(80, 8),
                    ],
                    vec![Request::at_least(33, 8)],
                    vec![Request::range(256, 8)],
                ] {
                    let (offset, bytes) = cursor
                        .next(&mut source, &mut Wants(requests), &Unstoppable)
                        .await
                        .unwrap()
                        .into_inner();
                    assert_eq!(bytes, bytes_at(offset, bytes.len()));
                    assert!(cursor.retained_input_capacity() <= cap);
                    assert!(source.batches.iter().all(|n| *n <= concurrent));
                }
            }
        }
        let mut source = Source::default();
        let error = AsyncCursor::new(options(16, 3, 3))
            .next(
                &mut source,
                &mut Wants(vec![Request::range(32, 17)]),
                &Unstoppable,
            )
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            DriveFailure::Input(InputError::Limit {
                needed: 17,
                limit: 16
            })
        ));
        assert!(source.reads.is_empty());
    }
}
