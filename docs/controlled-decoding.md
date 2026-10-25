# Mandatory interpretation boundaries

`ControlledIncrementalDecode` separates a required decision from an ordinary
`MetadataUpdate`. The controlled protocol lives in `zencodec::decode`; it is
additive alongside the existing incremental protocol.

```rust,ignore
trait ControlledIncrementalDecode: IncrementalDecode {
    fn step_controlled(
        &mut self,
        input: Input<'_>,
        sink: &mut dyn DecodeRowSink,
        stop: &dyn Stop,
    ) -> Result<ControlledProgress, StepError<Self::Error>>;

    fn resume(
        &mut self,
        boundary: BoundaryId,
        action: BoundaryAction,
        stop: &dyn Stop,
    ) -> Result<(), StepError<Self::Error>>;
}
```

`ControlledProgress::Decode(Progress)` carries ordinary byte/row/metadata
progress. `Suspended { consumed, boundary }` is a mandatory pause. Repeated
steps at that pause consume no input and produce no sink effects. A reply is
bound to the exact boundary identity. Stale replies, invalid mechanisms,
cancellation and ordinary failures terminate the session.

The still-image interpretation starts pending. Pending or provisional facts
permit `ContinueReading`: discover more facts while affected output stays
blocked. Final facts with a descriptor permit `AdoptInterpretation`. Unavailable
interpretation permits neither. These are codec mechanisms; the adapter decides
whether it can offer them under application requirements and storage admission.

`InterpretationFacts` contains output dimensions, an optional pixel descriptor
and `InterpretationState`. Its finality concerns the proposed pixels. It does
not certify that all metadata carriers have been inspected, that opaque bytes
are safe to retain, or that a native codec has bounded workspace. A profile
found in a complete container inventory may still differ from the profile that
governed the decoder's pixel operations.

`erase_controlled` applies `Fused` and preserves native error causes. The wrapper
holds a reported boundary itself, so an uncooperative native implementation is
not called again until a valid reply. It checks cancellation and rejects a legacy
`step` call at the pause. Controlled implementations must also refuse legacy
execution before any required decision can be bypassed. Unwinds poison the
fused session before native calls begin.

A final interpretation cannot change within the same still image after
adoption. Video configuration epochs and independent image items require their
own output contracts; this protocol does not silently reinterpret prior rows.

The core adversarial control deliberately uses a backend that accepts every
reply. The wrapper prevents repeated execution, rejects a discovery-only reply
to final facts, and blocks a legacy step without calling its sink. These tests
establish the mechanism, not native codec correctness or a memory bound.

## Coroutine authoring and transport execution

`CoroutineDecoder::controlled(input_cap, access, init)` creates a controlled
session. The codec sets its image info and output descriptor, then awaits
`ctx.interpretation(facts)`. The future is not polled beyond that await until a
matching reply arrives. Header notifications do not call `sink.begin` in this
mode. `write_rows` refuses before copying rows unless final interpretation was
adopted, and the runner checks the row descriptor and width before calling the
sink. Updates to source color, orientation, dimensions or output descriptor after
adoption are terminal. Descriptive-only metadata may still be announced.

`CoroutineDecoder::controlled_with_storage(input_cap, access, budget, init)`
admits input payload and a 64-span range index before calling `init`, then admits
row staging before sink effects. It reuses staging and releases its input/index
and spare rows at completion. The same budget can admit codec-owned buffers.
`controlled` retains its legacy input-store behavior when admission is not needed.
Neither constructor certifies native parser/workspace, image metadata, request
bookkeeping or sink allocations; this is no whole-operation memory claim.

`zencodec_io::PolicyDecoder::new(decoder, resolver)` serves these suspensions
through the shared `PolicyResolver`. It implements the existing
`IncrementalDecode`, so slice, positional-read, stream and async drivers retain
their input admission and scheduling behavior. The adapter records the chosen
mechanism, resumes the same native session, and returns to the driver before
polling more native work. A callback failure, cancellation or caught panic
leaves the adapter terminal. `report()` remains available after driver failure.

The authored late-fact fixture exercises short slices and positional reads,
strided rows, stale/replayed replies, callback error traces and redacted default
formatting, decline, cancellation, panic, early output and changed final facts.
Native JPEG qualification is separate work; a passing authored fixture does not
establish JPEG metadata authority or codec storage bounds.

```text
just control-check io-contract-check
run-heavy: done rc=0 14s | peak-RSS 0.37GiB | min-avail 17135MiB | peak-load 0.49
```

This receipt is for workspace tests and Clippy, not decoder working memory.


## Independent sink enforcement

The fused controlled wrapper also guards the actual sink. A backend cannot call
`begin`, final/provisional row access or `finish` before adoption, even if it
swallows the guard's refusal and returns successful progress. After adoption,
geometry, row extents and descriptors must match the adopted facts. A controlled
erased decoder refuses legacy execution from its first call. This adds no pixel
copy. Native algorithms remain responsible for their internal computations and
for the truth of their final facts; qualification must check those separately.

The early-effect regression failed before this guard with an actual call to the
application sink. The regression, coroutine tests, workspace tests and Clippy
pass after the fix.

## Native JPEG qualification

`perf/jpeg-policy-validation/src/control.rs` executes this protocol around
zenjpeg `de26db2f20c2290b7f4f93ea2377a14bcbec5c6d`. This qualification adapter
holds the whole encoded input in a managed allocation and feeds the existing
native row decoder. It does not make JPEG byte-incremental or establish a
whole-codec memory bound.

