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

The coroutine's input cap remains scoped to its input store. Its existing row
copy, native parser storage and native workspace are not covered by the managed
`StorageBudget` yet. This authoring path adds no whole-operation memory claim.

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
