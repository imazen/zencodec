//! Protocol enforcement tests for `zencodec-io` cursors:
//! - Over-consumption (`consumed > input.bytes.len()`) is caught on every `Step` outcome
//!   (`Output`, `Finished`, `Need`) on both empty first offers and non-empty second windows
//!   across `SliceCursor`, `ReadAtCursor`, `ReadCursor`, and `AsyncCursor`.
//! - Exact `InputRequest::Range` rejects short EOF slices at the driver level.
//! - `bind_reposition` on started sync/async cursors preserves usable unread prefetched
//!   bytes on forward-only routes and refuses before mutating state on unsupported backtracks.

use std::convert::Infallible;

use zencodec::decode::{Input, InputRequest};
use zencodec::how_far::NoPulse;
use zencodec::source::{AccessPattern, ReadAt, SourceAccess, SourceError};
use zencodec_io::{
    ByteStepper, DriveFailure, DriveOptions, ProtocolViolation, ReadAtCursor, SliceCursor, Step,
    Stepped, Yield,
};
#[cfg(feature = "std")]
use zencodec_io::{PumpFailure, ReadCursor};

#[derive(Clone, Copy)]
enum Mode {
    Output,
    Finished,
    Need,
}

struct LyingStepper {
    mode: Mode,
    wait_for_nonempty: bool,
    req: [InputRequest; 1],
}

impl LyingStepper {
    fn new(mode: Mode, wait_for_nonempty: bool) -> Self {
        Self {
            mode,
            wait_for_nonempty,
            req: [InputRequest::at_least(0, 4)],
        }
    }
}

impl ByteStepper for LyingStepper {
    type Output = u8;
    type Error = Infallible;

    fn step(
        &mut self,
        input: Input<'_>,
        _: Option<&dyn zencodec::Pulse>,
    ) -> Result<Stepped<u8>, Infallible> {
        if self.wait_for_nonempty && input.bytes.is_empty() {
            self.req = [InputRequest::at_least(input.offset, 4)];
            return Ok(Stepped {
                consumed: 0,
                step: Step::Need,
            });
        }
        let over = input.bytes.len().saturating_add(1);
        let step = match self.mode {
            Mode::Output => Step::Output(1),
            Mode::Finished => Step::Finished(2),
            Mode::Need => {
                self.req = [InputRequest::at_least(input.offset + 1, 1)];
                Step::Need
            }
        };
        Ok(Stepped {
            consumed: over,
            step,
        })
    }

    fn requested(&self) -> &[InputRequest] {
        &self.req
    }
}

struct Remote(Vec<u8>);
impl ReadAt for Remote {
    type Error = SourceError;
    fn len(&mut self) -> Result<u64, SourceError> {
        Ok(self.0.len() as u64)
    }
    fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), SourceError> {
        let n = self.read_at(offset, out)?;
        if n != out.len() {
            return Err(SourceError::UnexpectedEof {
                offset,
                len: out.len() as u64,
                total: Some(self.0.len() as u64),
            });
        }
        Ok(())
    }
    fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<usize, SourceError> {
        let start = (offset as usize).min(self.0.len());
        let n = out.len().min(self.0.len() - start);
        out[..n].copy_from_slice(&self.0[start..start + n]);
        Ok(n)
    }
}

/// Forward-only `ReadAt` that fails if asked to reread behind its furthest read head.
struct ForwardOnlyReadAt {
    data: Vec<u8>,
    pos: u64,
}
impl ReadAt for ForwardOnlyReadAt {
    type Error = SourceError;
    fn len(&mut self) -> Result<u64, SourceError> {
        Err(SourceError::Io {
            message: "forward stream length unknown".into(),
        })
    }
    fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<usize, SourceError> {
        if offset < self.pos {
            return Err(SourceError::BeyondWindow {
                offset,
                oldest_retained: self.pos,
            });
        }
        let start = (offset as usize).min(self.data.len());
        let n = out.len().min(self.data.len() - start);
        out[..n].copy_from_slice(&self.data[start..start + n]);
        self.pos = (start + n) as u64;
        Ok(n)
    }
    fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), SourceError> {
        let n = self.read_at(offset, out)?;
        if n != out.len() {
            return Err(SourceError::UnexpectedEof {
                offset,
                len: out.len() as u64,
                total: Some(self.data.len() as u64),
            });
        }
        Ok(())
    }
}

