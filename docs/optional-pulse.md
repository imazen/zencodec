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
The guide source checks (24/24) and browser checks pass on `f4b9fdc1`,
including responsive roles, signatures, full-example copying and inline
expansion. Native adapters still require qualification against these signatures. Core CI on `feb09f9f` passes for the optional signatures. Its documentation
workflow fails in native integration: older container adapters and wrappers still
use required references. Those gates remain required; native qualification is
not inferred from the shared workspace checks.


## Related stack landing (2026-10-29)

The following reviewed stacks were merged and their heads verified as ancestors
of the remote main branches. These receipts cover their existing contract, not
subsequent optional-Pulse edits:

| Repository | Landed change | Merge commit |
| --- | --- | --- |
| zengif | [Retained Pulse and animation controls](https://github.com/imazen/zengif/pull/32) | `8756aa80` |
| zenwebp | [Native and shared Pulse controls](https://github.com/imazen/zenwebp/pull/105) | `59f8d7a8` |
| rav1d-safe | [Picture allocation accounting and packet context](https://github.com/imazen/rav1d-safe/pull/537) | `25c1e89e` |
| zenavif | [Packet context and picture admission](https://github.com/imazen/zenavif/pull/74) | `13cc0394` |

Picture accounting covers requested native picture planes, not total process or
operation workspace. The remaining native optional-control source changes must
be paired with one core/pixel dependency graph and tested before landing.


Native follow-up found a test-graph blocker: the bitmap, WebP and JPEG test roots
still resolve native-only `zenpng 0.1.4`, which pulls in `zencodec 0.1.26`. Its
fixed `DecodeOutput == 360` assertion does not admit the paired pixel buffer's
new representation. All three builds stop before native tests. Keep that
failure visible and align the PNG test dependencies; do not remove the tests or
loosen the old assertion to disguise the split. The isolated IVF/WebM controls
pass (27 tests, including explicit optional timeout-before-effects controls).
