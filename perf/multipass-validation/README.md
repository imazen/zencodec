# Multi-pass input contract experiment

This excluded harness tests a candidate driver and encoder interface. It is not
an implementation of the production zencodec traits. The starting driver and
native adapter came from Opus review revision `0ec4726`; independent review
replaced its detached replay token with a driver-retained raster input.

```rust,ignore
// Freeze and retain actual pixels, including stride and color context.
let input = multipass_validation::Input::raster(pixels);
// Or use a forward/positional source; its callback cannot claim borrowed proof.
let input = multipass_validation::Input::source(source);
```

The driver reads the retained raster itself on every pass. External source
replay uses explicitly disclosed digest/version evidence, or an admitted owned
snapshot. Digests detect accidental changes; they do not prove exact equality.
Exact replay permits only no replay, the retained immutable raster, or driver-owned
memory staging. Source pass budgets apply to every route, including staging.
When a budget requires staging, the replay callback can refuse its cost before
any pixel read. A direct raster that needs no staging makes no digest or raw
staging reservation.

Run public controls with `just multipass-check`. The isolated native crate
requires access to the private IMI repository and is explicitly opt-in:
`just multipass-native-check <artifact-directory>`. It calls the native codec's
public session API; no private codec implementation is copied here. Its pinned
graph now shares one zencodec, zenpixels and how-far family with the harness,
as recorded in the shared-core validation below. The driver protocol is still
experimental; dependency unification does not promote it into the public API.

The public controls exercise protocol misuse, swallowed interruptions, replay
changes, staging choices, pass limits, detached-proof substitution and direct
raster retention. Native IMI gates exercise byte/sample parity, padded rows,
forward and seek-back output, positional/disk/memory replay, source mutation,
cancellation, partial I/O and unsupported interpretation. Native full-suite and
strict-lint receipt (2026-10-27):
`rc=0 13s | peak-RSS 0.67GiB | min-avail 8289MiB | peak-load 0.95`.
Log: `~/tmp/zencodec-retained-raster-pass-budget-20261027.log`.
These are compilation/test resources, not codec performance or memory bounds.

Remaining contracts: merge replay decisions into the single PolicyResolver;
compose mandatory requirement scope across replay and output scheduling; unify
single-pass and multi-pass source/destination drivers; provide a bounded
production diagnostic trace; qualify total retention and asynchronous source
lifetimes. Zero zenpixels files changed for the exercised paths. Pixel-aligned
strides remain required for PixelSlice; the file source handles unaligned file
row strides internally.

## Bounded pass admission and final-pass output

The integrated pass-plan candidate declares a minimum and maximum before source
reads. A finite source-pass budget admits only a proven maximum within that
budget, or an admitted staging route with one source pass. Unknown maxima cannot
satisfy a finite source-pass requirement. The driver rejects an undeclared pass
before reading it.

Output-start phase is separate from byte timing: `PassOutput::FinalPass` forbids
bytes from earlier passes, and `OutputTiming` then measures lag within the final
pass. IMI takes its bounds and band size from the native plan accessor; it holds
single-band headers until finish. Pulse advances only for rows supplied, with an
unknown total until the pass count is resolved. The adversarial controls and
native parity tests from `../native-contract-validation/` accompany this change;
that directory retains the frozen baseline reproduction and candidate patch.

Integrated protocol/native tests and strict Clippy passed on 2026-10-27.
Command: `just multipass-check && just multipass-native-check <scratch>`.
Receipt: `rc=0 54s | peak-RSS 1.24GiB | min-avail 7907MiB | peak-load 1.16`.
Log: `~/tmp/zencodec-multipass-integrated-20261027.log`. These are validation
resources, not codec throughput or whole-operation memory measurements.

## Shared-core native validation (2026-10-28)