#[test]
fn over_consumed_rejected_on_every_step_variant_empty_and_nonempty_window() {
    let data = vec![10u8, 20, 30, 40, 50];
    for wait_for_nonempty in [false, true] {
        let expected_offered = if wait_for_nonempty { 4 } else { 0 };
        for mode in [Mode::Output, Mode::Finished, Mode::Need] {
            // SliceCursor
            let mut slice = SliceCursor::new(4);
            let mut stage = LyingStepper::new(mode, wait_for_nonempty);
            let err = slice.next(&data, &mut stage, Some(&NoPulse)).unwrap_err();
            assert!(
                matches!(
                    err,
                    DriveFailure::Protocol(ProtocolViolation::OverConsumed { consumed, offered })
                        if offered == expected_offered && consumed == offered + 1
                ),
                "SliceCursor failed (wait={wait_for_nonempty}): {err:?}"
            );

            // ReadAtCursor (remote so windowed path runs)
            let mut remote = Remote(data.clone());
            let mut at = ReadAtCursor::new(DriveOptions::new(0, 4));
            let mut stage = LyingStepper::new(mode, wait_for_nonempty);
            let err = at
                .next(&mut remote, &mut stage, Some(&NoPulse))
                .unwrap_err();
            assert!(
                matches!(
                    err,
                    DriveFailure::Protocol(ProtocolViolation::OverConsumed { consumed, offered })
                        if offered == expected_offered && consumed == offered + 1
                ),
                "ReadAtCursor failed (wait={wait_for_nonempty}): {err:?}"
            );

            // ReadCursor (requires `std`)
            #[cfg(feature = "std")]
            {
                let mut reader = &data[..];
                let mut read = ReadCursor::new(4, 1024);
                let mut stage = LyingStepper::new(mode, wait_for_nonempty);
                let err = read
                    .next(&mut reader, &mut stage, Some(&NoPulse))
                    .unwrap_err();
                let expected_read_offered = if wait_for_nonempty { data.len() } else { 0 };
                assert!(
                    matches!(
                        err,
                        PumpFailure::Protocol(ProtocolViolation::OverConsumed { consumed, offered })
                            if offered == expected_read_offered && consumed == offered + 1
                    ),
                    "ReadCursor failed (wait={wait_for_nonempty}): {err:?}"
                );
            }
        }
    }
}

#[test]
fn driver_level_short_eof_range_request_is_rejected() {
    struct ExactRangeStage {
        req: [InputRequest; 1],
    }
    impl ByteStepper for ExactRangeStage {
        type Output = usize;
        type Error = Infallible;
        fn step(
            &mut self,
            input: Input<'_>,
            _: Option<&dyn zencodec::Pulse>,
        ) -> Result<Stepped<usize>, Infallible> {
            if input.offset == 2 && input.bytes.len() >= 8 {
                Ok(Stepped {
                    consumed: 8,
                    step: Step::Finished(input.bytes.len()),
                })
            } else {
                Ok(Stepped {
                    consumed: 0,
                    step: Step::Need,
                })
            }
        }
        fn requested(&self) -> &[InputRequest] {
            &self.req
        }
    }

    let short_data = vec![1u8, 2, 3, 4, 5, 6];
    let mut stage = ExactRangeStage {
        req: [InputRequest::range(2, 8)],
    };
    let mut slice = SliceCursor::new(16);
    let err = slice
        .next(&short_data, &mut stage, Some(&NoPulse))
        .unwrap_err();
    assert!(
        matches!(
            err,
            DriveFailure::Protocol(ProtocolViolation::NeedPastEnd { total: 6, .. })
        ),
        "SliceCursor must reject short-EOF Range: {err:?}"
    );

    let mut remote = Remote(short_data);
    let mut at = ReadAtCursor::new(DriveOptions::new(0, 16));
    let err = at
        .next(&mut remote, &mut stage, Some(&NoPulse))
        .unwrap_err();
    assert!(
        matches!(
            err,
            DriveFailure::Source(SourceError::UnexpectedEof {
                offset: 2,
                len: 8,
                total: Some(6)
            })
        ),
        "ReadAtCursor must reject short-EOF Range via SourceError::UnexpectedEof: {err:?}"
    );
}

