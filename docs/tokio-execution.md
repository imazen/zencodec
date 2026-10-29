# Tokio execution and backpressure

The [code-first guide](https://imazen.github.io/zencodec/code.html#app-tokio)
shows caller, adapter and codec code, with complete source context and signatures.

## Responsibilities

- Caller: choose completed output or deliberately coupled live output, retain
  cancellation ownership, and treat failed publication as potentially partial.
- Adapter: fulfill positional reads, schedule CPU work, preserve original errors,
  and drain pending bytes only after confirmed writes.
- Codec: implement synchronous resumable work, honest output timing and strided
  rows; poll cancellation within expensive operations. No Tokio dependency is
  required by the codec interface.

`Encoder` itself does not require `Send`. `transcode_async` requires owned
`Encoder + Send + 'static` and `IncrementalDecode + Send + 'static` because it
moves their state into compute jobs. This applies even with `Inline`. `Send`
permits movement, not concurrent access or thread affinity. Native thread-bound
handles cannot simply acquire that guarantee; construct and retain such objects
on a dedicated worker, using a suitable message boundary instead.

## Current execution limits

`transcode_async` sends batches through `Compute`, then awaits the destination
with both codecs retained. Its final `EncoderSink::finish` also runs through `Compute`.
The regression first failed on the polling thread, then passed on the named
worker with exact encoded-byte and count checks after the fix. `Inline`
intentionally executes both batches and finalization on the polling task.
Offloading does not make an individual native call preemptible.

Finishing before awaiting publication releases the encoder sooner but retains
compressed output. A bounded output queue does not bound the codec's raw or
intermediate allocations. Live output is appropriate only with configuration-
specific retained-state evidence. The synthetic streaming tests establish
protocol behavior, not native codec memory bounds.

A dropped async future does not stop an already-dispatched compute job. Signal
its shared Pulse as well. A worker that drops its result, including unwinding,
can surface as `Disconnected`; `Compute` does not expose a JoinHandle. Instrument
the worker boundary when diagnosing the original panic. A cancelled or failed
`write_all` may have written a prefix without returning a count; do not retry
into the same destination as though nothing happened.

The blocking pull bridge occupies a worker during network waits. Its async
request fulfiller must run concurrently. Likewise, a bounded ChannelRowSink
requires a concurrently running receiver; its blocking send belongs on a worker.

## Qualification

On source `97ae82ba`, `just tokio-guide-check` runs the exact driver and work-quota
targets used by the guide, then strict Clippy. On 2026-10-29 all 34 driver tests
and 3 quota tests passed, with no ignored tests. The controls include concurrent
ranges, off-runtime batches, live writes, a blocked strip consumer, hung input,
cancellation and failed-batch publication. They do not qualify every native
codec, finish-time CPU scheduling, or whole-operation memory.

Command: `run-heavy --mem 3G --jobs 2 -- timeout 900s just tokio-guide-check`.
Receipt: rc0, 6s, peak RSS 0.34GiB, minimum available 14491MiB. This is a test-run
resource receipt, not a codec performance measurement. Full log:
`~/tmp/zencodec-tokio-guide/rust.log`.

The finalization repair passes 35 driver tests, the same 3 quota tests and strict
Clippy: rc0, 2s, peak RSS 0.29GiB, minimum available 14484MiB. Failing-first and
fixed logs are `~/tmp/zencodec-tokio-guide/finish-before.log` and `finish-after.log`.
The thread-placement regression is synthetic; no native codec speedup is claimed.

## Cancellation during publication

The finalization-boundary regression first observed `Prefix: Ok(10)` after the
writer signalled cancellation: the driver finalized, wrote the tail and returned
success. The driver now checks Pulse before successful-batch publication, after
that write, inside the queued finalizer before native execution, after finalizing,
after the tail write and after flush. Checks do not undo earlier output.

The executable [effect controls](../zencodec-io/tests/drivers/async_finalization.rs)
cover ordinary success, cancellation in the prefix writer, before queued finish,
inside finish, in the tail writer and during flush. A seventh case makes finish
both signal cancellation and return a native error; the original typed cause is
preserved. Earlier bytes are asserted exactly, along with finish and flush counts.
This is an effect-ordering guarantee, not prompt cancellation inside a native call
or an I/O future that does not wake. Use an application timeout/drop for a stalled
transport and signal the shared Pulse for dispatched native work.

Validation of this repair: the full zencodec-io all-features test command passes
151 tests/doctests, with one pre-existing ignored doctest. The selected driver
and quota targets also pass strict Clippy. The final fixture fails before the
fix at `Prefix: Ok(10)` and passes after it without changing assertions.
Logs: `~/tmp/zencodec-tokio-guide/cancel-before-final-fixture.log`,
`cancel-qualified.log`, and `cancel-io-all.log`. The full I/O test wrapper reports
rc0, 4s, peak RSS 0.27GiB and minimum available 14584MiB; this is not a runtime
performance comparison.

## Container seek-back completion

Synchronous media output shares the same rule: cancellation after restoring the
final output position must prevent a subsequent flush. The testkit regression
first observed one flush where zero was required. The repair checks the shared
Pulse between final position restoration and flush. Confirmed append counts stay
available; a real seek failure retains its I/O cause and Patch stage even when
that seek also signals cancellation. `just media-publication-check` covers the
media library and testkit. This fixture validates container effect ordering,
not native codec performance or an atomic rollback guarantee.

The media library and testkit pass all 84 tests/doctests and strict all-target
Clippy with all features. The failing-first and fixed receipts are retained at
`~/tmp/zencodec-tokio-guide/media-cancel-before2.log` and `media-publication.log`.
The latter wrapper reports rc0, 4s, peak RSS 0.28GiB and minimum available
14407MiB; no native performance claim follows from these controls.

## Cancellation before an immediately ready send

The channel bridge formerly checked Pulse only after a failed nonblocking
attempt. A cancelled `ChannelRowSink::begin` could therefore publish into a
channel with spare capacity. The failing-first control
`cancelled_strip_sink_does_not_publish_into_an_available_channel` reproduces
that effect; the wait helper now checks before every attempt. This also covers
retry attempts in the pull bridge. It does not make check-and-send atomic with
concurrent cancellation.

The driver/quota suite and strict Clippy pass after the fix. Logs:
`~/tmp/zencodec-tokio-guide/sink-cancel-before.log` (expected failure) and
`sink-cancel-after.log` (rc0, 2s, peak RSS 0.31GiB, minimum available 14478MiB).
The sleep backoff caps requested sleep at 2ms; it is not a wall-clock cancellation
latency guarantee. OS scheduling and arbitrary Pulse implementations can delay it.
