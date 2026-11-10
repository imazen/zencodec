# Full zenpixels adoption and PR disposition

Observed 2026-11-10 UTC. The pixel foundation is ahead of its consumers. We have
qualified individual paths and review stacks, but do not yet have one adopted,
releasable cohort across the main branches.

This review inventories **63 open PRs in 16 repositories**, refreshes their
heads and ancestry against main, inspects main-root dependency declarations and
the relevant contract/qualification descriptions, and runs the current core
review against current pixel main. It is not a line-by-line audit or fresh test
run of all 63 diffs. The [machine-readable inventory](pixel-adoption-pr-inventory-2026-11-10.json)
retains every head, base, draft state and ancestry result. The comparison
is PR-head → main: main_behind = 0 proves that the PR head is incorporated;
otherwise ancestry alone does not establish equivalent content. PR descriptions'
older test results qualify their named graph only.

## What is adopted, and what is not

| Contract | Pixel implementation | Consumer adoption |
|---|---|---|
| Plane ownership/stride, per-plane encoding and layout | Main | Canonical layout is already in zencodec main. Native wrappers have several unmerged adapters and older pins. |
| Current color authority, gray export, HDR anchors/HLG rendering | Main, with explicit unsupported cases | Core gray/authority work is still in PR199/201. Native PNG/TIFF work is in reviews. Pipeline color selection needs migration. |
| Current planar ColorContext and alpha association | `PlanarSlice` supports them | Zencodec FrameInterpretation still carries only CICP claims, not complete current color/alpha. The checked lending bridge remains proposed. |
| Coupled conversion/output interpretation, fallible worker clone and descriptor errors | Main | Existing OutputPlan-based renderer integrates without runtime source changes in the tested cohort. Other codec/pipeline roots remain pinned behind it. |
| Native 10/12-bit payloads and canonical U16 conversion | Main for supported U8/U16 readers/arithmetic | Real codec lowering and precision decisions are not uniformly adopted. Signed/wide descriptions do not implement signed/wide numerical readers. |
| Provenance/capacity versus verified current exactness | Separate pixel APIs, clearer documentation | A real resize/filter/composite consumer still must invalidate/refresh evidence. Zenpipe PR121 is not a qualified full-pipeline solution. |
| CMS backends | Explicit Mox and Rust LCMS in pixel main | Separate af19 native CMYK LCMS gate passes; Mox fails the retained reference bound. Current-main native qualification is still needed. |
| Fallible image allocation | Main hardening plus open PR153 | Not a universal fallibility guarantee. Planner/control/cache/backend exceptions remain documented. Consuming typed extraction still needs the new fallible sibling adopted. |
| Hashing current samples/interpretation | PR149/152 merged, optional feature | Useful for explicit cache/evidence use; not required in every decoder, and not a new preservation proof. |
| Seek/replay, cancellation, publication and output backpressure | Codec/driver responsibility | Pixel changes do not replace the existing sans-I/O/Pulse state machines or prove whole-operation memory limits. |

## Current main dependency split

These are root declarations read from GitHub, not graphs resolved under local
workspace overrides. A wide semver range does not undo an explicit old patch.
A manifest source is also not proof that its whole dependency graph is unified.

| Main consumer | Selected pixel source | Selected core source / qualification concern |
|---|---|---|
| zencodec `1fb997bb` | `fde59b96` | Main canonical plane API; current pixel changes not adopted. |
| JPEG `a8e92e81` | `fa836abf` | Core family `1816b4d0`; sans-I/O/stable reviews remain separate. |
| PNG `a26cdc64` | `fa836abf` | Core `1816b4d0`; qualified companion stack is not main. |
| UltraHDR `aadf1021` | `fa836abf` | Core `1816b4d0`; full std and no_std scopes differ. |
| AVIF/MP4 `69914c73` | `fa836abf` | Core `1816b4d0`; optional media also has direct git edges. |
| zenpipe `0b3274f8` | `f23ac753` | Core `3f4fd14f`; many concrete-codec pins must move as one graph. |
| zenjxl `cc2ec685` | `f23ac753` | Root additionally expects a `.zencodec` path bridge; portable graph must be qualified. |
| JXL encoder `d1a6a14e` | `f23ac753` | Root has sibling zencodec/zenmeta path patches; metric adapters have separate review pins. |
| HEIC `4d72cadd` | `f23ac753` | Core `a51d4119`; native plane/Pulse proposals remain open. |
| WebP `50c378d8` | `fa836abf` | Core `1816b4d0`; YUV plane adapter is a draft. |
| zenextras `86d1d52d` | `283655c5` | Core `f02a17ec`; image and container review graphs differ. |
| RAW `9b3fb11f` | `283655c5` | Core `feb09f9f`; no open PR does not mean current adoption. |
| bitmaps `4330dbaa` | `283655c5` | Core `feb09f9f`; canonical deep PNM change is PR36. |
| GIF `0b2a001e` | `789d69db` | Core `e59b3803`; needs a fresh paired graph despite no open PR. |