struct ConsumeAt {
    offset: u64,
    take: usize,
    req: [InputRequest; 1],
}
impl ConsumeAt {
    fn new(offset: u64, take: usize) -> Self {
        Self {
            offset,
            take,
            req: [InputRequest::at_least(offset, take)],
        }
    }
}
impl ByteStepper for ConsumeAt {
    type Output = Vec<u8>;
    type Error = Infallible;
    fn step(
        &mut self,
        input: Input<'_>,
        _: Option<&dyn zencodec::Pulse>,
    ) -> Result<Stepped<Vec<u8>>, Infallible> {
        if input.offset == self.offset && input.bytes.len() >= self.take {
            let out = input.bytes[..self.take].to_vec();
            Ok(Stepped {
                consumed: self.take,
                step: Step::Output(out),
            })
        } else {
            self.req = [InputRequest::at_least(self.offset, self.take)];
            Ok(Stepped {
                consumed: 0,
                step: Step::Need,
            })
        }
    }
    fn requested(&self) -> &[InputRequest] {
        &self.req
    }
}

#[test]
fn started_read_at_cursor_bind_reposition_preserves_unread_forward_bytes_and_refuses_backtrack() {
    let bytes: Vec<u8> = (0..64u8).collect();
    let mut fwd = ForwardOnlyReadAt {
        data: bytes.clone(),
        pos: 0,
    };
    let mut cursor = ReadAtCursor::new(DriveOptions::new(0, 8));

    // Read [0, 8) into window and consume 2 bytes -> [2, 8) stays buffered while source pos == 8.
    let mut s1 = ConsumeAt::new(0, 2);
    let y1 = cursor.next(&mut fwd, &mut s1, Some(&NoPulse)).unwrap();
    assert!(matches!(y1, Yield::Output(ref v) if v == &[0, 1]));
    assert_eq!(fwd.pos, 8);

    // Backward reposition to byte 1 on a forward-only source MUST fail BEFORE mutating [2, 8).
    assert!(
        cursor
            .bind_reposition(
                &SourceAccess::forward_stream(),
                AccessPattern::Sequential,
                InputRequest::at_least(1, 1),
            )
            .is_err()
    );
    // RandomAccess request on forward-only source fails BEFORE mutating [2, 8).
    assert!(
        cursor
            .bind_reposition(
                &SourceAccess::forward_stream(),
                AccessPattern::RandomAccess,
                InputRequest::range(4, 2),
            )
            .is_err()
    );

    // Forward sequential reposition to byte 4 preserves buffered [4, 8) without rereading!
    cursor
        .bind_reposition(
            &SourceAccess::forward_stream(),
            AccessPattern::Sequential,
            InputRequest::at_least(4, 2),
        )
        .unwrap();
    let mut s2 = ConsumeAt::new(4, 4);
    let y2 = cursor.next(&mut fwd, &mut s2, Some(&NoPulse)).unwrap();
    assert!(matches!(y2, Yield::Output(ref v) if v == &[4, 5, 6, 7]));
    assert_eq!(
        fwd.pos, 8,
        "reposition to buffered offset 4 must not reread"
    );

    // Continuation at offset 8 reads fresh forward bytes [8, 16) cleanly.
    let mut s3 = ConsumeAt::new(8, 2);
    let y3 = cursor.next(&mut fwd, &mut s3, Some(&NoPulse)).unwrap();
    assert!(matches!(y3, Yield::Output(ref v) if v == &[8, 9]));
    assert_eq!(fwd.pos, 16);

    // Random-access source allows jumping to arbitrary target offsets.
    let mut remote = Remote(bytes);
    let mut ra = ReadAtCursor::new(DriveOptions::new(0, 8));
    let mut s_init = ConsumeAt::new(0, 2);
    let _ = ra.next(&mut remote, &mut s_init, Some(&NoPulse)).unwrap();
    ra.bind_reposition(
        &SourceAccess::positional(Some(64)),
        AccessPattern::RandomAccess,
        InputRequest::range(41, 1),
    )
    .unwrap();
    let mut s_jump = ConsumeAt::new(41, 1);
    let y4 = ra.next(&mut remote, &mut s_jump, Some(&NoPulse)).unwrap();
    assert!(matches!(y4, Yield::Output(ref v) if v == &[41]));
}

