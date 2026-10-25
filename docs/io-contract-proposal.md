# IO contracts after implementation and transport experiments

Status: proposal for review, 2026-10-24. Tracking:
[159](https://github.com/imazen/zencodec/issues/159),
[160](https://github.com/imazen/zencodec/issues/160),
[161](https://github.com/imazen/zencodec/issues/161).
The [current contract](range-decoding-contract.md) remains the API reference.
No public library types or signatures in this proposal have been added.
The [minimal 0.3 proposal](minimal-0.3.md) places these driver changes in the
broader image/media boundary, removal candidates, and consumer experiment gates.

Keep `IncrementalDecode`, `ByteStepper`, `Input`/`Request`, `ReadAt`, and the
media session/container interfaces. The experiments justify driver admission,
bounded scheduling and explicit storage ownership. They do not justify another
general IO trait hierarchy. Disk and network sources can share the byte demand
protocol while choosing different fetch, caching and concurrency policies.

The evidence is in the [baseline investigation](../benchmarks/io_validation_2026-10-24.md)
and [implementation qualification](../benchmarks/io_contracts_2026-10-24.md).
Those reports preserve source pins, methods, raw summaries, resource-wrapper RSS
and qualifications. The executable candidates live in
[`perf/io-validation`](../perf/io-validation/README.md), outside the library API.

## What the implementations establish

| Candidate or repair | Executed evidence | Consequence |
|---|---|---|
| Safe positional `FileReadAt` | Exact bytes, sparse offsets above 4 GiB, shared-handle readers, warm/cache-miss benchmarks, syscall and heap profiles | Landed under existing `ReadAt`; no codec trait change needed |
| Reuse of active and batched input | Forward-only overlap/suffix tests; 156/156 random-source container cases | Landed for sync/async drivers; does not make arbitrary backtracking forward-compatible |
| One-shot writer fallback | Default-method and categorized-refusal regressions; actual BMP's 70 output bytes | Landed; refusal before row/output acceptance permits fallback |
| Output work batches | Exact synthetic and QOI bytes; delayed short writer; three scheduling variants | An output-aware quantum is useful; a row-by-row worker handoff is too costly in these runs |
| All-dependency range planning | Independent byte coverage, arithmetic, split, coalesce and descriptor tests | Valid candidate for region/read plans; cannot replace `requested()` alternatives |
| Reserved transfers and shared leases | Cancellation, concurrent reservations, ownership clones, malformed completion, native strided QOI | Valid payload accounting experiment; not a total process-memory guarantee |
| Bounded completion order | Delayed head, stale generation, ordinal-window admission | Transport completion order must be separated from decode order |
| Native AV1 sessions | Six 8/10/12-bit lossless geometries, queues 1/3/64, reset/retained frames, cancellation and presentation order | Existing media contracts need integration with IO limits, not a row-shaped video API |

The disk result is deliberately conditional. For 128 warm dispersed 4096-byte
reads, seek/read takes 109.44 microseconds and current `FileReadAt` 71.65.
With per-file eviction and verified kernel block-read charges, the same shape
takes 9.04 and 8.65 milliseconds. At 65536-byte sequential cache misses the
order reverses: 2.16 and 2.36 milliseconds. There is no universal cold-speedup
claim. Heaptrack reports 20 allocations at both 256 and 1280128 reads for both
adapters: this workload already reuses its buffers.

## 1. Admit input before allocating or reading

The next library addition should be a small `zencodec-io` limit object and an
explicit driver-limit error. Keep the fields private with checked constructors
and builders; the following names describe the proposed semantics:

| Proposed control | What it bounds |
|---|---|
| `IoLimits::max_live_input_bytes` | Driver-owned payload capacities across active windows, fetched ranges and in-flight transfers; one allocation is charged once across shared owners |
| `IoLimits::max_ranges` | Stored descriptors, including split/coalesced plans; check input and output counts before allocating metadata |
| `IoLimits::max_in_flight` | Started transport requests; a separate count from retained bytes and queued descriptors |

Apply limits to both sync and async drivers. The resident borrowed-slice path
does not acquire a mandatory allocation or shared owner; its caller-owned bytes
are outside the driver's allocation allowance. Codec allocations remain governed
by `ResourceLimits`, and media structural/queue limits by existing `MediaLimits`.
An end-to-end claim requires all applicable domains to be accounted, including
the HTTP client and backend. A driver input cap alone cannot establish it.

Every range needs checked `offset + length`, checked `u64` to `usize` conversion,
known-length validation where available, and fallible reservation before I/O.
Coalescing includes gap bytes in the reservation. A codec's minimum contiguous
request larger than the allowance is refused; splitting HTTP transfers does not
make that contiguous requirement disappear. Fetch concurrency does not authorize
speculative allocation past the retained-byte cap.

Add `BufferPolicy::try_for_pattern(pattern, max_bytes) -> Result<BufferPolicy,
SourceError>` as the checked policy selector. It can reuse `BeyondLimit` to
refuse an unsatisfied lookback. Selecting a policy establishes only that its
retention requirement fits; later input requests still need admission. Existing
`for_pattern` cannot communicate refusal, so do not give it stronger guarantees
by silently clamping a requested lookback.

The new driver failure must distinguish a driver-owned limit/allocation failure
from a codec error and from `Source::Error`; the generic source cannot manufacture
an arbitrary error for an allocation performed by the driver. This is an
additive public API review item, not a reason to erase the associated source
error type.

Current #161 remains open: the existing 8-byte spool can consume 1024 bytes
before refusing, and sequential policy selection can exceed its supplied cap.
The candidate allocator does not repair those library paths. Before landing the
API, tests must prove no oversized read is issued and no oversized retained
capacity is created, including EOF discovery at the exact spool limit. Unknown
EOF does not justify consuming an unbudgeted body chunk.

## 2. Distinguish alternatives from complete dependency plans

`IncrementalDecode::requested()` and `Demuxer::requested()` mean that satisfying
any one request permits progress. The current driver may prefetch several, but
they are not an all-required region inventory. Preserve that meaning.

[`plan_all`](../perf/io-validation/src/admission.rs) is a separate executable
candidate for callers that already know every dependency of a decode unit.
It validates and unions overlaps, bounds gap coalescing, and splits transfers
within byte and descriptor allowances. A region plan must include reference,
dictionary and checksum dependencies; silently reading unplanned payload bytes
invalidates a promised complete plan. A plan can be refined when later metadata
arrives, but that needs a new admission step.

Keep this planner internal to drivers/consumers initially. Export a common plan
type only after independent consumers need the same semantics. Priority, view
geometry, background prefetch and cache eviction policy belong to the caller;
the codec supplies format dependencies and access requirements.

Request count alone is not the objective. In the authored four-range example,
128 useful bytes become 160 transferred bytes with a small merge, or 16416
with a large gap allowance. Record useful bytes, overfetch, dependency rounds,
first usable output and total latency. Network round-trip savings and disk
read-ahead need separate measurements before choosing defaults.

## 3. Retain storage through its actual owners

The executable lifecycle is `Storage::reserve` -> `Transfer::append` ->
`Transfer::publish` -> cloned `Lease` -> final drop. Publication requires the
expected snapshot identity, exact length, and no earlier append overflow.
Cancellation frees payload storage before returning its reservation. Cloning a
lease retains the charge without charging the same allocation twice. A cache
eviction does not release a charge still held by a decoder.

The candidate's `Identity(u64)` is only an application-issued token. It does
not prove an HTTP object version. The allocator currently bounds payload
capacity and allocation count; its `Arc` and `BTreeMap` metadata allocations
are not fallible. Waiter fairness, transport scheduling and cache eviction are
not implemented. Keep the type experimental until those limits are explicit.

This ownership has a measured cost. The hardened candidate's complete
allocation/copy/clone/drop path takes 50.4 ns versus 16.4 ns for Vec/Arc at
1024 bytes, and 1233 ns versus 818.7 ns at 65536. These are uncontended
within-process experiments, not a network scheduler benchmark. Preserve direct
borrowing and reusable-buffer paths where the bytes do not escape the call.

Media already has `PacketPayload::Owned(Vec<u8>)` and shared `Arc<[u8]>` slices,
plus independently retained frame handles. Neither packet representation
currently carries an arbitrary IO reservation owner. Converting a lease's
slice into `Arc<[u8]>` copies it; releasing a sidecar reservation when `send`
accepts a packet is premature if the backend retains the payload. Before
promising cross-backend accounting, either attach ownership to the actual
payload owner in a reviewed API or explicitly transfer accounting to a bounded
backend domain. Do not hide that integration gap behind a zero-copy claim.

## 4. Validate transport publication and separate generations

An HTTP adapter should establish a stable object identity, selected
representation/encoding and total length where needed. Validate 206 status,
range bounds, response length and identity before publishing cache entries.
Multipart responses need complete framing and all admitted dependencies before
atomic publication. A strict adapter can reject coalesced/superset responses;
an adapter accepting them must validate coverage and budget the extra bytes.
These are adapter policies built on [RFC 9110 range semantics](https://www.rfc-editor.org/rfc/rfc9110.html#section-14).

An ignored Range returning 200 must trigger an explicitly admitted full-body
fallback or refusal. It must not silently spool an arbitrary file. Automatic
decompression must not make byte offsets refer to a different representation.
Retries need bounded attempts, body-idle timeout, cancellation and fresh partial
storage. Invalid or incomplete retries publish no new cache bytes.

An object's identity and a view/seek generation are separate. Cancelling an old
view can cancel its pending work without invalidating already verified bytes
for the same immutable object. An old-generation completion cannot publish into
the new view or feed the new decode sequence. A multi-object representation
needs an immutable manifest/version contract; independently observing each
part's first ETag does not prove one coherent revision.

Transport validation is also separate from format completion. Final pixel rows
mean that later passes will not replace those pixels. They do not by themselves
prove that trailing metadata, integrity checks and final output status have
completed. Keep completion/validation state available to callers that render
early; authoritative color must be known before an irreversible downstream
encode. [Issue 154](https://github.com/imazen/zencodec/issues/154) tracks the
shared color handoff.

## 5. Bound compute batches and preserve media order

Separate CPU fairness from transport backpressure. A maximum-steps `WorkQuota`
may yield a worker without waiting for a client to read. Coupling further encode
work to draining output is appropriate only for a configuration whose retained
raw/intermediate state is bounded and whose upstream can stop supplying pixels.
`OutputTiming::streaming`, a small output lag, or a codec-wide capability flag
does not establish that ownership/release property.

| Admitted operation | Scheduling policy |
|---|---|
| Verified sequential decode/encode with bounded retained state and pausable upstream | Bound pending encoded bytes and propagate a slow destination upstream; this is essential to keeping the whole pipeline bounded |
| Encoder or caller already owns a complete raw image, coefficient planes or substantial lookahead | Prefer completing an admitted work unit and releasing that state into separately budgeted encoded storage before waiting on a slow destination |
| Retention/release behavior unknown | Account the whole admitted job conservatively; do not infer a streaming memory guarantee or select output-coupled scheduling from early bytes alone |

The encoded staging store needs its own byte/job cap or explicit disk-spool
policy. A full store stops new admission; it does not authorize unlimited
encode-ahead. Required capacity must be admitted before starting work that
cannot safely suspend. Output backpressure still exists at that storage boundary.
Cancellation and CPU fairness remain active in every mode.

The [native QOI retention control](../benchmarks/io_encoder_retention_2026-10-25.md)
demonstrates the distinction with real drivers: both wrappers report streaming
output, but the deliberately retaining wrapper still owns 786432 raw bytes at
the first blocked write. Completing to owned encoded output first releases them
before that write. The non-retaining control owns no copied raw rows there.
All three emit the same 4255 QOI bytes. This is an ownership counterexample,
not evidence that native QOI retains a whole raster or a general spool solution.

The 64-row/65536-byte experiment reduced the high-expansion fixture's first
write from 1024 produced rows to 64 and pending output from 1048576 to 65536
bytes. It used 18 pool jobs; yielding every row used 1026. QOI parity holds,
but extra worker handoffs still cost time. These values are experiment points,
not selected library defaults. They qualify that sequential workload only;
they do not justify applying output-coupled scheduling to buffering encoders.

A decoder step, encoder call or finalizer is indivisible to the driver. A
single whole-frame step still queues 1048576 bytes in that experiment. Thus
`WorkQuota` is a scheduling control, not a hard memory or wall-time limit.
Native codecs must expose appropriate work units and enforce their own
allocation/cancellation contracts. Required bounded operation must refuse an
incompatible whole-file fallback before committing to it.

Measure raw/intermediate ownership at a stalled writer, time until those
allocations are released, peak allocated storage and aggregate occupancy across
slow clients. First-byte latency and pending encoded bytes alone can reward a
schedule that keeps much larger raw allocations alive. A caller-owned complete
image also remains live while borrowed by a slow streaming call, even when the
encoder itself is sequential. The operation plan must include that ownership.

The tested `Ordered` candidate admits completions only in the next bounded
ordinal window, preserving a slot for the missing head. Reserve the head's
transport bytes before later requests; a reserved queue slot alone cannot
prevent storage starvation. Production scheduling must also bound all selected
tracks together so an unread audio/subtitle queue cannot absorb unlimited data.

Demuxed packets feed the decoder in decode order. The existing `VideoDecoder`
returns frames in presentation order. EOF means `end_input` followed by receive
until `EndOfStream`; seek means discard obsolete queued work, reset, configure
the right epoch and restart at an admitted random-access point. Returned frames
remain valid after reset. These existing native session behaviors passed the
AV1 tests; the entire network/multi-track pipeline has not been qualified.

## 6. Keep output patching and source replay separate

Media already supplies `MuxHint::forward_only`, `MuxTiming` and
`MuxFinish::patches`. Reuse that capability split when a concrete image encoder
needs patchable output. Admit the destination before sending bytes; validate
patch offsets and lengths against emitted output and bound retained patch
storage. A forward-only destination chooses a supported layout or refuses.

A multi-pass encoder separately needs replayable pixel input or an explicitly
admitted scratch store. Seekable encoded output cannot recreate source pixels.
`with_prefer_streaming` remains a preference: it should not silently change
explicit effort/fidelity settings. Required streaming/bounded behavior needs
an operation-level admission result for the actual configuration.

Avoid another universal facade until it can disclose native versus whole-file
fallback, source access, output timing/patches and replay requirements honestly.
Observed strip height alone cannot prove bounded internal memory.

## Scope of the next API review

Start with `IoLimits`, a driver limit failure, checked `BufferPolicy` selection,
and a CPU-fairness `WorkQuota` in `zencodec-io`. Output-coupled scheduling needs
an explicit, configuration-specific admission policy; it is not the universal
meaning of that quota. Keep range plans, shared leases, HTTP validation
and completion scheduling private while integrating actual consumers. Leave
codec signatures and the existing media payload/session APIs unchanged in that
first review. Changes needed to propagate a lease into backend ownership are
a separate, concrete review, with allocation evidence.

[nostdio 0.2.0](https://docs.rs/nostdio/latest/nostdio/) supplies synchronous
Read/BufRead/Write/Seek compatibility, cursors and endian helpers; with `std`
it re-exports standard IO traits. It can serve an actual pull-parser adapter.
Its interface does not express suspended input, asynchronous range completion,
object identity or owned-storage admission. No dependency or duplicate progress
interface is proposed for zencodec core.

The pending-pr qualification remains specific: zenavif's native session tests
ran with [zenrav1e44](https://github.com/imazen/zenrav1e/pull/44) at `9057c4ba`.
[zenpipe86](https://github.com/imazen/zenpipe/pull/86), head `330eea4b`, has not
been run end to end here. [WebP105](https://github.com/imazen/zenwebp/pull/105)
at `6e94c8ec` concerns animation/color/cancellation; its older performance
report does not measure that newer cancellation change. [JXL20](https://github.com/imazen/zenjxl/pull/20)
at `56184931` keeps compressed input for frame-at-a-time animation; that is not
proof of network-fed image decoding. These states were rechecked after the
experiments. Earlier review statements and format-potential tables are design
inputs, not substitutes for native execution at the cited revisions.

Remaining qualification gates are browser HTTP/2/CDN behavior and controlled
WAN throughput; large-frame and full multi-track pipeline scheduling; allocator
failure and contended scheduler fairness; late-color/integrity display policy;
and other codecs' native incremental performance. The cache-miss disk and
native AV1 correctness gates now have measurements. Forward MP4 remains 4/108
cases: [zenavif72](https://github.com/imazen/zenavif/issues/72) needs demuxer
work, and no new trait can make the observed backwards reads forward-only.

## Native error provenance, 2026-10-25

Source reviewed at `ae41e8a1`: `FileReadAt::io_err` and the read path in
`BufferedReadAt` turn `std::io::Error` into `SourceError::Io { message }`.
`SourceError` exposes no underlying cause. Keeping typed callback errors or
propagating a driver result with `?` cannot recover an error already flattened
inside its source adapter.

The existing `just io-contract-probes` now opens the same absent path directly
and through FileReadAt. Observed:

```text
file_error native_kind=NotFound native_cause_preserved=false
run-heavy: done rc=0 4s | peak-RSS 0.45GiB | min-avail 17748MiB | peak-load 0.18
```

The command also retains its previous exact-output/stalled-writer controls.
Scoped example Clippy passed. This is a process receipt, not an I/O performance
comparison. The probe prints behavior instead of asserting that cause loss is
desirable. BufferedReadAt's equivalent flattening is source-verified here; this
new native file observation does not exercise that path.

The next driver API review should retain the original transport/filesystem
cause, operation stage and structured range failure. A std-only driver error
envelope can preserve `std::io::Error` without adding std to the core error
vocabulary. Do not force callers to parse a localized message, or infer a retry
permission from the error kind alone. Source replayability and output
commitments still control retries. No production error signature changed in
this investigation.
