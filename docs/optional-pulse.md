# Optional per-call Pulse

The public migration uses `Option<&dyn Pulse>` for borrowed operation controls.
`None` means no additional per-call control. It must never disable a retained
job control, mandatory policy, resource admission or protocol validation.
Owned jobs continue to accept `SharedPulse`; its default is a no-op.

```rust,ignore
// No progress/cancellation source is required just to use the API.
decoder.step(input, sink, None)?;
// Add progress, planning, sharing and cancellation together.
decoder.step(input, sink, Some(pulse))?;
```

Forwarding and erasure preserve the Option, including absence. Code that performs
local work may resolve `None` to its retained control or `NoPulse`. It must not
replace an absent control with `Some(NoPulse)` before handing it to a native
codec that uses absence to select its retained control. An explicit per-call
no-op also cannot disable a retained stop.

A framework-supplied control in a callback is different: the caller receives an
already resolved operation control rather than being required to supply one.
Getters can likewise return a borrowed `&dyn Pulse` without optionality.

## Success, requirements and late facts

The default policy chooses an executable permitted route. Missing optional facts
are not a blanket reason to fail. Requirements such as verified metadata removal
or a finite native-workspace bound still need evidence before their affected
effects. A callback may choose a permitted adaptation when additional facts
arrive; it cannot silently discharge an unsatisfied hard requirement.

Discovery can continue while an obligation is pending only if the implementation
can withhold its affected effects under the admitted storage and execution plan.
Buffering, another permitted conversion or retention choice, and refusal are
situational choices. Do not require every caller to pre-coordinate capabilities,
and do not retry a native operation after it has produced effects.

## Qualification

The shared workspace runtime controls pass, including retained cancellation,
progress/sharing/planning, default success on unknown optional evidence, and the
existing callback that continues provisional discovery before accepting final
interpretation. Workspace doctests and strict Clippy pass. Standalone renderer
controls, no-default checks and the core/coroutine WASM build also pass.

[Qualification receipt](../benchmarks/optional_pulse_2026-10-29.json) records the
source snapshots and preserves the earlier fixture and orchestration failures.
This establishes the shared protocol, not every native codec's implementation.
The eight-crate generated API catalog and Rustdoc reference also build cleanly.
The guide browser and native adapters still require qualification against these
optional signatures. Earlier green platform CI covers the required-reference
contract, not this migration.