Baseline and progressive JPEG, with no ICC or a complete header ICC, agree
row-for-row with native direct decoding over short slice windows and positional
reads. An independent djpeg invocation accepts each source. The native row
constructor (which can decode progressive coefficients) and every row call occur
only after final adoption. The full probe inspects through the primary EOI;
profiles discovered only later, or split between header and later scans, receive
an unavailable interpretation and no executable offer. This is a qualification
limit, not a universal JPEG placement rule. Source inventory and pixel authority
remain separate contracts.

The native test exposed retained input after a policy refusal. `PolicyDecoder`
now drops its native session on error or unwind while retaining the decision
report. Thus keeping a failed adapter for debugging does not retain the native
session's managed allocations. Its metadata accessors return no native metadata
after failure. Admission failure, policy decline, unavailable interpretation,
callback unwind and native missing-EOI failure are checked before any sink effect.
Native error causes survive the adapter.

```text
just jpeg-policy-check control-check io-contract-check
run-heavy: done rc=0 8s | peak-RSS 0.33GiB | min-avail 17323MiB | peak-load 0.50
```

This receipt measures the validation process. Native workspace, source caches,
coroutine row copies and sink allocations are outside the input-owner allowance.
The qualification adapter is private; a production codec entry point must carry
these restrictions and use the same checked operation boundary.

The admitted range store uses fixed payload and index allocations. Sequential
append/read/release does not reallocate them; out-of-order insertion and hole
release compact within the same storage. A 64-span limit makes index exhaustion
a typed error without eviction. An independent byte map checks insertion,
duplicate feeds, holes and refills; overflow and index exhaustion leave retained
bytes intact. Object exhaustion before construction does not invoke the codec
initializer. Row admission failure occurs before sink begin.

Native baseline/progressive JPEG also runs with one shared allowance for the
coroutine input/index, whole encoded source and row staging. Pixel parity and
release after successful completion or policy decline pass. Native workspace
and the output sink remain outside that allowance.

## Stored requests and registered codec jobs

`DecodeRequest::start_controlled(&codecs, budget)` detects the source format and
uses the registered template, inherited limits/stop context and checked decode
policy. A supplied format is prepared before any input is read; detection takes
no prefix bytes and passes that same input to the selected session. A required
output descriptor is checked at final interpretation before policy adoption and
pixel effects. An explicit preference may accept the native fallback.

The codec implements `DecodeJob::dyn_controlled_decoder(preferred, storage)`;
`DynDecodeJob::into_controlled_decoder` delegates to that same primitive.
The default returns `ControlError::NotControlled`. Ordinary incremental support
never becomes implicit evidence of controlled execution. The request exposes the
existing controlled protocol, so `PolicyDecoder` and the established slice,
positional-read and async drivers remain the transport layer.

Registration controls cover detection, inherited-policy rejection, limits,
required-versus-preferred output, pre-effect refusal and reuse of a request over
two drivers. The initial authored controls establish facade behavior; the native
factory qualification below exercises the codec-owned implementation.

### Codec-owned JPEG factory

[zenjpeg PR 240](https://github.com/imazen/zenjpeg/pull/240), pinned here at
`3e9050efb1b33cc0e553b2f6712631979a505918`, implements the factory behind the
default-off `controlled-decode` feature. The native crate tests generic jobs,
erased jobs and registered requests, with baseline/progressive RGB8, RGBA8,
BGRA8, RGBX8, BGRX8 and grayscale output. Its input, input-index and row staging
share the supplied allowance. Native workspace and sink storage remain outside
that allowance; this route retains complete encoded input.

The integration harness now calls `DecodeRequest::start_controlled` directly
through a real `CodecSet`. Short slices and positioned reads agree exactly with
the same stored request's ordinary decode. Header ICC is qualified; late/split
ICC refuses before sink effects and retains `LateColorMetadata` in the report.
Admitted storage returns to zero even while the failed adapter is retained.

Float reconstruction still differs between the native one-shot and strip
paths ([zenjpeg issue 239](https://github.com/imazen/zenjpeg/issues/239)). The
controlled factory reports `UnqualifiedOutput` before affected work. The native
qualification also exposed invalid grayscale entropy geometry under a color
encoder configuration; `581ccc4e` fixes it on zenjpeg main with independent
decoder readback. These findings are distinct from interpretation suspension.

```text
just jpeg-policy-check
run-heavy: done rc=0 6s | peak-RSS 0.55GiB | min-avail 17493MiB | peak-load 0.21
```

This receipt measures the integration test/Clippy process, not native codec RAM.
The API and native factory remain under review; no crate release is implied.

## Diagnosing a refused boundary

`InterpretationFacts::with_unavailable(InterpretationUnavailable::...)` records
why a route cannot establish interpretation: late, forbidden or incomplete color
metadata, unsupported color, or an unqualified output path. These codes are
facts, not policy exceptions. Setting `Final` cannot authorize facts that still
carry an unavailability reason.

Every `DecisionRecord` retains `DecisionContext`, including refusals and callback
errors. A failed question therefore retains the interpretation facts even when
its `selected` field is `None`. Reports contain no metadata bytes, profile names,
paths or application error text; traced native causes remain separately inspectable.