#[cfg(feature = "tokio")]
#[tokio::test]
async fn async_cursor_over_consumed_short_eof_and_started_forward_reposition() {
    use zencodec::encode::BoxedError;
    use zencodec_io::tokio_io::{AsyncCursor, AsyncSource};

    struct AsyncRemote(Vec<u8>);
    impl AsyncSource for AsyncRemote {
        async fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<usize, BoxedError> {
            let start = (offset as usize).min(self.0.len());
            let n = out.len().min(self.0.len() - start);
            out[..n].copy_from_slice(&self.0[start..start + n]);
            Ok(n)
        }
        async fn len(&mut self) -> Result<u64, BoxedError> {
            Ok(self.0.len() as u64)
        }
    }

    struct AsyncForwardOnly {
        data: Vec<u8>,
        pos: u64,
    }
    impl AsyncSource for AsyncForwardOnly {
        async fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<usize, BoxedError> {
            if offset < self.pos {
                return Err(Box::new(SourceError::BeyondWindow {
                    offset,
                    oldest_retained: self.pos,
                }));
            }
            let start = (offset as usize).min(self.data.len());
            let n = out.len().min(self.data.len() - start);
            out[..n].copy_from_slice(&self.data[start..start + n]);
            self.pos = (start + n) as u64;
            Ok(n)
        }
        async fn len(&mut self) -> Result<u64, BoxedError> {
            Err(Box::new(SourceError::Io {
                message: "forward stream length unknown".into(),
            }))
        }
    }

    let data = vec![10u8, 20, 30, 40, 50];
    for wait_for_nonempty in [false, true] {
        let expected_offered = if wait_for_nonempty { 4 } else { 0 };
        for mode in [Mode::Output, Mode::Finished, Mode::Need] {
            let mut src = AsyncRemote(data.clone());
            let mut cursor = AsyncCursor::new(DriveOptions::new(0, 4));
            let mut stage = LyingStepper::new(mode, wait_for_nonempty);
            let err = cursor
                .next(&mut src, &mut stage, Some(&NoPulse))
                .await
                .unwrap_err();
            assert!(
                matches!(
                    err,
                    DriveFailure::Protocol(ProtocolViolation::OverConsumed { consumed, offered })
                        if offered == expected_offered && consumed == offered + 1
                ),
                "AsyncCursor failed (wait={wait_for_nonempty}): {err:?}"
            );
        }
    }

    // Started AsyncCursor forward-only reposition preserves [4, 8) without rereading.
    let bytes: Vec<u8> = (0..64u8).collect();
    let mut fwd = AsyncForwardOnly {
        data: bytes,
        pos: 0,
    };
    let mut cursor = AsyncCursor::new(DriveOptions::new(0, 8));
    let mut s1 = ConsumeAt::new(0, 2);
    let y1 = cursor
        .next(&mut fwd, &mut s1, Some(&NoPulse))
        .await
        .unwrap();
    assert!(matches!(y1, Yield::Output(ref v) if v == &[0, 1]));
    assert_eq!(fwd.pos, 8);

    // Backward reposition fails BEFORE mutating buffered [2, 8).
    assert!(
        cursor
            .bind_reposition(
                &SourceAccess::forward_stream(),
                AccessPattern::Sequential,
                InputRequest::at_least(1, 1),
            )
            .is_err()
    );
    // Forward reposition to offset 4 keeps [4, 8) intact.
    cursor
        .bind_reposition(
            &SourceAccess::forward_stream(),
            AccessPattern::Sequential,
            InputRequest::at_least(4, 2),
        )
        .unwrap();
    let mut s2 = ConsumeAt::new(4, 4);
    let y2 = cursor
        .next(&mut fwd, &mut s2, Some(&NoPulse))
        .await
        .unwrap();
    assert!(matches!(y2, Yield::Output(ref v) if v == &[4, 5, 6, 7]));
    assert_eq!(fwd.pos, 8);

    // Continuation at offset 8 reads fresh forward bytes.
    let mut s3 = ConsumeAt::new(8, 2);
    let y3 = cursor
        .next(&mut fwd, &mut s3, Some(&NoPulse))
        .await
        .unwrap();
    assert!(matches!(y3, Yield::Output(ref v) if v == &[8, 9]));
    assert_eq!(fwd.pos, 16);
}

