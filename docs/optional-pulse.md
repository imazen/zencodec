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
expansion. Native qualification is partial, as detailed below. Core CI on
`feb09f9f` passes for the optional signatures. Its documentation
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
operation workspace. The remaining native optional-control changes must be paired with one
core/pixel dependency graph and tested before landing.


## Native optional-control qualification

[PNG PR 48](https://github.com/imazen/zenpng/pull/48) is merged as `a5f2a588`,
including optional controls and the joint encoding-route callback. Its portable
root and fuzz graphs, native integration, metadata rewrite, row planning,
shared contracts, strict Clippy and no-default build passed before merge.
This is separate from the earlier required-Pulse stack receipts above.

The portable JPEG and WebP graphs also pass the selected native runtime and
strict Clippy controls. [WebP PR 119](https://github.com/imazen/zenwebp/pull/119)
is merged as `d5893295`, including separately audited fuzz/recompress locks.
[JPEG PR 250](https://github.com/imazen/zenjpeg/pull/250) is merged as
`26cbe9b8`; [bitmap PR 35](https://github.com/imazen/zenbitmaps/pull/35) is
merged as `595949e0`.
Bitmap runtime controls pass; the inventory helper repair now passes strict
Clippy with and without inventory.
These are focused contract checks, not full-codec or cross-platform certification.

| Codec / source | Checked behavior | Remaining qualification |
| --- | --- | --- |
| PNG `4b75d4d5` (landed with receipt and fuzz lock as `7b6d1436`) | Incremental decode, controlled adoption, metadata rewrite, declared row planning and shared contracts; fuzz bins compile | Post-merge CI passed; no release publication |
| WebP `6cba688f` | Optional calls preserve retained cancellation; supplied progress and stop polling; incremental sample parity; strict Clippy | Fuzz bins and recompress library also compile on the audited portable graphs; broader feature/platform coverage remains |
| JPEG `265a0447` | Native optional helpers, controlled decode, metadata rewrite and row planning; strict Clippy | API snapshot generated and verified; paired fuzz graph and all fuzz bins pass; broader feature/platform coverage remains; one inherited ignored case remains ignored |
| Bitmaps `54f98180` | Retained stop polling, QOI and bitmap incremental controls | Feature-gate repair `769e6547` passes both strict library configurations; paired fuzz graph and all fuzz bins pass |
| IVF/WebM `9c118829` | 27 controls passed in an isolated dependency override, including timeout before effects | Portable graph and strict Clippy; not a portable landing receipt |

The initial bitmap/WebP/JPEG builds failed because native-only `zenpng 0.1.4`
pulled in `zencodec 0.1.26`, whose fixed layout assertion rejects the paired pixel
representation. Aligning the PNG test dependency with the new shared graph
removed that split without changing the old assertion. JPEG additionally uses
an existing metric revision with identical consumed crate source and a widened
PNG requirement. The reviewed lock changes retain exactly one shared core,
pixel, converter and Pulse package in each checked root.

The combined three-codec wrapper still reports failure because bitmap Clippy
failed. Successful JPEG/WebP steps do not erase that failure. The
[receipt](../benchmarks/optional_pulse_2026-10-29.json) records both results.
The AVIF/container harness now passes on the optional signatures, as recorded
below. IMI, native AVIF-specific regression controls and the complete zenpipe
graph still need qualification.


The follow-up at bitmap `7d89635c`, WebP `fb4388e2` and JPEG `31b789f3`
preserved its aggregate failure: bitmap and JPEG standalone fuzz locks still
select pixels 0.2.16 before the paired 0.2.17 source can resolve. Targeted lock
updates are required. The WebP fuzz/recompress audits and compile checks,
bitmap lint configurations, and JPEG API generation/check passed. The generated
JPEG API diff contains exactly the two optional helper arguments. Existing
ambiguous rustdoc links still warn; this is not a warning-free rustdoc claim.


The targeted JPEG/bitmap fuzz follow-up passed after resolving the paired pixel
packages. Bitmap's lock changed only core, pixels and converter. JPEG additionally
needed converter's lz4_flex 0.14.0 requirement; its complete transition matched
the earlier reviewed root dependency change. The initial audit failure remains
recorded, followed by the explicit review and passing JPEG compile. JPEG's
inventory fuzzer retains an existing deprecated-policy warning. These are
compile checks, not fuzz campaigns or full feature/platform certification.

## Native media and executable guide follow-through

At `cfd8734b`, the guide's 87 executable tests and strict Clippy pass. The native
AV1 harness passes 47 tests and strict Clippy with AVIF/MP4 `94116d26` and
IVF/WebM `403d46b9`; the container repositories' suites pass 61 tests and strict
Clippy through the isolated guide consumer graph. Independent float readback and
deliberate-clipping controls pass for 54 presentations, the Gamma22 verifier
passes, and the transport simulation CSV is unchanged. The AV1 lock changes
only the six explicitly selected package sources. The guide lock is unchanged.

CI on `441399e6` also passes the matching Rustdoc/site build, source/browser
checks and native TIFF/JXL spatial selection. Its failed native I/O resolution,
old AV1 lock and old zenpipe signatures are retained as separate failures. The
first two now have the local qualification above; JPEG/zenpipe integration still
needs its updated dependency graph. None of these receipts certifies all native
features or platforms.
