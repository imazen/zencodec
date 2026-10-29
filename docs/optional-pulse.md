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

Source migration in progress. Core controls cover `None` through erasure and a
fused decoder, retained cancellation, full reporting/sharing/planning with `Some`,
and default policy success versus mandatory refusal on the same unknown facts.
Companion and native implementations need the same signature migration. These
new controls are not qualified until the serialized tests run; earlier green CI
covers the prior required-reference contract.