/// Stage that eagerly accepts any >= 4 byte window at offset 4, to verify that
/// `bind_reposition(..., Range(4, 8))` fulfills the full 8-byte requirement (or fails
/// short EOF) BEFORE invoking `stage.step`.
struct EagerPrefixStage {
    steps: usize,
    seen_len: usize,
}
impl ByteStepper for EagerPrefixStage {
    type Output = Vec<u8>;
    type Error = Infallible;
    fn step(
        &mut self,
        input: Input<'_>,
        _: Option<&dyn zencodec::Pulse>,
    ) -> Result<Stepped<Vec<u8>>, Infallible> {
        self.steps += 1;
        self.seen_len = input.bytes.len();
        Ok(Stepped {
            consumed: input.bytes.len(),
            step: Step::Output(input.bytes.to_vec()),
        })
    }
    fn requested(&self) -> &[InputRequest] {
        &[]
    }
}

#[test]
fn bind_reposition_fulfills_exact_range_before_stepping_sync_and_slice() {
    // 1. Forward-only 12-byte source with [4, 8) retained: bind_reposition(Range(4, 8))
    //    must fetch [8, 12) BEFORE calling step, even though EagerPrefixStage would
    //    have returned early on a 4-byte prefix!
    let data12: Vec<u8> = (0..12u8).collect();
    let mut fwd = ForwardOnlyReadAt {
        data: data12.clone(),
        pos: 0,
    };
    let mut cursor = ReadAtCursor::new(DriveOptions::new(0, 8));
    let mut init = ConsumeAt::new(0, 4);
    let _ = cursor.next(&mut fwd, &mut init, Some(&NoPulse)).unwrap();
    assert_eq!(fwd.pos, 8);

    cursor
        .bind_reposition(
            &SourceAccess::forward_stream(),
            AccessPattern::Sequential,
            InputRequest::range(4, 8),
        )
        .unwrap();
    let mut eager = EagerPrefixStage {
        steps: 0,
        seen_len: 0,
    };
    let out = cursor.next(&mut fwd, &mut eager, Some(&NoPulse)).unwrap();
    assert_eq!(eager.steps, 1);
    assert_eq!(eager.seen_len, 8);
    assert!(matches!(out, Yield::Output(ref v) if v == &data12[4..12]));
    assert_eq!(fwd.pos, 12);

    // 2. Short 10-byte source with [4, 8) retained: bind_reposition(Range(4, 8)) must
    //    fail with UnexpectedEof BEFORE invoking EagerPrefixStage::step!
    let data10: Vec<u8> = (0..10u8).collect();
    let mut short_fwd = ForwardOnlyReadAt {
        data: data10.clone(),
        pos: 0,
    };
    let mut short_cur = ReadAtCursor::new(DriveOptions::new(0, 8));
    let mut init2 = ConsumeAt::new(0, 4);
    let _ = short_cur
        .next(&mut short_fwd, &mut init2, Some(&NoPulse))
        .unwrap();
    short_cur
        .bind_reposition(
            &SourceAccess::forward_stream(),
            AccessPattern::Sequential,
            InputRequest::range(4, 8),
        )
        .unwrap();
    let mut never_step = EagerPrefixStage {
        steps: 0,
        seen_len: 0,
    };
    let err = short_cur
        .next(&mut short_fwd, &mut never_step, Some(&NoPulse))
        .unwrap_err();
    assert_eq!(never_step.steps, 0, "short EOF must fail before stage.step");
    assert!(matches!(
        err,
        DriveFailure::Source(SourceError::UnexpectedEof { .. })
    ));
    // Repeating `next` after the failed seek must STILL refuse before stepping `stage`!
    let err2 = short_cur
        .next(&mut short_fwd, &mut never_step, Some(&NoPulse))
        .unwrap_err();
    assert_eq!(
        never_step.steps, 0,
        "repeated next after failed seek must not expose partial prefix"
    );
    assert!(matches!(
        err2,
        DriveFailure::Source(SourceError::UnexpectedEof { .. } | SourceError::BeyondWindow { .. })
    ));

    // 3. In-memory slice source (10 bytes): bind_reposition(Range(4, 8)) must fail
    //    with NeedPastEnd BEFORE invoking stage.step, including on repeated `next`.
    let mut slice_cur = ReadAtCursor::new(DriveOptions::new(0, 8));
    let mut slice_src = zencodec::source::SliceReadAt::new(data10.as_slice());
    let mut init3 = ConsumeAt::new(0, 4);
    let _ = slice_cur
        .next(&mut slice_src, &mut init3, Some(&NoPulse))
        .unwrap();
    slice_cur
        .bind_reposition(
            &SourceAccess::forward_stream(),
            AccessPattern::Sequential,
            InputRequest::range(4, 8),
        )
        .unwrap();
    let mut never_slice = EagerPrefixStage {
        steps: 0,
        seen_len: 0,
    };
    let err = slice_cur
        .next(&mut slice_src, &mut never_slice, Some(&NoPulse))
        .unwrap_err();
    assert_eq!(never_slice.steps, 0);
    assert!(matches!(
        err,
        DriveFailure::Protocol(ProtocolViolation::NeedPastEnd { total: 10, .. })
    ));
    let err2 = slice_cur
        .next(&mut slice_src, &mut never_slice, Some(&NoPulse))
        .unwrap_err();
    assert_eq!(never_slice.steps, 0);
    assert!(matches!(
        err2,
        DriveFailure::Protocol(ProtocolViolation::NeedPastEnd { total: 10, .. })
    ));
}

