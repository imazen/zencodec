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

## Choose the execution owner before choosing async syntax

| Execution | Suitable work | Failure mode to avoid |
|---|---|---|
| `Inline` | Short, bounded synchronous steps | One expensive step blocks all other work on that runtime thread; a work quota cannot preempt it |
| Tokio `spawn_blocking` | Finite blocking operations | Dropping/aborting the request does not stop a running closure; bound admission before dispatch |
| Dedicated Rayon pool | CPU work, including codecs that cooperate with that pool | A worker blocked on socket/row delivery occupies pool capacity; a fixed thread count alone does not bound queued jobs or retained images |
| Dedicated manual thread | Long-lived blocking parser or thread-affine native state | Unbounded thread creation, blocking joins on runtime threads, and worker lifetime outliving request accounting |

Tokio documents the distinction between blocking tasks and persistent workers,
and that running blocking tasks cannot be aborted:
[spawn_blocking](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html).
Rayon provides explicit pool size and panic handlers through
[ThreadPoolBuilder](https://docs.rs/rayon/latest/rayon/struct.ThreadPoolBuilder.html).
Its [spawn](https://docs.rs/rayon/latest/rayon/fn.spawn.html) has no execution-order
guarantee. Do not depend on adjacent codec batches staying on the same thread.

The executable [worker ownership controls](../zencodec-io/tests/drivers/worker_lifetime.rs)
show the actual code for two boundaries:

1. Acquire an owned permit **before** dispatch. Move it into the worker with the
   input and cancellation handle. Keep it until worker-local state is dropped.
   Aborting the request task must not make that permit available.
2. Construct `Rc` or thread-affine native state inside a dedicated worker. Return
   only a `Send` result. Join off the runtime polling thread. This demonstrates
   the ownership mechanism, not compatibility of a particular native handle.

A production supervisor must retain worker completion/panic receipts even if the
HTTP request disappears. `Compute::spawn` currently returns no receipt, rejection
or join handle. It can schedule a batch, but it is not an admission controller or
shutdown supervisor. Keep those responsibilities in the hosting adapter for now;
a new core executor abstraction needs a demonstrated consumer before adoption.

## Deadlock, spin and cancellation review

| Situation | Current behavior or required discipline | Remaining qualification |
|---|---|---|
| Full strip channel | Producer blocks; receiver must run concurrently; shared Pulse permits cancellation polling | Native retained-state bytes while blocked |
| Pull parser awaiting HTTP | Occupies a worker while async fulfiller runs | Starvation with several parsers and a saturated pool |
| Pulse-enabled channel wait | 64 initial spin attempts, then sleep backoff up to 2ms per requested sleep | CPU/cache cost and cancellation latency distribution under load |
| No Pulse channel wait | Uses channel blocking operation | Dropping the peer must wake shutdown paths |
| Work-quota yield | Limits synchronous steps per batch | One step can still take arbitrarily long; yielding is not a latency bound |
| Request abort while worker runs | Worker remains alive; retain its admission and signal shared Pulse | Hung native calls need explicit shutdown policy; no forced cancellation promise |
| Writer timeout/partial write | Prefix may already be externally visible | Terminate that response; do not replay the whole buffer on the same stream |
| Panic in dispatched work | Result channel can disconnect; executor panic policy is external | Preserve operation ID and original panic at supervisor boundary |
| Blocking nested pool call | Parent can occupy a worker while waiting for children | Test actual codec pool nesting; do not assume Rayon and native private pools share budgets |

Do not replace polling with a tight `try_recv` loop. A wake-driven cancellation
bridge would require a cancellation notification contract; arbitrary `Pulse`
currently supplies polling, so merely wrapping it in `Notify` cannot invent a
wake when its state changes. Polling's cost must be measured before selecting a
replacement or tuning the spin/backoff constants.

`select!` polls branches on the same task; blocking one branch blocks the others.
Recreating cancelled `write_all` or `read_exact` futures can lose partial progress.
Keep operation state and offsets outside retry loops, or abandon the transport.
Do not repeatedly cancel/recreate semaphore acquisition when queue position
matters. See Tokio's [select cancellation safety](https://docs.rs/tokio/latest/tokio/macro.select.html)
and [semaphore fairness](https://docs.rs/tokio/latest/tokio/sync/struct.Semaphore.html).
A large weighted request can also block smaller requests behind it; admission
policy must explicitly choose whether that fairness is desirable.

## HTTP proxy qualification: two distinct lifetimes

These are implementation/qualification requirements, **not a claim that a real
HTTP native-codec proxy has passed**. The existing guide uses extracted driver
controls and native in-memory source adapters. They do not cover HTTP framing,
connection reuse, disconnects or deployment pool sizing.

**Buffered codec route:** bound input/spool acquisition, inspect configuration,
admit CPU and workspace, execute on a bounded worker, finish into separately
budgeted encoded storage, drop raw/intermediate state, then stream the completed
object to the client. CPU admission ends when computation actually ends; encoded
storage admission lasts through network delivery. If metadata evidence is late,
resolve it before irreversible headers/output, or retain output until it can be
resolved. An HTTP 200 already sent cannot become a later policy refusal response.

**Sequential route:** select a demonstrated sequential codec/configuration and
pass-through orientation, retain bounded source/strip/output buffers, and run
input fulfillment and output consumption concurrently. Deliberate output
backpressure may pause encoding only while its retained state remains acceptable.
A slow origin and slow client must not deadlock each other. A route that needs
rotation, replay, global analysis or late privacy admission may require spooling
or a different route. Preserve the option to succeed through that alternative.

HTTP controls must include: chunked/short bodies, explicit EOF and truncation,
slow reader/writer, cancellation at each boundary, writer failure after headers,
Range/206 validation, ignored ranges, version/ETag change, retry ownership,
connection reuse, and bounded queues under overload. Reuse the existing positional
HTTP source/version checks; they are not evidence that all these proxy paths pass.
Pixel and metadata outputs must match an ordinary native reference route before
using the proxy for performance measurements.

## Adaptive concurrency is an experiment, not a memory guarantee

Keep four accounting lifetimes distinct: input/spool storage, active CPU work,
raw/intermediate state, and completed compressed output. A request-count limit
cannot substitute for these. An empirical memory estimate is not a proven bound;
strict policies still need evidence or refusal. Unknown costs should select an
explicit conservative/spool route, not silently count as zero.

Candidate controller: classify by actual codec/configuration, dimensions, sample
layout and pass plan; choose a measured starting concurrency; revise only future
admissions as observations arrive. Retain existing permits when lowering a target.
Re-estimate when new facts change the class **before** allocating the new state.
Do not wait for extra permits while every job holds partial reservations: that
can deadlock. Use atomic admission of the required resource vector, or a defined
release/requeue/spool transition. Never acquire the same exhausted CPU budget
again inside an already-admitted nested codec task.

Measure a fixed-concurrency baseline before evaluating an adaptive controller:

- Separate queue delay, compute service time and network drain time; slow clients
  must not be misdiagnosed as CPU saturation. Record cancellations and rejected
  requests too; successful-only latency conceals overload.
- Cross request concurrency with actual codec thread count and batch size, using
  identical frozen inputs and output parity controls. Include small/large and
  baseline/progressive workloads, mixed arrivals and slow-client phases.
- Use zenbench for repeated timing with interference checks; retain invalid runs.
  Use `perf stat`/sampling for cycles, instructions, cache misses and context
  switches, and heap profiling for retained/peak bytes. Run profiling separately
  from clean timing. Cache misses alone do not prove cache-churn causality.
- Compare the controller against the best fixed limit and an unchanged default
  on held-out workload sequences. Record throughput, tail latency, memory,
  wasted work after disconnect, fairness and recovery/oscillation. Choose an
  acceptance envelope before looking at the results.

Latency-based controllers such as [Netflix concurrency-limits](https://github.com/Netflix/concurrency-limits)
are useful prior art, not a ready-made codec memory model. No adaptive controller,
cache-churn reduction or optimal pool size is qualified here yet.

Worker ownership qualification on 2026-10-29: all 39 driver tests and 3 quota
tests pass, with strict Clippy. Receipt: rc0, 1s, peak RSS 0.32GiB, minimum
available 14635MiB; `~/tmp/zencodec-tokio-guide/worker-lifetime-fixed.log`.
The initial compile failure is retained in `worker-lifetime.log`. These controls
use explicit release channels rather than timing-based assertions; the five-second
timeouts are hang guards, not cancellation latency measurements.
