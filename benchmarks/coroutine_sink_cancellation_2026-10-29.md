# Coroutine sink cancellation at completion

Base: `828a4955b168b1c055d05d55d3584a35e95f65ba`.
Failing-first regression: `9b36fe2c`.

A final sink begin/finish callback could signal cancellation but return success.
The coroutine adapter accepted Done, made pending details available, and (for a
stop during begin) also called finish. The new control fails on that behavior.
The fix checks Pulse between those callbacks and before committing Done. State
is already poisoned, so cancellation releases pending result details and later
calls return Finished even with None. Sink errors remain primary. Earlier sink
effects are irrevocable; this is not native-call preemption.

The eight-case control combines both callback positions, both Cancelled/TimedOut
reasons and both successful/error-returning sink hooks. The signal is one-shot.
It asserts exact finish counts, pending-owner destruction before decoder drop,
unavailable details and terminal behavior. Existing success, panic, geometry,
striding, staged-row and cancellation controls remain unchanged.

ARM64 Rust 1.96.0, run-heavy 3G/jobs2, timeout1200:

- Before fix: the regression fails with successful Done (rc101).
- After fix: coroutine and I/O all-feature library/integration tests and
  doctests: 202 passed, 0 failed, 2 existing ignored doctests.
- Strict all-feature/all-target Clippy for both packages: passed.
- Wrapper rc0, 80s, peak RSS 0.40 GiB, minimum available 14219 MiB,
  peak load 1.71. These are validation resources, not native memory bounds.

Reproduce the focused control with `just coroutine-sink-cancellation-check`.
The broader commands use `cargo test -p zencodec-coroutine -p zencodec-io
--all-features --lib --tests --locked`, the same selection with `--doc`, and
`cargo clippy -p zencodec-coroutine -p zencodec-io --all-features --all-targets
--locked -- -D warnings`.

Logs and runners remain under `~/tmp/coroutine-sink-stop-20261029/`. The first
fixture compilation omitted the required sink buffer method; that failure is
retained separately and is not behavioral evidence. The completion-only fixture
explicitly rejects an unexpected row-buffer callback. Native codec progress and
polling overhead are separate qualification; no new native timing claim is made.

All eight compiler catalogs and strict Rustdoc references regenerated; all 26
site source tests pass. Documentation wrapper rc0, 83s, peak RSS 0.35 GiB,
minimum available 14152 MiB. No new browser-layout qualification is claimed.
