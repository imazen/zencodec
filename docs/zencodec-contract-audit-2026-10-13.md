# zencodec main versus crates.io: contract audit

**Recommendation:** keep the metadata extraction compatible with 0.1.26. Do not
spend a breaking release on moving modules or adding inventory. Reserve the next
break for enforceable operation requirements, independent parsing/extraction
controls, and an unambiguous contract between output pixels and their metadata.
Resolve the new dispatch/rewrite ambiguities before publishing them.

This records the source/API audit at the baseline below. Later implementation
separates ordinary and exact timing and moves rewrite execution to zenmeta.
The proposed boundary is under review for the next coordinated breaking
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

### 5. CodecSet is the ergonomic codec entry point

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

**YAGNI verdict:** keep `CodecSet` available by default. Registering concrete
codecs once and calling `decode(bytes)` is an essential ergonomic path for an
application that handles several formats. It also permits that path without a
zenpipe or zencodecs dependency. Its source line count and a search of indexed
callers do not establish that this API is unnecessary or expensive to compile.
Keep it free of pixel processing, CMS, plugin discovery and another format
registry. Review the semantics of individual higher-level methods, especially
`transcode`, without making basic decode harder to reach.

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


## PR 138 adversarial review (2026-10-13)

[PR 138](https://github.com/imazen/zencodec/pull/138) is a compatibility step,
not approval or implementation of every queued breaking change. The initial
review head was `bebdaf05153a`; fixes below are in `eddbdd51` and `4ec35983`.
The original baseline observations above remain a historical record.

**Verdict:** the ownership boundary and ordinary-versus-exact timing split are
suitable for API review after these fixes. Do not merge on the assumption that
the proposed breaking-release design has been approved. Do not describe this as
complete physical extraction of EXIF processing or completion of all adapters.

### Reproduced defects and resolutions

| Finding | Reproduction / consequence | Resolution |
|---|---|---|
| Child append partially mutated a rejected plan | A later out-of-bounds child report left earlier reports in the parent without their output bytes | Validate the whole child extent and translated ranges, reserve storage, then mutate (`eddbdd51`) |
| Same-length inputs were interchangeable | One builder accepted ranges from distinct six-byte inputs | Retain and compare borrowed input identity; lifetime keeps that source alive (`eddbdd51`) |
| Replacing context input bypassed admission | A context admitted at one byte accepted a three-byte replacement under the same one-byte cap | Check the current input and cancellation before each packet operation (`eddbdd51`) |
| EXIF expansion escaped the metadata cap | Two overlapping 130-byte values in a 169-byte packet serialize beyond a 190-byte cap | Preflight expanded storage, including tables, alignment and thumbnails, then check final output; exact aliases retain serializer deduplication (`eddbdd51`) |
| Integral timing could report false overflow | Four distinct large source clocks followed by an integral ten destination ticks unnecessarily expanded the phase denominator | Reduce the incoming destination-clock fraction first; carry large fractional sums without a 129-bit intermediate (`4ec35983`) |

The rewrite reproductions failed before their fixes. The integral-tick
reproduction failed before its timing fix. Additional tests cover successful
nested plans, cancellation before invoking a service, exact EXIF aliases,
generic/erased timing results and errors, and an independent integer endpoint
oracle across mixed clocks. Existing test expectations were not relaxed.

### API and maintenance judgment

- Published 0.1.26 APIs remain available. The deprecated EXIF/metadata helpers
  retain their implementations. New ordinary timing reports its actual encoded
  duration; exactness is explicit and rejection is a returned error.
- `Request` is a zencodec data contract; execution checks belong to zenmeta's
  `Context`. `Plan::from_parts` checks source bounds and output arithmetic,
  while codec planners remain responsible for container validity, report/output
  correspondence and carrier selection. No generic plan proves whole-file privacy.
- Core color, orientation, ISO gain-map and opaque-packet contracts do not depend
  on zenmeta. Rich parsing and editing can use zenmeta without leaking its
  specialist types into codec signatures. Keep this dependency direction.
- The public API snapshots now include the actual timing and rewrite signatures.
  Changed unpublished methods are source changes for git-pinned consumers even
  though they are not breaking changes against crates.io 0.1.26.

### Limits and release gates still open

The owned EXIF parser/writer remains in zencodec as the one compatibility
implementation. Its allocations are not all fallible. The new expansion check
bounds admitted serialized storage; it is **not** a measured peak-memory bound
or a general allocator-failure guarantee. Relocation must preserve its existing
fixtures and add fallible allocation before claiming that guarantee.

Timing phase arithmetic is bounded by u128. An intermediate common denominator
that exceeds this bound returns an explicit error without advancing state, even
if arbitrary-precision reduction could produce a representable final phase. A timing capability bit is
coarse discovery, not a promise that every rational duration is encodable.

Rewrite limit errors still use static strings. Align them with typed limit
classification before publishing the new rewrite surface; review that signature
change with codec consumers instead of adding another competing error taxonomy.
Effective capabilities, independent decode controls and current-pixel metadata
authority remain proposals requiring their own implementation and review.

[JXL integration](https://github.com/imazen/zenjxl/pull/27) and
[WebP integration](https://github.com/imazen/zenwebp/pull/113) validate the timing
split against native encoders and independent decoders. They pin the earlier
PR implementation, not the final audit fixes. AVIF, APNG and GIF migration is
not complete. All affected consumers need final-revision validation before a
coordinated release; neither these two integrations nor semver checks prove the
entire codec set agrees. The wider zenpixels dependency range also needs the
supported-version matrix described above.

### Validation after fixes

`just metadata-check metadata-semver contract-semver-minimal` passed workspace
all-feature tests and doctests, the zenmeta feature matrix, warnings-denied
clippy, and both registry comparisons. Each semver profile against the actual
0.1.26 archive reported 196 passed and 58 skipped. `just review-contract-check`
exercises the focused adversarial cases. Regenerated API snapshots are checked
with `just api-doc-check`. CI and review status remain live on the PR; no crate
has been published or merge authorized by this audit.

## Post-merge YAGNI pass: crates.io 0.1.26 to main

PR 138 merged at `e924793ec30bb69477ce32306678b4547ec9e644` on
2026-10-13. `cargo search zencodec --limit 1` still returned 0.1.26 at this
review. The [source comparison script](../scripts/audit-published-contract.py)
verified that every released source file and original manifest matches the
archive's VCS commit `998edf5f5152f675bbe3b4e959470dffe1536c36`, then
compared it with `origin/main`. The 222,215-byte patch changes 22 source or
manifest paths; its SHA-256 is
`19a7d95ccc865cea39b071e6e18f5666cebcf1fa0534c303e398e07143c49b76`.
Regenerate it with:

```sh
just contract-diff /home/lilith/.cache/cargo-read/zencodec-0.1.26 \
  /mnt/v/output/zencodec/yagni-2026-10-13 origin/main
```

This is source volume, not a build-time, binary-size or memory measurement.
`cargo metadata --no-deps` confirms zencodec has four normal dependencies
(`almost-enough`, `enough`, `whereat`, `zenpixels`) and no dependency on
zenmeta, zencodecs or zenpipe. Zenmeta is a separate workspace package;
`roxmltree` is optional there.

| Delta | Evidence and decision |
|---|---|
| Rational frame duration, cumulative quantizer and explicit exact encoding (`animation.rs`, 185 lines) | **Keep.** JXL and WebP adapters exercise the same contract and independent native readers checked encoded timing. The shared integer rounding rule prevents each codec inventing its own. The old millisecond methods are published and remain compatible. Finish the remaining adapter matrix before release. |
| Shared structural inventory (`inventory.rs`, 1,278 lines) | **Keep the interchange type and the job method.** Public code search finds inventory implementations in JPEG XL, AVIF, HEIC, TIFF, RAW, PNG, WebP, GIF and bitmap codecs. The large per-codec walkers belong behind each codec's default-off inventory feature; making the shared types themselves conditional would spread feature coordination through every consumer. This review did not measure the core type's compile cost. |
| Rewrite requests, results and services (`metadata_rewrite.rs`, 353 lines) | **Keep the codec interchange and the proven hooks.** [PNG rewrite](https://github.com/imazen/zenpng/blob/main/src/metadata.rs) calls `Services::compress` for a rewritten iCCP profile. [JXL rewrite](https://github.com/imazen/zenjxl/blob/main/src/metadata.rs) checks `Request::remove_reconstruction` before removing JPEG reconstruction data. Both are concrete uses outside this workspace. `Services::merge_xmp` has no caller in the workspace or the indexed imazen search examined here, so require a real merge operation before depending on that callback. One unused default method is a narrow API review item, not a reason to discard the service boundary. |
| `CodecSet` (`set.rs`, 829 lines) | **Keep available by default.** It gives multi-format applications one registration point and a direct `decode(bytes)` path without requiring zencodecs or zenpipe. Source line count is not measured compile cost. The earlier suggestion to hide or move it was a mistaken YAGNI judgment. Review `transcode` as a separate, higher-level operation; retain the straightforward decode and probe methods. |
| Additional EXIF filtering and `EntryRef`/`Ifd` (`exif.rs`, +738/-83 lines) | **Keep current compatibility, finish the planned move.** The added categories support retention tests, but the owned editing tree still sits in zencodec. Moving its implementation into zenmeta is the substantive next-break reduction; deleting basic EXIF orientation reading would defeat the standalone core. |
| One-shot trait defaults and estimate constructors | **Keep.** These are short forwards, reuse existing traits, and add no dependency. Remove only after a demonstrated ambiguity or consumer migration, not to optimize line count. |

Two contract risks matter more than deleting helpers. `CodecSet::transcode`
forwards a `Fidelity` request through setters that may ignore it. If the target
cannot accept the first decoded pixel descriptor, it decodes a second time with
preferred descriptors; a target encoder's lossless mode cannot undo a precision
or color change made during that step. It correctly reads metadata from the
**final** decoded result, so the earlier claim that it reuses the first result's
metadata was wrong. The remaining question is whether that final info describes
the transformed pixels and whether any requested rendering properties were
lost. The provided `with_metadata_policy` filters metadata, then calls
`with_metadata`, whose published contract permits a codec to skip unsupported
fields. Thus `PreserveExact` is a retention request, not proof that every source
carrier was re-emitted. Test concrete HDR/depth/alpha and metadata cases and
require checked admission for stronger guarantees; keep ordinary decoding
ergonomic. The rewrite
`Error::Limit(&'static str)` also needs typed limit classification before the
new API is published. Its strings currently distinguish input, output, packet
and carrier caps, but callers cannot classify them without comparing text.

The stronger policy concern is separate from format dispatch: `CodecSet` stamps
a requested decode policy onto the chosen job, but the released
`DecodeJob::with_policy` has a no-op default. A codec that does not override it
can ignore `strict()` with no error. Keep the ergonomic `CodecSet::decode` path;
introduce checked admission for *required* policy fields in the coordinated
breaking release. Optional decode hints can remain best-effort.

The EXIF ownership issue is maintenance and allocation semantics, not an
argument against `CodecSet`. Zenmeta's EXIF entry points still call zencodec's
published writer. That writer uses `Vec::with_capacity` and other infallible
growth, while zenmeta's preflight only bounds the serialized packet. Moving the
owned writer to zenmeta and making its untrusted-input allocations fallible is
a separate breaking-release gate; the current preflight is not a heap cap.

The published-to-main comparison is source-compatible in both checked feature
profiles; behavior still changed in EXIF filtering and metadata retention as
listed above. No current API removal is needed for the YAGNI pass. Inventory,
rewrite and `CodecSet` have not appeared on crates.io yet, so changes to those
proposals can be reviewed before publication without spending the planned
breaking release. The [breaking queue](../CHANGELOG.md#queued-breaking-changes)
should focus on released compatibility debt and codec correctness requirements.

The GitHub README currently suggests `zencodec = "0.1.27"` while the packaged
`README.crates.md` says 0.1.26. Only 0.1.26 is available from crates.io at this
review. Reconcile the two before treating the README as installation guidance
for the next release.

## Which contracts need parallel migration?

Checked again against the actual 0.1.26 archive on 2026-10-13. A published
method must keep working while callers adopt a replacement. A main-only API
can be corrected before 0.1.27 without consuming the future breaking-release
budget, although git-pinned consumers still need coordinated updates. The
proposals below are design targets, not approved public signatures.

| Wart or boundary | In crates.io 0.1.26? | Migration path |
|---|---|---|
| `CodecSet`, including `transcode` | **No.** Keep the registry and its direct decode/probe path available by default. | Correct `transcode` now: distinguish requested fidelity and metadata retention from what the encode actually carried; reject unrequested precision/auxiliary loss. Keep basic multi-format decoding simple. Do not spend a breaking release on its unpublished signature. |
| `metadata_rewrite::Error::Limit(&'static str)`, `Request`, `Services`, `Plan` | **No.** The entire module is main-only; zenmeta's rewrite helpers are also unpublished. | Replace string-matched limits with typed categories before the first release. Reuse `LimitKind` for input/output where it fits; metadata bytes, rewritten bytes and carrier counts need names of their own. Keep the proven PNG compression and JXL reconstruction hooks. Review XMP merge against a real carrier operation. |
| `DecodeJob::with_policy` and `EncodeJob::with_policy` no-op defaults, `DecodePolicy` fields | **Yes.** Codecs may ignore these setters. | Add a fallible, checked operation for *required* parser behavior, extraction and content admission; default implementations must reject unsupported requirements. Carry it through generic jobs, erased jobs and `CodecSet`. Keep existing setters as compatibility hints until codec adoption, then deprecate the ambiguous required use. Split the old coupled decode fields only at the coordinated break. |
| `EncoderConfig::with_fidelity`, `EncodeJob::with_metadata_policy`, `Metadata::from(&ImageInfo)` | **Yes.** Fidelity is documented as best-effort; metadata filtering can still lead to unsupported fields being skipped, and `ImageInfo` describes the source rather than every later pixel transformation. | Add an encode-time required-operation path that can inspect actual pixels, plus a report of which requested metadata and color carriers were emitted. Tie emission to the final pixel descriptor and color context while retaining source provenance. A config-time `FidelityMatch` cannot settle content-dependent losslessness; do not revive it as a guarantee. Keep best-effort quality/effort knobs for callers that want hints. |
| `Exif::to_bytes`, `Exif::filtered`, `exif::retain`, `Metadata::filtered` | **Yes.** Main marks them deprecated but retains their implementation. | Build the owned, fallible EXIF edit/serialize path in zenmeta, move its existing tests and real-file cases, then migrate consumers. Keep small EXIF orientation reading in zencodec. Remove the legacy editing tree only at the approved breaking release; zenmeta's current wrapper is a bridge, not the final fallible writer. |
| `AnimationFrame::duration_ms`, `OwnedAnimationFrame::duration_ms`, millisecond constructors and `push_frame` | **Yes.** The getters truncate or saturate rational durations on main; legacy encoding may quantize the requested milliseconds. | Main already adds rational `duration()`/`with_duration`, ordinary `push_frame_timed` returning actual timing and explicit `push_frame_exact`. Migrate adapters and consumers, then deprecate the lossy getters and millisecond-only trait requirement. Retain exact `from_millis` as a useful constructor. |
| `ComputeEnvironment::new` | **Yes.** | Main already adds `conservative()` and `host()` and deprecates `new()`. No larger abstraction is needed. |

The checked paths should have one invariant: **an unsupported requirement fails
at the earliest stage that can know, and no accepted operation silently drops
it later**. A codec may reject a static impossibility when building a job; an
alpha/palette/HDR-dependent requirement may need the actual input pixels. The
same outcome must hold through direct, erased and `CodecSet` calls. Use the
existing error chain and category model where it fits rather than creating a
parallel error universe. Tests need unsupported codecs as well as capable
ones, including a requirement that fails only after inspecting pixels.

Deprecation is a migration signal, not a correctness fix. First land checked
alternatives and conformance cases, then migrate codec implementations and
known consumers, then deprecate the old entry points. Keep the old behavior
through the compatible release. The eventual breaking release can remove
published legacy methods after the replacement has demonstrated equivalent
coverage on real files.

## Follow-up PR review: publication scope

The compatible proposals are split into [PR139](https://github.com/imazen/zencodec/pull/139)
and [PR140](https://github.com/imazen/zencodec/pull/140). Neither is a crate release.

PR139 keeps CodecSet available by default, guards still-image transcode against
known sequence/auxiliary loss and an unsupported native pixel descriptor, makes
inventory construction/validation allocation failures recoverable, and replaces
rewrite-limit strings with shared `LimitKind` values. PNG, JXL and JPEG planners
must update their pinned contract revision with the typed constructors.

Inventory's part cap bounds record count. It is **not** a total memory budget:
caller-created labels, details, spare capacities and parser scratch remain
separate. `try_children` exposes fallible collection; the convenience `children`,
formatting, cloning and summary helpers can still allocate infallibly. No new
byte-budget field is promised without an accounting model covering those inputs.

PR140 adds one checked policy mutation primitive per generic job and delegates
erased mutation to it. A separate `accepts_policy` predicate was removed during
review because a predicate and setter could disagree and generic overrides
could be bypassed by erased dispatch. A rejected mutable update leaves the job
unchanged. The reference encoder demonstrates ICC/EXIF/XMP gates, including
atomic rejection of a mixed supported/unsupported policy. Other codecs reject
nonempty checked policies until they implement this primitive; their existing
best-effort setters retain published behavior. CodecSet callers can obtain a job
and use its checked setter without another registry policy abstraction.

Rational getters and explicit timing submission are available in parallel with
the deprecated millisecond getters. Deprecation does not complete consumer
migration: callers must choose exact timing or intentional rounding, including
when their destination is a millisecond-only API. The old methods still work.

Required pixel fidelity, actual metadata/carrier delivery, and authoritative
post-transform color/orientation remain in the coordinated breaking queue.
Neither checked policy admission nor transcode's early guards proves these
properties. Do not add a config-time success report that cannot inspect the
pixels or encoded output. Before removing the old setters, demonstrate required
operation admission and outcomes with real codec adapters across all feeding
modes. This is migration work, not a signature-compatibility blocker for these
two additive proposals.

### Final comparison artifacts

Registry rechecked: 0.1.26 remains the published release. Regenerated source
diffs verify the same archive against both actual main and the proposal:

| Comparison | Revision | Diff SHA-256 |
|---|---|---|
| Main vs registry | `c73078989015e00c510c423e0219f521f26e2628` | `11cde6dc5868926026ac7fb3fc6729045a909137166cc61a3ae7f732572ce850` |
| Proposed zencodec source vs registry | `4d542d6412469d0c99e2e3e723cdcecd9d1208a1` | `56ca5b366e1e78b4bb05e86ee429618387d841c0184d4d1ff3fef4c0bfb5fb8b` |

Regenerate with `just contract-diff` and the revision above. The source/manifest
diff covers zencodec; zenmeta has no published baseline. Its additional nested
plan check (`bcc2ac50`) rejects a child whose borrowed input is not the exact
parent subrange. The unchanged JPEG gain-map rewrite tests exposed a stale
caller: generated child carriers used the outer context. The corrected planner
uses component contexts and combines both child plans under a fresh parent.
This validates the need for input provenance at the shared builder boundary.

Both minimal and all-feature semver checks against 0.1.26 pass locally. Workspace
tests, Clippy, MSRV 1.88, WASI and warning-free rustdoc also pass. CI now runs for
stacked PRs as well as PRs targeting main, and checks both semver feature modes.
Actual codec validation includes PNG/APNG metadata and timing, JXL metadata and
timing, JPEG gain-map rewrite preservation, GIF owned frames and WebP animation.
This does not claim every codec/backend has adopted checked policies, or that
all consumers have migrated from the legacy getters. AVIF has interrupted timing
work requiring reconciliation, and zenpipe's `FrameInfo` still stores only
milliseconds; its migration must specify rounding explicitly.
