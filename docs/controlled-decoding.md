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