The decoder-only JXL root was inspected separately; it does not declare a
zenpixels edge in its root manifest. Its wrapper/output adapters, not every
native decoding kernel, need to speak the interchange API.

Even within this zencodec review, the root selects pixel b6b12bbe, the standalone
renderer and spatial harness select fde59b96, and the explicit LCMS harness
selects af19bcda. Those are intentional retained controls, not full adoption.
Do not silently erase their evidence while introducing the current cohort.

## Pixel PRs and the five reported risks

Current pixel main is `124c5dc44c72f6323ceca470a5dd968b74c98f12`.

- PRs 120/121/122 and 144–148 are merged: precision/HDR intent, canonical planes,
  stable dependencies, lending and conversion/authority corrections.
- PR149 adds optional hashing; PR152 and its main descendants address alpha
  declarations, current-authority predicates, paired ICC fingerprints and
  canonical normalized-byte verification. The two earlier executable contract
  findings are fixed in source. Old f2d2bfde witnesses remain historical evidence.
- Main's `ec0d8ab0` hardens several fallible allocation paths. **Open PR153**
  (`165b49f8`) adds fallible contiguous typed extraction and further allocation
  changes. Review/qualify it atop current main before consumers rely on it.
- Intermediate PRs 123–131 are closed historical proposals. Their final content
  was consolidated; do not queue them as additional merges.

