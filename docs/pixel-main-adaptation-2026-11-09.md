# Adapting zencodec to current zenpixels

Source review and isolated compatibility checks, 2026-11-09. This is a proposed
migration, not an implemented API change or release approval.

## Sources and review status

The dedicated review checkout is synced to zenpixels main
[`f2d2bfde352124dcaa9937c64b81596144d42675`](https://github.com/imazen/zenpixels/commit/f2d2bfde352124dcaa9937c64b81596144d42675).
Existing dirty checkouts were preserved. Remote PR metadata was refreshed:
zenpixels has **no open PRs** at inspection. Closed proposals are historical
inputs, not additional branches to merge.

The important merged changes are:

- [PR121](https://github.com/imazen/zenpixels/pull/121): growable planar layouts,
  per-plane validation and accessors.
- [PR120](https://github.com/imazen/zenpixels/pull/120): precision policy and
  explicit HDR intent, including luminance anchors.
- [PR144](https://github.com/imazen/zenpixels/pull/144): checked metric construction,
  planar lending, `PlanarSlice` and prepared interpretation binding.
- [PR145](https://github.com/imazen/zenpixels/pull/145): one `SampleEncoding`,
  coupled operations/output context, fallible descriptor construction and worker
  cloning. Speculative `NativeCodeSlice` was removed.
- [PR146](https://github.com/imazen/zenpixels/pull/146): in-place target validation
  and luma-policy checks before CMS dispatch.
- [PR147](https://github.com/imazen/zenpixels/pull/147): a reusable downstream
  compatibility harness; its receipt covers core main, not our review stack.
- [PR148](https://github.com/imazen/zenpixels/pull/148): recognition retention and
  revocation of an unqualified ICC substitution permission.

Earlier pixel PRs 123–130 are closed; their final CMS/export work is integrated
in main through the reviewed consolidation. In particular, af19 is now a useful
historical numerical control, not the preferred future dependency target.

The main pixel implementation reference is
[design-reference.md](https://github.com/imazen/zenpixels/blob/f2d2bfde352124dcaa9937c64b81596144d42675/docs/design-reference.md).
Older design documents sometimes describe pending work that is now merged;
source and current PR dispositions take precedence.

## Keep the ownership boundary

| Responsibility | Owner and direction |
|---|---|
| Samples, stride, storage type, payload depth/shift | zenpixels `Plane`, `AnyPlane`, `SampleEncoding` |
| Roles, chroma subsampling/siting, checked borrowed planar picture | zenpixels `PlanarLayout` and `PlanarSlice` |
| Current pixel color, ICC authority, white/peak anchors | zenpixels `ColorContext`; retain with delivered samples |
| Conversion, HDR rendering, CMS execution and resulting context | zenpixels-convert prepared plans |
| Native owner/mapping lifetime, coded crop/render geometry, timing, presentation, epoch | zencodec `Picture`, `Frame`, `FrameInterpretation` |
| Collections, auxiliary meaning and source provenance | zencodec `ImageOutput`/collection APIs |
| Admission, late evidence, publication, replay and I/O | zencodec policy/state machines and adapters |

Do not move `Frame` into zenpixels or add another general image hierarchy.
`PlanarSlice` is a CPU view borrowed from a mapped owner; it is neither that
owner nor a timed presentation. A metric accepting it does not establish that
our renderer or codecs can process every layout it describes.

## First contract change: current planar interpretation

Current `src/frame.rs::FrameInterpretation` has geometry, layout, `ColorClaim`
and epoch. `ColorClaim` contains CICP only. This cannot transport an authoritative
ICC, a relative-light white anchor, a declared HDR peak or alpha association.
Those are concrete inputs to `PlanarSlice::new` and subsequent processing.

The existing pixel constructor is:

```rust,ignore
let samples = zenpixels::plane::PlanarSlice::new(
    &planes, &layout, coded_width, coded_height, &current_color, alpha_mode,
)?;
```

`current_color` must describe the held samples, and `alpha_mode` must state their
actual association. Neither can come from a generic codec default.

The next public API review should put a retained **current `ColorContext`** and
explicit **alpha association** on the immutable planar interpretation. Empty
context means unknown; it must not imply sRGB. Alpha interpretation must be
present exactly when the layout contains alpha. Do not infer premultiplication
from the role or codec family. An alpha-only layout can be transported without
being accepted as grayscale color.

Keep container/bitstream claims as provenance. Avoid two independently writable
current color authorities: `ColorClaim::resolved` must be clearly source-format
resolution, while current context describes the emitted samples. Converted
frames get a fresh interpretation; original claims remain source provenance.
This is a proposed declaration change, requiring native adapter controls before
its signatures become permanent.

The desired consumer shape is a *proposed*, scoped convenience:

```rust,ignore
frame.with_planar(|samples: zenpixels::plane::PlanarSlice<'_>| {
    metric.evaluate_planar(samples)
})?
```

Implementation requirements are more important than this spelling:

1. Retain the native mapping guard throughout the callback.
2. Validate every plane against the held layout, using `check_plane`/`check`.
3. Preserve per-plane pitch, code placement, matrix/range, siting and current color.
4. Validate alpha association; preserve coded geometry and crop phase separately.
5. Borrow from that exact interpretation, not a mutable current decoder epoch.
6. Assemble borrowed plane handles once per map, never copy sample buffers or
   allocate a new list per row. `PlanarSlice` needs a borrowed contiguous handle
   slice; a locally assembled slice cannot escape its mapping owner.

This helper cannot be implemented honestly from existing frame fields for every
input. Supplying a guessed context/alpha to make it compile would hide the gap.

## Consolidate existing validation before adding more views

`src/image.rs::ComponentMap::plane` repeats geometry/type checks already exposed
by `PlanarLayout::check_plane`. Use the pixel validator internally, preserving
our typed mapping diagnostics and independently comparing recorded encoding.
Both `Frame::map` consumers and `ImageOutput` consumers need the same checked
boundary; currently only the latter validates agreement.

`MappedPlanes::encoding` still defaults to `None`; `ComponentMap` substitutes
the interpretation's declaration. A layout validator checks storage type and
shape, **not whether U16 words were actually written as LSB10 or MSB10**. Require
an explicit encoding declaration for native owners before freezing this API,
or expose a clearly unchecked low-level path and make consumers use the checked
one. Neither choice can detect a backend lying about its bytes.

Keep growable `PlanarLayout`: RGB/YUV color layouts are currently bounded, but
that is not a reason to freeze a general image/auxiliary model to four planes.
Continue separating arbitrary auxiliary/depth/spot channels from this layout.

Thin `image::CodePlane` toward pixel component readers; do not alias it blindly.
It describes a whole mapped plane with component count and row access, whereas
`zenpixels::sample::CodePlane` is a selected component reader. Cache/select that
reader outside a per-pixel loop. Cost is source-inferred; no speed claim.
Use the fallible `AnyPlane::stride_bytes` rather than repeating type dispatch;
retain `GeometryOverflow` instead of returning a sentinel zero. Mark `Samples`
and `Row` extensible before release. Interleaved F32 still needs explicit
component semantics; exposing word rows is not a spatial component API.

These extend [issue198](https://github.com/imazen/zencodec/issues/198). Its
mixed-layout equality and alpha-as-gray findings already have core regressions;
the mapping/encoding/lending concerns remain. Do not reopen the fixed findings.

## Conversion and policy

Retain the renderer's existing `OutputPlan` boundary. It already binds the exact
source descriptor/context and consumes the converter's output context. The new
pixel implementation fixes wrapper disagreement inside that boundary; zencodec
should not rebuild white anchors or restore source ICC after conversion.

A complete execution flow is:

```text
hold decoded output at a real suspension
  -> establish actual current samples/context
  -> prepare executable pixel plan and inspect its output metadata
  -> admit codec carriers, acquisition/output schedule and resource evidence
  -> run rows with Pulse checks
  -> verify final obligations before publication
```

`OutputPlan` is a color plan, not proof of encoder carrier support, metadata
privacy, memory admission or lossless encoding. Continue resolving those jointly
against the retained native job. A prepared CMS scratch size is not a bound on
the whole operation. `OutputPlan::prepare`/CMS calls are synchronous; checking
Pulse around them does not preempt the middle of one call.

Use one application callback through `OperationPolicy`. A private adapter may
translate executable conversion offers; callers should not need to coordinate
an independent conversion resolver with the codec resolver. Mandatory
requirements remain filters, while defaults can choose a supported successful
route. Unknown late facts require a real suspension before affected effects;
callbacks cannot undo already published bytes.

Keep three precision concepts distinct:

- `SampleEncoding`: actual stored integer payload, including 10/12-bit shifts.
- Pixel precision facts: modeled capacity of the current processing samples.
- `ExactDepth`: evidence obtained from current sample values, invalidated by
  processing unless re-established.

`image::SourcePrecision` describes original auxiliary provenance, including
floating-point exponents; it is not a duplicate of the pixel capacity model.
Do not replace it by name alone. Neither original 10-bit signaling nor modeled
capacity proves that processed RGB is exactly representable in 10 bits.
Filtering, resizing, CMS conversion and compositing need evidence invalidation.
Pixel `ConversionEffect::preserves_modeled_capacity` deliberately makes a
narrower promise than pixel equality or codec losslessness.

Keep CMS backends explicit. The prior native CMYK gate demonstrated that Mox
can integrate correctly yet miss the reference tolerance; LCMS55 passed that
same witness exactly. Advancing to main requires rerunning the actual native
control, not assuming adapter identity or a compile pass proves accuracy.

## Current zencodec PR disposition

Current heads below were fetched, not silently rebased or merged. A combined
release branch should integrate these small deltas once, then select one tested
pixel cohort. Avoid repeatedly repinning every historical native harness.

| PR / current head | Recommendation |
|---|---|
| [199](https://github.com/imazen/zencodec/pull/199), `88fc5871` | Retain gray/RGB distinction and source-depth preference. Fold the HLG native-carrier correction and PR201 authority guard into the release stack before adopting pixel main. Source alone still extracts authoritative ICC CICP without `valid_use`. |
| [201](https://github.com/imazen/zencodec/pull/201), `af9dc2b5` | Retain authority guard and independent controls. Migrate the gray round-trip test described below; a raw `to_cicp()` result is no longer permission to emit replacement CICP. Keep frozen CMYK failures as explicit historical controls. |
| [200](https://github.com/imazen/zencodec/pull/200), `490e0b29` | Net diff against main is docs/recipes/probe, not a replacement sans-I/O runtime. Retain existing state machines and optional Pulse. Refresh its native probe cohort with the integration stack. |
| [196](https://github.com/imazen/zencodec/pull/196), `dc128a4f` | Keep IQA in the optional adapter. Its current packed-only admission is intentional. Adopt pixel main's shared planar view through the scoped mapping boundary; do not add another metric or frame hierarchy. |
| [188](https://github.com/imazen/zencodec/pull/188), `7ba092d8` | Timeline/edit-list work remains a separate media contract. A planar sample view does not solve edits, display time or mux admission. Rebase without changing time semantics; native replay/playback gates still apply. |
| [189](https://github.com/imazen/zencodec/pull/189), `d13cd9e7` | Manifest-only publication preparation. Rebase after the common dependency cohort is qualified; it authorizes no publication. |
| [158](https://github.com/imazen/zencodec/pull/158), `13da3da0` | Keep optional HDR metadata capability distinctions if still absent after consolidation. Do not infer complete mastering/CLL carriage from pixel HDR support. Reapply the actual small delta rather than the old full graph. |
| [157](https://github.com/imazen/zencodec/pull/157), `9efbc24d` | C2PA structural auditing is orthogonal. Retain bounded/opaque semantics; pixel color recognition does not establish privacy or authenticity. |

## Measured migration blocker and gates

An isolated export of PR201 `af9dc2b5` against pixel main `f2d2bfde`, with pixel
main's garb58ce/linear528 sources, passes **workspace all-target checks with std
and with all features**. The standalone renderer passes its all-target check
and **8 integration tests**. No production source was changed for these checks.

PR196 `dc128a4f` was also exported independently and tested with its direct
`zenpsy-api` and registry pixel edge both redirected to the same pixel main
snapshot. Its existing all-feature suite passes **3 integration tests and
1 doctest**, rc0, 4s, peak RSS 0.32 GiB. This qualifies its existing packed bridge,
not a new planar bridge. Other PR heads were reviewed by source; their full
suites were not rerun in this review.

The unchanged workspace std test command stops in
`zencodec-testkit/tests/gray_color_emit.rs::lowered_profile_is_gray_class_and_round_trips_through_the_resolver`:
its expected CICP is `Some(1,16,0,full)`, while the guarded resolver returns
`None`. The ICC remains `KeepSource`. The test compares extraction (`to_cicp`)
to emission permission, which PR201 intentionally separates. Update that
expectation to require qualified substitution, retaining explicit CICP-authority
and truly qualified positive controls. Do not remove the authority guard or
regrant a recognition-table permission to make this test green.

The reusable pixel-owned command is:

```sh
python3 zenpixels/tools/downstream-cohort/check.py ZENCODEC_CHECKOUT FRESH_OUTPUT
```

Run through run-heavy with a timeout. It snapshots clean committed sources,
patches both standalone workspaces, audits a single selected pixel family and
retains commands/locks/logs. It intentionally stops on the first failed gate;
we ran the remaining renderer stages independently after the failure.
Initial cohort: rc1, 58s, peak RSS 0.48 GiB. Remaining all-feature/render stages:
rc0, 14s, peak RSS 0.32 GiB. These are resource receipts, not speed comparisons.
No full test-suite, native codec, platform or release qualification is implied.

## Smallest migration sequence

1. Qualify one dependency-only integration graph against pixel main. Fix the
   demonstrated gray test contract, refresh compiler catalogs, and rerun core,
   render, IQA and native JPEG/PNG/AV1/JXL gates with the original bounds.
2. Finalize current planar context/alpha, checked mapping and lifetime-bound
   lending together, with typed and erased native owners. Validate odd 4:2:0
   crops, P010/12-bit shifts, mixed alpha depth, unknown siting, ICC/HDR anchors,
   late interpretation changes and owner cleanup before freezing signatures.
3. Connect that view to planar IQA/render/encode consumers. Keep unsupported
   layouts explicit; add convenience at the adapter boundary once its semantics
   are proved. Preserve precision evidence only while it remains true.

This targets the hard data contract first. It does not require a new I/O runtime,
a global policy engine in zenpixels, or a second plane representation.

## Design red flags before freezing the pixel API

This follow-up inspects the same pixel main, f2d2bfde. It changes no behavior.

### 1. Planar alpha has a contradictory accepted declaration

`PlanarSlice::new` checks whether an alpha role and `Option<AlphaMode>` are
both present. It does not check `AlphaMode::has_alpha()`. Consequently:

```rust,ignore
// A valid one-pixel Luma + Alpha layout, with independent U8 planes.
let view = PlanarSlice::new(&planes, &layout, 1, 1, &color,
    Some(AlphaMode::Undefined));
assert!(view.is_err()); // FAILS on f2d2bfde: constructor returns Ok.
```

`AlphaMode::Undefined` explicitly means undefined padding (RGBX/BGRX), while
`PlaneRole::Alpha` declares an alpha component. Choose a coherent contract:
reject padding as real alpha, or explicitly model a padding role. Do not
make individual metrics/encoders guess. Straight, premultiplied and opaque
alpha must remain distinct. No downstream wrong-pixel result is claimed here.

### 2. Current color has competing interpretations

This executable witness passes on the current code:

```rust
let context = ColorContext::from_icc(&b"opaque ICC declaration"[..])
    .with_cicp(Cicp::SRGB);
assert!(context.is_srgb());
assert!(matches!(context.as_profile_source(), Some(ColorProfileSource::Icc(_))));
```

The bytes are deliberately opaque; this checks declaration behavior, not ICC
parsing. `is_srgb` consults CICP alone, whereas profile selection prefers ICC.
A non-sRGB authoritative ICC and sRGB companion has the same contradictory
public predicates. Current production source has no call site to this context
predicate beyond its own tests, so this is a demonstrated API inconsistency,
not a reproduced conversion failure. Make current authority unambiguous and
make predicates honor it. A current-context constructor should normalize or
reject conflicting declarations; original dual-carrier claims belong in source
provenance. This does not require merging ColorOrigin with ColorContext.

The two native witnesses above report one failed alpha expectation and one
passing authority-disagreement witness, rc101, 3s, peak RSS 0.23 GiB, using
no-default-features core. These are contract tests, not a performance study.

### 3. Source depth can be mistaken for processing fidelity

`SourcePrecision` describes original source information but also feeds
`ConversionEffect`, is joined during composition, and is mutated by
`after_conversion`. Its docs explicitly retain an eight-bit source depth after
resizing. Averaging adjacent U8 values in F32 creates values that do not lie on
the U8 grid: an original-depth fact cannot establish a preserving U8 exit.

The current module correctly disclaims pixel equality, and
`preserves_modeled_capacity` is an improvement over `is_lossless`. Nevertheless,
a preset named KeepPrecision and fields named bits_kept/bits_dropped remain
easy to overinterpret. Keep immutable origin depth separate from current
processing requirements; invalidate exactness after arithmetic. Do not use
this capacity model to discharge a zencodec sample-preservation requirement.
No failing processing path was executed in this follow-up.

### 4. ICC substitution evidence lacks a caller-visible error contract

`MatrixTrcSubstitution` promises equivalence within an identification tolerance,
but the returned identification does not carry that numerical bound or its
sample domain. Recognition and normalization also use normalized FNV64 keys;
`normalize_known_icc` checks hash and length, without normalized byte equality.
Neither is an exact equivalence proof. PR148's permission revocation is useful,
but does not retroactively qualify every historical table entry.

Keep permissive approximate substitution separate from exact preservation.
For the four canonical profiles whose bytes are already available, compare
normalized bytes before calling a replacement exact. A strict path should
retain the actual profile/use the chosen CMS unless its required guarantee is
established. This recommendation does not require requalifying the entire ICC
corpus before shipping paths that preserve the source profile.

### 5. Chain cost is narrower than operation memory

`ChainPlan::peak_bytes` is the maximum of selected node declarations. That is
valid only for the scope represented by those declarations. It does not account
for overlapping input/output lifetimes, decoder/encoder ownership, worker
multiplicity, queued strips or CMS caches automatically. Keep this expressly
node-local or require offers to cover a common operation scope; zencodec must
not treat this scalar as whole-operation admission. No memory undercount was
measured here.

These are bounded follow-ups: alpha validation, consistent current authority,
precision/evidence semantics, qualified ICC guarantees and memory scope. The
underlying storage/layout/owner split does not need another general hierarchy.
