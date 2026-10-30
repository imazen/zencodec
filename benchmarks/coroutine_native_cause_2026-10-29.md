# Coroutine native cause versus simultaneous cancellation

Base: `6472a697e711756d2d562288c164b3dd5330e30a`.
Failing-first control: `f448302a`.

A coroutine poll returned a native UnexpectedEof and signalled cancellation.
The adapter's following Pulse check replaced that concrete cause with Cancelled.
The control fails before repair with `Err(Cancelled(Cancelled))`.

The final implementation extracts the pending event and checks recorded protocol
violations first, then retains the native error, then polls cancellation before
successful or pending work proceeds. This preserves the prior protocol priority
and releases extracted pending event data on failure. It does not add a successful-
path lock or a new public interface. Native failure retains its typed payload;
terminal state and pending-detail destruction remain enforced.

The control exercises both Cancelled and TimedOut with the exact EOF offset,
length and total, checks unavailable completion details, destruction before the
decoder is dropped, no sink finish and refusal on a subsequent None-Pulse call.
The initial fixture used a nonexistent field name and failed compilation; that
log is retained separately and is not behavioral evidence.

ARM64 Rust 1.96.0, run-heavy 3G/jobs2, timeout1200:

- Coroutine/I/O all-feature library/integration tests and doctests: 203 passed,
  0 failed, 2 existing ignored doctests.
- Both packages' all-feature/all-target strict Clippy: passed.
- Final wrapper rc0, 19s, peak RSS 0.28 GiB, minimum available 14380 MiB,
  peak load 0.96. These are validation resources, not codec memory bounds.

An earlier repair also passed (rc0, 81s, peak RSS 0.40 GiB); source review refined
it to retain pending-event protocol precedence before native error selection.
Both logs remain. `just coroutine-io-check` reproduces the complete gate;
`just coroutine-sink-cancellation-check` includes the focused details controls.
Full logs/runners: `~/tmp/coroutine-native-cause-20261029/`. No native codec timing
or cross-platform runtime qualification is inferred from these controls.

All eight compiler catalogs and strict Rustdoc references regenerated; all 26
site source tests pass. Documentation wrapper rc0, 84s, peak RSS 0.35 GiB,
minimum available 14170 MiB. No new browser-layout qualification is claimed.
