# Unified component owner experiment — 2026-10-30

Private source is `perf/av1-policy-validation/src/component_owner.rs`, with
controls in `component_owner_tests.rs`. Base is `e4260797`; the native graph and
import-only AVIF patch are unchanged from the plane/grid experiments. No
production owner traits, error types or signatures change in this experiment.

The final focused `just plane-ablation-native-check` passes 18 tests and
all-target strict native-harness Clippy. Existing storage/grid controls remain.
New controls cover:

- Actual AV1 8/10/12-bit 4:2:0 picture lifetime after decoder drop; repeated
  presentation shares storage; all codes remain equal; the final presentation
  drops the actual picture (observed through its Weak owner).
- Packed pixels using the same mapping interface, without a separate packed
  owner trait at this experimental boundary.
- Missing/changed/backend failures retain their category, original typed cause,
  bounded default formatting and terminal mapping state.
- Caught native mapping panic cannot resume the guard.
- A timed-out optional Pulse refuses before a backend whose map method panics
  if invoked. The first draft control counted only plane calls; the final
  panic sentinel makes the entry claim decisive without weakening assertions.

Final receipt:
run-heavy: done rc=0 2s | peak-RSS 0.35GiB | min-avail 13647MiB | peak-load 0.74

Earlier successful drafts have separate `run.log` and `pulse.log`; final source
compilation is recorded in `final.log`. Full logs live under
`~/tmp/zencodec-component-owner-20261030/`; source export remains under
`~/tmp/zencodec-plane-native-20261030/`. No executable archive was created.

Scope limits: 52 unrelated native tests remain filtered. This is an ownership
and mapping contract experiment, not independent codec correctness, allocation
or performance parity. The adapter retains the original Frame and adds a boxed
map shim/grid vector. Complete image metadata, independent auxiliary retention,
map cancellation beyond entry, TIFF/JXL adoption, owner groups and production
API promotion remain unresolved. See the experiment README before reusing its
private signatures.