#[cfg(feature = "tokio")]
#[tokio::test]
async fn async_bind_reposition_fulfills_exact_range_before_stepping() {
    use zencodec::encode::BoxedError;
    use zencodec_io::tokio_io::{AsyncCursor, AsyncSource};

    struct AsyncFwd {
        data: Vec<u8>,
        pos: u64,
    }
    impl AsyncSource for AsyncFwd {
        async fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<usize, BoxedError> {
            if offset < self.pos {
                return Err(Box::new(SourceError::BeyondWindow {
                    offset,
                    oldest_retained: self.pos,
                }));
            }
            let start = (offset as usize).min(self.data.len());
            let n = out.len().min(self.data.len() - start);
            out[..n].copy_from_slice(&self.data[start..start + n]);
            self.pos = (start + n) as u64;
            Ok(n)
        }
        async fn len(&mut self) -> Result<u64, BoxedError> {
            Err(Box::new(SourceError::Io {
                message: "unknown".into(),
            }))
        }
    }

    let data12: Vec<u8> = (0..12u8).collect();
    let mut fwd = AsyncFwd {
        data: data12.clone(),
        pos: 0,
    };
    let mut cursor = AsyncCursor::new(DriveOptions::new(0, 8));
    let mut init = ConsumeAt::new(0, 4);
    let _ = cursor
        .next(&mut fwd, &mut init, Some(&NoPulse))
        .await
        .unwrap();
    cursor
        .bind_reposition(
            &SourceAccess::forward_stream(),
            AccessPattern::Sequential,
            InputRequest::range(4, 8),
        )
        .unwrap();
    let mut eager = EagerPrefixStage {
        steps: 0,
        seen_len: 0,
    };
    let out = cursor
        .next(&mut fwd, &mut eager, Some(&NoPulse))
        .await
        .unwrap();
    assert_eq!(eager.steps, 1);
    assert_eq!(eager.seen_len, 8);
    assert!(matches!(out, Yield::Output(ref v) if v == &data12[4..12]));

    let mut short_fwd = AsyncFwd {
        data: (0..10u8).collect(),
        pos: 0,
    };
    let mut short_cur = AsyncCursor::new(DriveOptions::new(0, 8));
    let mut init2 = ConsumeAt::new(0, 4);
    let _ = short_cur
        .next(&mut short_fwd, &mut init2, Some(&NoPulse))
        .await
        .unwrap();
    short_cur
        .bind_reposition(
            &SourceAccess::forward_stream(),
            AccessPattern::Sequential,
            InputRequest::range(4, 8),
        )
        .unwrap();
    let mut never = EagerPrefixStage {
        steps: 0,
        seen_len: 0,
    };
    let err = short_cur
        .next(&mut short_fwd, &mut never, Some(&NoPulse))
        .await
        .unwrap_err();
    assert_eq!(
        never.steps, 0,
        "async short EOF must fail before stage.step"
    );
    assert!(matches!(err, DriveFailure::Source(_)));
    let err2 = short_cur
        .next(&mut short_fwd, &mut never, Some(&NoPulse))
        .await
        .unwrap_err();
    assert_eq!(
        never.steps, 0,
        "repeated async next after failed seek must not expose partial prefix"
    );
    assert!(matches!(err2, DriveFailure::Source(_)));
}
