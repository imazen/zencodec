# zencodec main versus crates.io: contract audit

**Recommendation:** keep the metadata extraction compatible with 0.1.26. Do not
spend a breaking release on moving modules or adding inventory. Reserve the next
break for enforceable operation requirements, independent parsing/extraction
controls, and an unambiguous contract between output pixels and their metadata.
Resolve the new dispatch/rewrite ambiguities before publishing them.

This records the source/API audit at the baseline below. Later implementation
separates ordinary and exact timing and moves rewrite execution to zenmeta.
The user has approved completing the boundary in the next coordinated breaking
release; the current target and acceptance gates are in
[zenmeta design](zenmeta-design.md#next-breaking-release-target) and the
[changelog queue](../CHANGELOG.md). Historical findings below describe the audited
commit, not a claim that those issues remain in every later revision. No release
is being made by this audit.

## Baseline and reproducible diff

Observed 2026-10-13 UTC (2026-10-12 in America/Denver):

| Item | Verified value |
|---|---|
| Latest non-yanked registry release | [zencodec 0.1.26](https://crates.io/crates/zencodec/0.1.26), published 2026-07-14 |
| Released source | [998edf5f5152f675bbe3b4e959470dffe1536c36](https://github.com/imazen/zencodec/tree/998edf5f5152f675bbe3b4e959470dffe1536c36), from the archive's `.cargo_vcs_info.json` |
| Audited main | [717a55d795a58d90fd0ebf2a3ef504a7f200ade7](https://github.com/imazen/zencodec/tree/717a55d795a58d90fd0ebf2a3ef504a7f200ade7), manifest version 0.1.27 |
| Archive verification | Every published `src/` file and `Cargo.toml.orig` matches the declared release commit byte for byte; source file sets also match |
| Signature check, all features | cargo-semver-checks 0.50.0: 196 passed, 58 skipped; no semver update required |
| Signature check, no features | Same result, checked separately with `--only-explicit-features` |
| Features | `std` is the only zencodec feature; default remains `no_std + alloc` |
| Normal dependency delta | Only `zenpixels`: `0.2.14` becomes `>=0.2.14, <0.4.0` |

The registry was queried at
[its API endpoint](https://crates.io/api/v1/crates/zencodec), and the source was
obtained with `cargo read zencodec==0.1.26 --path-only`. The
[released API documentation](https://docs.rs/zencodec/0.1.26/zencodec/) is a useful
companion, but the archive is the comparison source.

The complete source/manifest diff is generated, not abridged to metadata:

```sh
just contract-diff "$HOME/.cache/cargo-read/zencodec-0.1.26" \
  /mnt/v/output/zencodec/contract-audit-2026-10-13 717a55d7
just metadata-semver
just contract-audit-check
```

Apply the workspace's build resource wrapper to Cargo recipes. The diff is
`/mnt/v/output/zencodec/contract-audit-2026-10-13/source.diff`, with a
`provenance.json` beside it. It is 209117 bytes; SHA-256:
`b1b744d81bd91d98fb2a6f2671437359e620ec7bbad1349d988bd32348794dbd`.
The [generator](../scripts/audit-published-contract.py) checks the archive's VCS
provenance before comparing it with the pinned main. Generated diff data is kept
outside git; the recipe regenerates it from the named inputs.

The checks establish source compatibility within their supported lints and
feature selections. They do not certify behavior, downstream method-name
ambiguities, every target, clean package publication, or compatibility with a
future zenpixels 0.3 release. No build-time, binary-size, or codec-wide performance
comparison was measured here. This audit did not rerun every consumer codec.

## What actually changed

All 22 changed source/manifest paths are covered below. Inline tests and comments
are included in the raw diff; additions are not all executable library code.

| Area and files | Main versus 0.1.26 | Compatibility assessment |
|---|---|---|
| `Cargo.toml` | Version 0.1.27; zenmeta workspace member; broader zenpixels range | No zenmeta normal dependency. A wider range does not prove future dependency compatibility |
| `animation.rs`, `output.rs`, `traits/encoder.rs`, `traits/decoder.rs`, `traits/dyn_encoding.rs` | Exact rational `FrameDuration`, `TimingError`, frame constructors/accessors and provided `push_frame_timed`; owned frames preserve exact duration | Additive. Legacy millisecond accessors still truncate/saturate; exact callers must use the new path. Default exact encoding rejects rather than rounds |
| `inventory.rs`, `traits/decoding.rs`, `traits/dyn_decoding.rs` | Structural parts, ranges, dispositions, coverage validation, and inventory operations | Entirely unpublished. Generic result is `Result<Inventory, At<CodecError>>`; erased result uses `BoxedError`. Disabled implementations return typed unsupported |
| `metadata_rewrite.rs` | Request, services, report entries, borrowed output plan, compression/action/error types | Entirely unpublished. Container implementation remains codec-owned; XML/compression services are injected |
| `exif.rs`, `exif/privacy_tests.rs` | `Ifd`, `EntryRef`, allocation-free `entries()` iteration; color, UTC-offset, device-ID, camera-owner and image-ID categories, queries and policy builders | Additive surface, intentional changes to existing filtering semantics |
| `metadata.rs` | Publishing presets retain validated EXIF color; filtering retains diffuse-white HDR anchor; anchor-only metadata is nonempty | Behavioral change despite compatible signatures |
| `set.rs` | `CodecSet`, `CodecSetError`; registration/detection, probing, decode modes, encode jobs, transcode and estimate forwarding | Entirely unpublished dispatch implementation, not necessary for using any metadata API |
| `traits/decoding.rs`, `traits/encoding.rs`, `traits/dyn_encoding.rs` | Provided config/job one-shot decode/probe/encode conveniences | Additive defaults; direct codec use remains possible |
| `capabilities.rs`, `limits.rs` | Inventory capability; appended `Inventory`, `AnimationTiming`, `InventoryParts` variants; gain-map contract documentation | Released discriminants retained. Exact-timing capability reporting remains incomplete |
| `estimate.rs` | `ComputeEnvironment::conservative()`, `host()` under std; `new()` deprecated | Existing default behavior retained; deprecation can affect callers denying warnings |
| `lib.rs` | Module exports, CodecSet exports, generic/dynamic trait prelude | Additive; broad trait imports can create downstream name-resolution collisions |
| `format/builtins.rs` | Checked TIFF/DNG offset arithmetic | Implementation correctness fix, especially relevant on 32-bit |
| `info.rs` | More precise per-format progressive-image documentation | No field/signature change. A contract clarification still warrants consumer agreement checks |
| `color.rs` | Corrected documentation of per-codec lowering and caller-owned pixel/metadata consistency | Resolver implementation is unchanged; the absence of an enforced consistency boundary is inherited |

**Already published, not introduced by zenmeta:** the EXIF parser/filter, metadata
policies, ICC/CICP and orientation helpers, ISO 21496-1 gain-map types/parsers,
color-emission planning, resource/error models and static codec capabilities.
`policy.rs`, `gainmap.rs`, `fidelity.rs`, and the central error model did not change
in this comparison. Problems in those APIs are inherited debt, not regressions
caused by the metadata extraction.

Behavior changes to call out in release notes even with green semver checks:

- Keeping camera descriptions or timestamps no longer implicitly retains their
  separately classified identifiers/ownership or UTC offsets.
- EXIF tree rewrites drop opaque MakerNotes and unvalidated Interop entries.
  `exif::retain(..., KEEP_ALL)` is the byte-preservation path; reconstructing an
  EXIF tree is not equivalent to preserving the original blob.
- The HDR diffuse-white anchor now survives retaining policies.
- Loop-count documentation explicitly requires total plays, including the first,
  and rejection of unrepresentable counts. This does not prove every adapter
  implements the conversion. Exact durations do not solve loop-count drift.
- `PreserveExact` has existing delivery exceptions, including orientation
  reconciliation and unsupported carriers. It is not whole-file preservation.

## Adversarial findings

### 1. A requirement can currently become a silent hint

Both [decode policy](../src/traits/decoding.rs) and
[encode policy](../src/traits/encoding.rs) have no-op provided setters.
`with_fidelity` explicitly permits silent substitution; its default lossless
branch calls a setter that can also do nothing. `resolved_target_fidelity` is an
optional config report, not an encode-time proof about the actual samples.

Adversarial case: request mathematically lossless encoding through an encoder
that does not implement lossless control. The common trait permits the request
to be ignored. The newer `CodecSet::encode_with` and `transcode` forward that
request without requiring confirmation. This is a contract-level observation,
not a claim that all current encoders ignore it.

**Propose:** distinguish optional tuning hints from required output properties.
At executor construction, reject requirements that cannot be honored; finish
content-dependent validation when the pixels become available. Lossless palette
encoding, alpha preservation, HDR, unsupported metadata carriers and exact timing
need real outcomes, not stronger adjectives on capability booleans. Require the
same behavior through generic, erased, one-shot, row and animation paths.

**YAGNI check:** do not resurrect an elaborate config-time `FidelityMatch` state
machine. The source already documents why it cannot settle content-dependent
losslessness. Add a narrow required-operation path first; replacing the old
best-effort guarantee is what deserves the breaking queue.

### 2. Parsing strictness, extraction, and rendering interpretation are mixed

[DecodePolicy](../src/policy.rs) combines strict parsing, truncated-input
acceptance, progressive/animation admission and returned ICC/EXIF/XMP. Its
`strict()` preset also disables progressive images and metadata. Denying ICC
promises skipped parsing and no returned profile. HDR reconstruction and correct
orientation can still depend on information in the denied carrier.

The [metadata integration findings](zenmeta-migration.md#findings-from-the-completed-jpeg-adapter-checks-2026-10-13)
record actual JPEG disagreements: extraction denial was overridden, ordinary XMP
was mistaken for a broken gain map, and truncation control selected broader
strictness. Those integration results are prior evidence; the policy definitions
were re-read for this audit.

**Propose three independent decisions:** bounded compatible versus strict
interpretation; which metadata is returned; and which operations/content the
caller admits. Rendering-critical interpretation must either remain available
or cause an explicit rejection. It must never disappear while pixels are still
labeled as though it were applied. Keep compatible real-world decoding as the
ordinary path; strict inspection and safe rewriting can impose different gates.

**YAGNI check:** not one strictness flag per vendor defect. Share named parser
modes and typed diagnostics. Add a new policy path compatibly, then retire the
ambiguous old fields/preset in the breaking release. Do not silently reinterpret
existing `allow_*` calls in a patch.

### 3. Source metadata and the meaning of current pixels need distinct contracts

[SourceColor](../src/info.rs) describes source signals;
[DecodeOutput](../src/output.rs) also carries pixels with a descriptor and color
context. `DecodeOutput::metadata()` returns metadata derived from `ImageInfo`.
[Color planning](../src/color.rs) accepts a separate source description and cannot
verify that it describes the submitted pixels. This is particularly important
after orientation, HDR reconstruction or color conversion. Source provenance
should remain available even when it legitimately differs from output color.

The executable probe also confirms that Display-P3 into a target with neither
ICC nor CICP yields `SynthesizeFrom(P3)`, despite `target.icc() == false`.
This is an intent plan, not proof the target can carry the required description.
A codec can correctly reject it later; the common resolver itself cannot.

**Propose:** one authoritative description of the current output pixels at the
encode boundary, plus separately retained source facts and descriptive metadata.
A checked finalization step must either produce realizable color/orientation
signaling or return an error/conversion requirement. It must not perform hidden
CMS conversion. Preserve the codec's ownership of actual container emission.

**Breaking portion:** retiring independently authoritative legacy inputs or
changing what `metadata()` promises. An additive checked path can come first.
Avoid banning valid source/output differences with a naive equality assertion.
Do not force a new pixel wrapper hierarchy if existing zenpixels context can
carry the required facts.

### 4. Capability booleans describe a codec type, not a configured operation

`EncoderConfig::capabilities()` and `supported_descriptors()` are static;
[the decode equivalents](../src/traits/decoding.rs) are also static. They cannot
express backend, fidelity, source bit depth, frame topology and limits together.
The gain-map boolean's strengthened documentation expects both map directions
and 1/3 channels: that is a substantial promise for one bit.

One concrete inconsistency: `EncodeCapabilities::supports(AnimationTiming)`
always returns false; no builder can make it true. Exact-timing implementation
and discovery can therefore disagree. This is new, unpublished surface and
should be repaired before release, not deferred to a breaking version.

**Propose:** keep static summaries for discovery, add effective config/job
queries and validation for the actual operation. Unknown is not unsupported,
and recognition of a file feature is not support for rendering it. Only removal
or changed guarantees of the old static methods need a breaking release.

**YAGNI check:** no cartesian-product capability matrix, backend enum for every
library, or global registry of all codec options. A query tied to an actual
request is smaller and more truthful. Specialized settings stay on native APIs.

### 5. CodecSet adds orchestration decisions to the contract crate

[Registration](../src/set.rs) is first-wins for duplicate formats. Two AVIF
encoders with different capabilities cannot be selected independently through a
single format-keyed encode call. Registering both does not imply fallback or
capability-based selection. Direct codec configuration remains an escape hatch.

`transcode` decodes a single image, may decode again to negotiate a descriptor,
then forwards source metadata. It neither iterates animations/pages nor extracts
and forwards gain-map pixels. It does not require the resolved fidelity to match
the request. The descriptor intersection is necessary, not sufficient to prove
rendering preservation. Its documentation states the single-image scope, but
an innocently named convenience can still make lossy decisions surprising.

**Before publishing:** explicitly limit transcode to a selected still image and
make loss of animation, auxiliary rendering data and precision an explicit
request or an error. Offer explicit backend selection if the multi-backend
consumer needs it; a small caller-selected key or separate configured sets may
suffice. Do not implement automatic retry across encoders without specified
failure, cost and output semantics.

**YAGNI verdict:** the registry is useful optional convenience, not part of the
metadata foundation. Keep it free of pipeline processing, CMS, plugin discovery
and a second format registry. Moving it to another crate is not justified by a
measured cost here. Its estimate forwarding and one-shot wrappers are lower
priority than getting these semantics right.

### 6. Rewrite policy and its reports need a clearer trust boundary

The [probes](../examples/contract_audit_probes.rs) print:

```text
out_of_bounds_report_accepted=true
preserve_exact_empty_request=Err(Unsupported("source XMP retention requires an explicit editor"))
icc_required_without_target_icc=true
animation_capability_reports_exact_timing=false
```

`Plan::from_parts` checks reversed report ranges and output-size arithmetic. It
has no input length to check source bounds against, and cannot prove that report
actions match output chunks. This is allowed by its codec-validation contract;
calling the type “validated” must not be mistaken for validation by its constructor.
No memory-safety violation or observed malformed codec output is claimed.

`Request::with_policy` documents EXIF retention, but takes the same
`MetadataPolicy` used for broader encode retention. `PreserveExact` requests XMP
retention, and `validate()` rejects it without an explicit editor even before
seeing whether the input contains XMP. That is a naming/semantic mismatch, not
an XML parser bug.

**Before publishing:** separate byte preservation from descriptive filtering and
required rendering signals. Make unsupported preservation explicit. At least
validate report bounds against the input; test output/report correspondence in
codec conformance fixtures. Keep the runtime `Services` boundary: it avoids
pulling XML and compression into zencodec. Align rewrite limit errors with typed
limit reporting instead of forcing callers to inspect static strings.

**YAGNI check:** no proof-carrying graph of every byte transformation, and no
separate provider trait for every optional callback. These APIs are absent from
0.1.26, so reshaping them now is not a released breaking change.

### 7. Inventory coverage is not decoder agreement or a memory budget

[Inventory::validate](../src/inventory.rs) verifies structural coverage, nesting
and overlap. It cannot establish that a decoder selected the same duplicate EXIF,
accepted the same ICC workaround, or honored the same policy. `Disposition` is a
semantic assertion by the walker, not an execution trace or privacy verdict.

`push` bounds part count but uses infallible Vec growth; labels/details and
validation scratch also allocate. A part cap is useful, but is not a fallible
allocation contract or a bound on total bytes. No memory measurement is claimed
by this audit. Add fallible growth and explicit accounting before promising a
hard inventory memory budget; new APIs can be corrected before release.

Keep the inventory method on the configured job: disposition depends on policy,
limits and rendering/extraction choices. Decouple the scan from pixel execution,
not from the context that determines its answer. Native codec inspection should
remain callable without constructing a codec registry or pipeline. Expensive
per-codec scanners stay default-off; disabled calls return unsupported. The
shared inventory types/helpers themselves are currently always compiled into
zencodec, which is distinct from feature-gating codec scanners.

Avoid a separate scanner interpretation for every API. Share codec-owned carrier
selection, bounds checks and recovery decisions with decode, probe and rewrite;
let zenmeta share packet interpretation. Do not attempt one universal container
parser for JPEG markers, TIFF offsets and HEIF item graphs.

## What belongs in the breaking-change queue

Queue goals and acceptance gates, not speculative final signatures. None of these
recommendations requires an immediate version bump.

| Priority | Proposed next-break item | Additive preparation and acceptance gate |
|---|---|---|
| P0 | Replace silently ignored correctness requirements with validated operations; retain explicitly named hints for effort/tuning | Add required-operation path first. Unsupported lossless/alpha/HDR/policy requests fail through both generic and erased APIs; content-dependent claims checked against actual input |
| P0 | Replace coupled decode policy fields with independent interpretation, extraction and admission controls | Preserve old policy as adapter until migration. Same valid real-world corpus accepted by default; strict mode rejects documented exceptions; extraction denial cannot silently change rendering |
| P0 | Retire ambiguous source-versus-current output metadata authority at the encode boundary | Add checked finalization first. Native and generic, buffered and streaming, base/HDR, rotated/unrotated outputs agree in pixels and labeling; unsupported carriers reject or explicitly request conversion |
| P1, conditional | Retire static capability guarantees where effective operation queries replace them | Do not remove summaries merely for cleanliness. Prove effective queries against JXL, WebP, each AVIF backend, JPEG, GIF and TIFF; include depth, alpha, animation and gain-map combinations |
| P1, conditional | Make exact duration the canonical animation encode operation and retire ambiguous millisecond-only paths | Current rational API is additive and should ship independently. Required gate: representability checks, field overflow, zero duration, and total-play versus repeat conversion across animation codecs |

Unpublished `CodecSet`, inventory and rewrite changes belong in the **pre-release
correction list**, not the released breaking queue. Page/item selection can be
added as a separate operation: `ImageSequence::Multi` already exists, while its
generic page executor is explicitly still missing. Do not pretend an animation
frame index is an independent TIFF page or HEIF item identifier. Video is outside
this change; rational durations and separate sequence topology leave room for it
without adding packets, DTS/PTS, audio or a video pipeline now.

## Challenge the existing queue

The changelog already contains an older breaking-change list. Do not treat its
historical enforcement audit as a current consumer audit.

| Existing proposal | Recommendation |
|---|---|
| Remove `max_output_bytes` because encoders did not enforce it mid-stream | Reconsider. Output size is a valid requirement; distinguish bytes produced, bytes buffered and working memory. Implement bounded output or explicitly report unsupported enforcement. Missing enforcement is not evidence the requirement is unnecessary |
| Remove `max_animation_ms` | It can move from a resource budget to an admission constraint. Preserve the user's ability to reject excessive playback duration if there is a real caller; do not conflate elapsed compute with media duration |
| Remove omnibus limit-check helpers | Cheap deprecation cleanup if callers are migrated, not a reason for a major redesign. A helper checking shape does not certify runtime memory/output limits |
| Reduce ThreadingPolicy to Sequential/Parallel | Reasonable common denominator; retain backend-specific thread controls. Do not claim a Rayon pool controls internally threaded AV1 backends |
| Rename OutputInfo to PredictedOutputInfo | Add an alias/deprecation first. Its use in prediction and completed push-decode paths warrants a semantic review before a global rename |
| Remove deprecated ICC and descriptor helpers | Good cleanup after migrating consumers to the canonical zenpixels API. No extra metadata crate is needed |
| Remove ambiguous ISO gain-map parser/serializer names and old fraction conversion | Good cleanup after migration to explicit container-format and canonical fraction APIs |
| Remove SourceColor::has_hdr_transfer | Preserve a cheap source-signal query where useful; do not replace it with a mandatory pipeline dependency. Source signaling and a render decision are different questions |
| Remove ComputeEnvironment::new | Lowest priority: its compatible forwarding implementation is small. Keeping it deprecated costs less than prioritizing caller churn |

The widened zenpixels version range also needs release discipline. zencodec
exposes zenpixels types, so a resolver-selected incompatible release can change
type identity or break compilation even if zencodec's own signatures did not
change. Test supported dependency versions, including both sides of a permitted
breaking boundary when they exist. A version range is not a compatibility bridge.
This audit does not change the repository's standing dependency policy.

## Maintenance plan and YAGNI boundary

Keep the dependency direction already established:

- **zencodec:** small shared contracts and the useful published baseline: color,
  orientation, basic EXIF, ISO gain-map metadata, limits, errors and inventory /
  rewrite interchange types. It must work without zenmeta, zencodecs or zenpipe.
- **zenmeta:** richer packet parsing, XMP, MakerNote interpretation and auditing;
  codec-visible results use zencodec-owned contracts. Its own specialist API may
  expose its own types. Independent implementation releases do not require those
  types to leak into codec public signatures.
- **Each codec:** container framing, recovery, compressed packet transport, item
  selection and actual rendering/emission. Reuse shared packet interpretation and
  use the same local selection rules for probe/decode/inventory/rewrite.
- **testkit:** reusable behavioral assertions, plus native-versus-generic tests
  and third-party oracles in codec repos. Preserve existing fixtures and known
  real-world compatibility cases; do not replace them with easier synthetic cases.

No zenifd split solely for symmetry, no forced registry for metadata, no broad
parser rewrite, no new public type for every diagnostic string, no universal
backend enum, and no video implementation. Do not force all native errors into a
single concrete type just to make inventory look uniform; preserve native causes
and common classification. Add a stronger error bridge only with a demonstrated
operation that needs it.

The synchronization gate should exercise **the same file and configured request**
through native decode, generic decode, erased decode, probe, inventory and rewrite.
Cover duplicate carriers, malformed versus absent metadata, real-world recovery,
feature-disabled errors, input/part/memory caps, cancellation, EXIF framing,
color/orientation authority, and HDR auxiliary signaling. Structure-only
validation cannot replace those comparisons. Keep format oracles independent of
the implementation being tested.

For API changes, compare the actual registry artifact in CI for both std and
no_std, pin the snapshot tooling, and check at least one downstream implementor
and caller. Track accepted-input and metadata-selection changes beside signature
changes. Existing fixtures should gain additional assertions, not lose their
current requirements. Refresh the release baseline when crates.io changes.

Documentation discrepancies found here also need a coordinated correction:
`src/inventory.rs`'s overview and an older changelog addition still say `Ok(None)`,
although the method returns typed unsupported; `docs/correctness-model.md` calls
a deprecation warning compile-time prevention. The current method bodies are the
authority. Those existing claims were not silently rewritten by this audit.

## Validation of this audit

The complete archive-to-main comparison succeeded. Both feature selections passed
cargo-semver-checks. `just contract-audit-check` passed scoped formatting, clippy
with warnings denied, and the four executable observations above. These probes
print observations; they deliberately do not pin design gaps as desired test
expectations. No existing tests or public library signatures were changed.