[Issue150](https://github.com/imazen/zenpixels/issues/150) is partly addressed,
not wholly closed by the fixes: current processing precision requirements need
consumer integration; approximate ICC qualification remains distinct from exact
preservation; ChainPlan now explicitly states its limited cost scope.
[Issue151](https://github.com/imazen/zenpixels/issues/151) has an audit and initial
implementation, not an all-allocation/no-abort guarantee. Named exceptions include
trace/control allocation, lazy caches and caller-count-sized planner vectors.

The current re-audit also retains the truncated-ICC grayscale authority exception
and stale historical release-branch fallback exposure. See pixel
[issue re-audit](https://github.com/imazen/zenpixels/blob/124c5dc44c72f6323ceca470a5dd968b74c98f12/docs/issue-reaudit-2026-11-09.md)
and [allocation scope](https://github.com/imazen/zenpixels/blob/124c5dc44c72f6323ceca470a5dd968b74c98f12/docs/allocation-audit.md).

## Every open PR: recommended disposition

“Keep separate” means it has work worth reviewing, not that it should be dropped.
“Rebase/qualify” is a recommendation, not an action performed by this review.
The inventory contains exact heads for all entries below.

### zencodec: 8

| PR | Disposition |
|---|---|
| [199](https://github.com/imazen/zencodec/pull/199) gray carriers/source depth | Integrate its functional delta with the HLG correction and authority guard once, on one current pixel cohort. Do not land the older ICC-derived-CICP rule independently. |
| [201](https://github.com/imazen/zencodec/pull/201) ICC authority/native controls | This stack already contains the gray/HLG work. Keep the guard and native evidence; fix the measured gray test contract, update the cohort and requalify the combined branch. |
| [200](https://github.com/imazen/zencodec/pull/200) sans-I/O synchronization | Net change against main is docs/recipes/native probe, not another runtime. Consolidate the useful history/probe; do not restart the state-machine design. |
| [196](https://github.com/imazen/zencodec/pull/196) IQA bridge | Retain optional typed/erased packed bridge. Prior pixel-main tests pass. Planar integration depends on the honest checked frame-lending boundary. |
| [189](https://github.com/imazen/zencodec/pull/189) I/O publication | Reapply only the publication-manifest delta after package checks. The old branch also carries older requirements/bench declarations and must not replace the current manifest wholesale. |
| [188](https://github.com/imazen/zencodec/pull/188) exact timelines | Keep coordinated with AVIF79 and extras56. Edited-video interpretation is not solved by plane interchange. Decide its public contract while media is still unpublished; qualify native execution separately. |
| [158](https://github.com/imazen/zencodec/pull/158) optional HDR capabilities | Flags are not present in current EncodeCapabilities. Review the small capability/conformance delta against actual checked metadata admission; rebase rather than transplanting its old graph. |
| [157](https://github.com/imazen/zencodec/pull/157) C2PA audit | Independent bounded metadata work. Rebase/review separately; not a prerequisite for pixel adoption or a signing/privacy guarantee. |

### JPEG: 10

| PRs | Disposition |
|---|---|
| [258](https://github.com/imazen/zenjpeg/pull/258), [260](https://github.com/imazen/zenjpeg/pull/260) | Recovered native sans-I/O plus stable-cohort descendant. Preserve the extensive named-graph runtime/oracle receipts, update the common pins once, and carry current main's encoder fixes with it. Neither is main. |
| [259](https://github.com/imazen/zenjpeg/pull/259) | Depth/current-pixel ICC migration. Reconcile overlap with 258/260; preserve explicit depth semantics and actual current context. |
| [255](https://github.com/imazen/zenjpeg/pull/255), [257](https://github.com/imazen/zenjpeg/pull/257) | Plane borrowing and strided API changes. Useful consumer validation for the canonical plane contract; refresh against current layout/encoding and retain tight/strided parity. |
| [262](https://github.com/imazen/zenjpeg/pull/262) | Independent encoder edge-replication correctness fix, with failing-first/independent-reader evidence in its receipt. It must not be lost when landing an older sans-I/O branch. Requested no-merge review remains respected. |
| [261](https://github.com/imazen/zenjpeg/pull/261), [253](https://github.com/imazen/zenjpeg/pull/253), [191](https://github.com/imazen/zenjpeg/pull/191) | Restoration/parameter-record work; preserve separately from the foundational pixel rollout. No need to enable it to qualify the common codec contract. |
| [256](https://github.com/imazen/zenjpeg/pull/256) | Independent IDCT choice; retain its numerical contract, not a pixel-adoption dependency. |

### PNG: 6

| PRs | Disposition |
|---|---|
| [55](https://github.com/imazen/zenpng/pull/55) → [56](https://github.com/imazen/zenpng/pull/56) → [59](https://github.com/imazen/zenpng/pull/59) → [60](https://github.com/imazen/zenpng/pull/60) | One accumulated facade/stable/native-carrier stack, not four independent final implementations. PR60 has qualified native/platform receipts on core db6/pixel3ceb. Update/qualify the final stack against the chosen current cohort; do not mix that old receipt with new pins. |
| [54](https://github.com/imazen/zenpng/pull/54), [57](https://github.com/imazen/zenpng/pull/57) | Earlier gray/depth/cohort candidates. Compare their residual changes with the final carrier stack; supersede only after confirming coverage. Their old beta constraints are not a current design requirement. |

### UltraHDR: 3

| PRs | Disposition |
|---|---|
| [43](https://github.com/imazen/ultrahdr/pull/43) → [45](https://github.com/imazen/ultrahdr/pull/45) | One source/stable stack with qualified std numerical controls. Its JPEG/linear graph differs from JPEG260's graph. Preserve that distinction and qualify the selected common graph. Genuine no_std/WASM had 43 errors; no current success is inferred. |
| [44](https://github.com/imazen/ultrahdr/pull/44) | Depth-consumer cohort; compare the actual residual before folding it into the selected stack. |

### zenpipe/zencodecs: 11

| PRs | Disposition |
|---|---|
| [84](https://github.com/imazen/zenpipe/pull/84), [86](https://github.com/imazen/zenpipe/pull/86), [87](https://github.com/imazen/zenpipe/pull/87), [105](https://github.com/imazen/zenpipe/pull/105) | **Heads are already ancestors of main**, independently checked by GitHub comparison. Recommend closing as incorporated after recording that evidence; no second merge is needed. This does not prove their current behavior. |
| [106](https://github.com/imazen/zenpipe/pull/106) | Diverged Pulse follow-up. Compare its one remaining commit with current optional-control code before deciding whether any delta remains. |
| [109](https://github.com/imazen/zenpipe/pull/109), [110](https://github.com/imazen/zenpipe/pull/110) | Old AVIF backend/Pulse/lossless routing stack. Preserve useful routing settings while migrating the current 0.2 family; do not restore its old dependency family. |
| [116](https://github.com/imazen/zenpipe/pull/116) | Reconcile matte behavior with the converter's Color-based, linear-light policy. Retain explicit encoded/browser-matching mode and independent pixel controls. |
| [100](https://github.com/imazen/zenpipe/pull/100), [120](https://github.com/imazen/zenpipe/pull/120) | Reconcile as one HDR policy, not two tone-mapping implementations. Main pixels now owns explicit HLG rendering and BT.2446 support; avoid keeping a local fixed-display HLG workaround. Measured-peak buffering and declared-peak row streaming need truthful scheduling. |
| [121](https://github.com/imazen/zenpipe/pull/121) | Partial precision-consumer prototype. Its body explicitly reports patched/partial verification and uncompiled bridge/filter paths. Port to current modeled-capacity/evidence contracts and test real decode→resize/filter/composite→encode. It is not ready from a preset name alone. |

The source of `zencodecs::cms::srgb_transform_icc` still reconstructs authority
and can fall back to an ICC when selected CICP synthesis is unavailable. Port
this to actual current context/explicit authority and typed refusal or policy;
do not treat inability to synthesize as identity. This is source review, not a
new executed wrong-pixel witness. The older related finding is tracked by
[zenpipe115](https://github.com/imazen/zenpipe/issues/115).

### AVIF/MP4: 5

| PRs | Disposition |
|---|---|
| [80](https://github.com/imazen/zenavif/pull/80), [81](https://github.com/imazen/zenavif/pull/81) | Native YUV borrowing and strided SVT input: core interchange consumers. Keep low-bit alignment, unknown siting and real coded geometry; qualify mapped-owner lifetimes and padded output parity. |
| [79](https://github.com/imazen/zenavif/pull/79) | Complete edits paired with core188/extras56. Requires shared timeline decision and independent playback/execution qualification. |
| [83](https://github.com/imazen/zenavif/pull/83) | Explicit ready-fragment publication; keep additive boundary operation and original packet/duration/cancellation gates. Pixel adoption does not establish latency or queue bounds. |
| [76](https://github.com/imazen/zenavif/pull/76) | SVT 4:4:4/lossless routing; preserve declared unsupported envelope and independent lossless controls. Do not claim all 10-bit or animation cases from its qualified U8 still subset. |

### zenextras: 4

| PRs | Disposition |
|---|---|
| [57](https://github.com/imazen/zenextras/pull/57) | Native gray TIFF ICC/context. Useful independently checked source; full root was not qualified. Update with PNG/core gray semantics on one coherent family. |
| [55](https://github.com/imazen/zenextras/pull/55) | Optional Pulse base for containers; portable graph/fuzz/API qualification remains separate from earlier isolated tests. |
| [56](https://github.com/imazen/zenextras/pull/56) | Explicit refusal of unsupported edit programs, paired with core188. Refusal is necessary even where positive exact preservation is unsupported. |
| [58](https://github.com/imazen/zenextras/pull/58) | Explicit cluster publication, atop55. Keep prefix demux and cancellation checks; not a guarantee of random access or bounded queued output. |

### JPEG XL: 10 across wrapper, decoder and encoder

| PRs | Disposition |
|---|---|
| wrapper [20](https://github.com/imazen/zenjxl/pull/20), [30](https://github.com/imazen/zenjxl/pull/30) | Preserve animation/timing/current-color and complete metadata behavior while rebasing on current native sources. Qualify completeness at real output boundaries, not just probe metadata. |
| decoder [75](https://github.com/imazen/zenjxl-decoder/pull/75) | Important collection/raw-layer consumer, including CMYK and extras. Its own receipt leaves negative-origin and broader blend/seek/progressive cases open. Do not render/crop away those distinctions in native interchange. |
| decoder [66](https://github.com/imazen/zenjxl-decoder/pull/66) | Layout/seek offsets and bounded recording. Relevant to positional/collection exploration but independent of packed conversion. Preserve exact coordinate domains and opt-in cost. |
| decoder [77](https://github.com/imazen/zenjxl-decoder/pull/77), [61](https://github.com/imazen/zenjxl-decoder/pull/61) | SIMD/fast-path work; do not make these implementation changes prerequisites for selecting current pixel types. |
| encoder [143](https://github.com/imazen/jxl-encoder/pull/143) | Real PlanarSlice/metric-lending consumer. It supplies useful lifetime/stride/stop evidence, but its SDR/BT.709/80-nit/map semantics are intentionally narrow. It is not universal HDR/ICC metric qualification. |
| encoder [140](https://github.com/imazen/jxl-encoder/pull/140) | Strided metric ingestion, with separate metric graph. Retain no-extra-input-copy behavior and actual scorer qualification. |
| encoder [141](https://github.com/imazen/jxl-encoder/pull/141), [132](https://github.com/imazen/jxl-encoder/pull/132) | SIMD and encoder tuning; preserve separately from the foundational API migration. |

### HEIC/WebP/bitmaps: 5; RAW/GIF: no open PRs

| PRs | Disposition |
|---|---|
| HEIC [64](https://github.com/imazen/heic/pull/64), [65](https://github.com/imazen/heic/pull/65) | Pulse plus native coded-plane borrowing. Unify the core/pixel family, retain grid progress, original stop causes and crop/siting distinctions. Full root/platform/oracle qualification is not established by focused tests. |
| HEIC [67](https://github.com/imazen/heic/pull/67) | Independent depth/deblocking correctness work; keep its evidence while rebasing consumers. |
| WebP [120](https://github.com/imazen/zenwebp/pull/120) | Borrow native VP8 YUV with unknown siting and no fabricated color facts. Its old plane/core pins and nested-workspace gaps require a current cohort. |
| bitmaps [36](https://github.com/imazen/zenbitmaps/pull/36) | Essential deep-PNM contract fix: canonical full-range U16 plus measured current exactness. Qualified on its named graph; adopt/qualify on the common graph rather than retaining raw MAXVAL codes as normalized samples. |
| RAW / GIF | No open review does not qualify their old selected pixel families. Include both in cohort/type-identity, cancellation, metadata and real-codec checks. |

## Fresh checks in this review

Current core review `9058bfc6` was exported against pixel main `124c5dc4`, with
that main's garb58ce/linear528 patches. Single-family metadata admission passes;
workspace std all-target compilation passes. The std test gate stops at the
same known gray round-trip expectation: extracted CICP is compared with emission
permission, but the authoritative MetadataOnly ICC must remain retained without
replacement CICP. No test assertion or production code was changed here.

The run returns rc1, 60s, peak RSS 0.49 GiB. Remaining all-feature core all-target checking, standalone renderer all-target
checking and its eight integration tests pass independently (rc0, 14s, peak RSS
0.33 GiB). Earlier native PNG/JPEG/AV1/JXL and af19 CMYK evidence
remains pinned to its original source, not relabeled as current-main validation.

## Integration order and acceptance

1. **Select one current pixel/core integration source.** Combine core199's
   gray/depth delta, HLG native-carrier correction and core201 guard without
   duplicate migrations. Migrate the gray test's permission contract, update
   locks/catalogs, and qualify main plus the pending allocation delta separately.
2. **Finish the hard frame contract before freezing it.** Retain current planar
   ColorContext and alpha, require honest encoding evidence, share validation
   between direct Frame and ImageOutput, and lend checked PlanarSlice under the
   mapping lifetime. Exercise mixed depth, odd crops, P010, ICC/HDR, unknown facts,
   alpha, typed/erased owners and late interpretation changes.
3. **Move the codec cohort together.** JPEG/PNG/UltraHDR first because their
   git edges cascade; then AVIF/JXL/HEIC/WebP/TIFF and bitmap/GIF/RAW. Audit root,
   fuzz, examples, native harnesses and standalone workspaces independently.
   Dependency-level patches do not propagate to consumers. Retain frozen failure
   witnesses in separately named control graphs.
4. **Validate actual processing through zenpipe.** Current-value precision and
   alpha after resize/filter/composition, explicit HLG/white/peak policy, late
   metadata, cancellation, seek/replay, low-memory versus buffering schedules,
   and output publication must work through real codecs. A compile-only patch
   or automatic-narrowing resolver is not full adoption.
5. **Clean the PR queue and release artifacts.** Close the four proven-incorporated
   zenpipe heads, consolidate overlapping review stacks after coverage checks,
   refresh generated docs/API snapshots, then package/semver/platform gates and
   the owner's README/release approval. No merge or publication occurred here.

IQA refinement, restoration, SIMD experiments and optional hash adoption need
not delay the common codec API. They must keep explicit supported scopes. This
review does not authorize dropping the requested frame/video contracts to make
the release appear complete.

## Core landings before the codec rollout

The next core integration should be a small ordered series. Existing main
already supplies sans-I/O, optional Pulse, retained encoder planning, exact-depth
assertions and operation-policy binding. Do not start another implementation of
those mechanisms to adopt pixels.

### 1. Current pixel cohort plus the existing color review

Update the core integration root and standalone workspaces to one selected
current pixel source, including the now-merged PR153 allocation changes
(pixel main76005fc9 at verification). Combine the useful PR199/HLG/PR201 changes
once: gray-class ICC, native HLG CICP without targetless ICC synthesis, qualified
ICC substitution and source-depth selection. Migrate the gray roundtrip test
from extraction to permission without weakening its source-ICC retention.

This is existing implementation needing integration, not a new public model.
Keep named frozen numerical failures as separate controls. Passing core and
render tests must precede moving the main bookmark.

### 2. Complete frame interpretation and mapping together

This is the main new signature/data review, continuing issue198:

- Retain the full current ColorContext and explicit alpha association with each
  immutable frame interpretation; source carrier claims stay provenance.
- Require native mapped owners to state encoding. A missing declaration must
  not silently inherit the layout as though independently confirmed.
- Use shared layout/type/geometry/encoding checks for direct Frame consumers
  and ImageOutput consumers, preserving original typed errors and traces.
- Provide scoped checked PlanarSlice lending under the native mapping guard.
  Never return a view borrowing a locally assembled handle list.
- Preserve coded crop, odd chroma phase, render geometry, timing and epoch.
  Native interchange does not silently crop oversized layers into a canvas;
  explicit rendering supplies the display canvas.
- Make Samples/Row extensible and keep pixel types owned by zenpixels. Keep
  arbitrary auxiliary/CMYK/spot/depth meaning separate from RGB/YUV layout.

Prove the shape first with shared owned planes plus representative native video
and collection adapters, both typed and erased. Those small validation adapters
are necessary before broad delegation; all codec implementations need not be
completed before this core contract lands.

### 3. Freeze evidence and late-decision behavior on existing APIs

Reuse EncodeDeclaration.exact_depth, retained plans and OperationPolicy. Make
these obligations executable in shared controls:

- Origin bit depth and modeled capacity never prove lossless processing.
  Current exactness ends after resize/filter/CMS/composition unless verified or
  preserved by the specific operation. Canonical U16 differs from native shifted
  10/12-bit codes; no relabeling or false exactness certificate.
- Encoder planning binds actual current color and carrier obligations before
  native begin. Descriptive metadata cannot overwrite it.
- Late color/metadata decisions use the same callback at real suspension points
  before affected effects. Requirements filter choices; ordinary callers get
  supported best-effort routes when they did not require strict preservation.
- Unknown workspace stays unknown. Driver/node estimates cannot become an
  operation bound through naming or a preset. Keep input acquisition/replay
  separate from output timing and backpressure.
- Required metadata must either be retained with evidence or produce a named
  typed refusal. Optional HDR capability flags from PR158, if adopted, describe
  complete channels and remain advisory; callers must not coordinate queries
  before every operation.

Much of this exists already. Land concrete gaps and shared tests, not a new
precision history engine, policy hierarchy or mandatory capability handshake.

### 4. Land the media presentation declaration delta

Rebase PR188's TrackTimeline/EditList/exact-presentation admission onto the
current frame/core contract. Preserve Unknown/Unedited/Edited, ordered segments,
clock/rate/extents and refusal of conflicting legacy/full declarations. Deprecate
scalar-only projections for exact presentation. Mux defaults must refuse exact
preservation they cannot execute before publishing output.

The core declaration/default-refusal contract should precede the muxer/decoder
migration. Positive playback correctness remains native qualification; retained
timestamps or edit records alone must not be advertised as that proof. Keep
unpublished media outside the first package batch if its execution gates remain
open, without pretending the video contract is solved.

### 5. Ship the migration target and its reusable acceptance suite

Land testkit controls with the contracts: padded/minimal-final-row/mixed-depth
planes, alpha, odd crops, current versus source color, owner lifetime/cleanup,
late metadata, strict refusal and ordinary successful fallback, typed/erased
error parity, and cancellation through optional Pulse. A codec migration should
implement one documented signature set and run this same suite, adding its own
independent reader/native precision cases.

Update spec, compiler API catalog, code-first guide, conveniences and deprecation
map in the same series. A minimal consumer must not assemble a matrix of
capability queries. PR200's useful docs/probe can be folded here; PR196's optional
IQA bridge can follow the checked view. PR189 publication preparation and PR157
C2PA are not reasons to enlarge this contract series. Publication still requires
its separate approval and package/platform gates.

Exit criterion: one main revision exposes the reviewed interchange/policy/time
contract, passes the shared gates and representative native witnesses, and gives
every codec lane the same imports, signatures and expected outcomes. Broader
codec/backend coverage, tuning, restoration and metric loops follow that target.