The native harness pins IMI revision `0007b75a` from
[its Pulse migration PR](https://github.com/imazen/zqi/pull/85). Its registry
dependencies now use the harness's zencodec through a root patch. The locked
graph contains one zencodec, one zenpixels and one how-far; the former second
zencodec 0.1.27 dependency is gone.

An isolated source export passed all 17 native tests and strict Clippy using
`just multipass-native-check <artifacts>`. The tracked dependency lock matched
the lock used by the run. This qualifies the existing multipass experiment;
it does not promote the candidate driver into the production API or qualify
the separate zenpixels precision migration. The run receipt is in
[`benchmarks/imij_core_unification_2026-10-28.json`](../../benchmarks/imij_core_unification_2026-10-28.json).

## Precision and descriptor admission (2026-10-28)

The candidate now enforces the production `Description::exact_depth` contract.
An incompatible claim refuses before native startup. Each filled request checks
active samples, including staged replay, before returning success to the codec.
The first contradictory absolute row remains available as the existing typed
`row_encode::ExactDepthContradiction`; swallowing the interrupted fill cannot
turn the operation into success. Padding is not part of the claim. The request
buffer belongs to the codec and has already been filled when verification runs;
the guarantee is refusal of successful delivery and subsequent publication,
not isolation from a codec that reads its buffer after an error. Earlier live
output is not rolled back.

The native adapter also checks descriptor primaries and signal range before
constructing its sRGB output description. Display-P3, unknown primaries and
narrow-range declarations refuse instead of being relabeled. This adapter does
not add color conversion, even when a separate supplied context contradicts the
descriptor. The existing matching-sRGB context control remains unchanged.

Both precision controls and the native color control fail behaviorally before
their fixes. The fixed public suite passes 26 tests; the native suite passes
19 tests, including the existing byte/sample controls and claimed-versus-unclaimed
byte parity for four native configurations with forward and seek-back output.
Strict Clippy passes for both harnesses. This is input-grid verification, not a
lossless guarantee for IMI's lossy output. No performance or native workspace
bound was measured. Exact revisions, commands and the initial test-lint failure
are retained in the [receipt](../../benchmarks/multipass_declarations_2026-10-28.json).

## Production promotion gates

The shared `EncodePlan` already represents source pass bounds and final-pass
output timing. The candidate still duplicates these as `PassCount`, `PassPlan`
and `PassOutput`, and still asks a separate `ReplayResolver`. Promotion should
reuse the existing plan and one joint operation-policy decision over the actual
job, source replay and output schedule; it must not clear unrelated requirements
to make separate questions succeed.

Source review of the pinned native session identified two prerequisites:
discovering pass bounds currently constructs native session state, and the
session's `Rc` mailbox/non-Send future cannot satisfy `DynEncoder: Send`.
Planning must obtain the same facts without constructing that state. Safe
native mobility must be implemented and tested before claiming erased parity;
no unsafe Send implementation or implicit thread bridge is proposed.

The same review found remaining candidate gates: check cancellation before
final cursor/flush effects and bound the diagnostic trace. These are source
findings, not executed regressions. Reader/file validation and typed source-side
stop causes are qualified below. Async pending-buffer
ownership, total retained memory and metadata/carrier qualification also remain
unproven. Passing the tests above does not close these production gates.

## Source validation and padding (2026-10-28)

The concrete file/reader adapters now validate width, descriptor, color context,
row extent and source stride before touching input or destination samples. File
requests also check the final active-byte offset for overflow before the first
read. File pitch need not be pixel-aligned; destination views retain their own
stride. Reader padding uses a fallibly reserved buffer of at most 4096 bytes,
with cancellation checks between chunks, instead of allocating the whole pad.
An I/O failure remains the original error; Pulse refusal carries a typed
`CodecError` with its original stop category through `io::Error`.

Four regression controls fail before the fix: invalid reader input consumption,
invalid file destination effects, unbounded/uninterruptible padding, and lost
typed cancellation. A fifth control preserves active samples and destination
padding with a seven-byte source pitch. All 31 public and 19 native controls
pass after the fix, with strict Clippy for both harnesses. The existing native
byte/sample and forward/seek-back controls remain unchanged. This does not
bound an arbitrary reader's allocations or make blocking reads interruptible.
Receipt: [source validation](../../benchmarks/multipass_sources_2026-10-28.json).
