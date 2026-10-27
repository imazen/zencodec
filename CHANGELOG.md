# Changelog

## Native image selection (unreleased experiment)

- Expose retained destination layout/access/timing facts to frame-route policy
  and its selected report. Derive them from prepared/active output drivers; byte
  streaming does not imply safe encoder backpressure or bounded memory.

- Add `Renderer::render_first` for checked first-image rendering without caller
  discovery boilerplate. Preserve discovery/source failures and do not substitute
  thumbnails or skip unsupported selections.

- Prefer complete sample-preserving conversion plus lossless encoding before
  encoder-only lossless evidence, without bypassing required route constraints.

- Couple retained frame conversion evidence to encoder routes. Expose target
  interpretation/timing, sample/range preservation, siting assumptions and
  scoped admission; qualify fixed-output native AV1 conversion before acceptance.
  This extends the unpublished review API, without a new callback method.

- Replace the private float conversion kernel with the reviewed public
  `CodePlane` / `YcbcrToRgbF32` mechanism. Preserve exact native outputs and
  existing policy, admission and ownership behavior; capture actual shared
  signatures and structured refusal facts in the guide.

- Qualify native float processing through existing OutputPlan and owned-image
  replacement. Preserve exact row-batch output, strided padding and video timing.
  Measure continuous/IEC transfer differences and the clipping preset footgun
  separately; no new public API or zenpixels source change.

- Execute unclipped native AV1 float conversion through the existing policy and
  owned-image contracts. Preserve sRGB interpretation, exact crop consistency
  and admitted ownership; compare actual output with an independent float
  converter under a fixed numerical gate. The public planar plan remains open.

- Resolve retained image-conversion routes before destination initialization.
  Disclose assumed chroma siting, extended-range preservation and scoped storage;
  preserve selected records and original causes on failure. Add deferred
  `StorageBudget::prepare_filled` reservations whose charge transfers on allocation
  or releases on refusal. Qualify the private route with native AV1 strips/crops.

- Add explicit converted-frame adoption with `ImageOutput::from_frame_pixels`.
  Retain exact timing/presentation and separate original component facts through
  `source_frame()` and `ImageFacts::source_frame`; current packed samples retain
  their own descriptor/context. Adoption releases the consumed native picture,
  preserves payload admission, and performs no hidden conversion or resizing.

- Add `Muxer::begin_exact_timing`, including dyn dispatch. Unqualified
  implementations refuse before legacy setup; qualified instances preserve
  supplied packet timing or reject before packet output. Metadata, workspace
  admission and future-packet validity are separate requirements.

- Add `MuxFinish::validate` and `MuxFinish::apply_to`; immediately deprecate
  `apply_finish` in favor of the method. Validate every patch before appending
  the tail or changing bytes, and reserve output fallibly. Existing testkit
  consumers use the replacement; the compatibility helper retains the fix.

- Resolve prepared native frame encoding transitions through the existing
  fallible policy resolver. Keep input epochs separate from output segments,
  refuse fixed-output changes, preserve terminal failure/cancellation behavior,
  and qualify exact native AV1 output with independent readback. Segmented
  replacement does not establish continuous-container reconfiguration or a
  whole-operation memory bound.

- Require declared auxiliary alignment in the private canvas projection and
  qualify the packaged JXL adapter with subsampled source extras and exact
  odd-origin crops. Unknown alignment refuses before output admission.

- Retain explicit emitted auxiliary registration through owned/native planes,
  clones and separate color replacement. Registration defaults to unknown;
  equal dimensions and source reduction factors never imply alignment.
  `ColorSamples` checks dimensions and freezes the adapter's sample-center claim.

- Retain the collection's selected composition view with each checked output
  before publication policy. Preserve storage and existing clones; reject a
  changed adapter view or conflicting owner claim with expected/actual facts.
  Packed storage alone still does not establish display readiness.

- Add `PlaneView::as_strided_samples` for existing plane kernels to borrow the
  declared storage span without a full-plane copy. Keep raw word encoding,
  mapping lifetime and inter-row padding; exclude trailing allocation space.

- Add `ImageOutput::replace_packed_color`: retain original auxiliary mapping
  guards and frozen source facts beside new color, with the existing publication
  resolver. Charges follow both owners; repeated replacement releases
  intermediate colors. Component codes refuse this same-coordinate operation.

- Compare actual float ICC conversion for one JXL presentation and five raw
  RGB/RGBA layers against independently executed Little CMS. Keep the original
  color tolerance, exact alpha/padding and admitted destination cleanup. Pin
  the converter's float/range and ICC-precedence corrections, execute the
  example shown by the guide, and record the remaining preset and owner gaps.

- Combine the existing output-plan and native-sample review dependency pins.
  Validate borrowed JXL color input directly into admitted destination rows
  across four CMYK and five RGB layers. Preserve exact color/context and
  charges through retained output; discard unpublished storage on admission,
  interpretation, late CMS failure and unwind. Complete auxiliary-plane
  publication and owner-preserving identity remain outside this color stage.

- Qualify profile-aware CMYK identity finalization with native JXL samples:
  preserve exact samples and ICC, refuse unsupported RGB conversion and missing
  current color context, and demonstrate that the required owned input and
  final output copies remain outside the decode output pool. Keep this separate
  from the descriptor-only row-converter refusal; track finalization ownership
  in zenpixels issue 69.

- Add operation-scoped output storage to `OpenRequest` and group resolved
  factory requirements in `CollectionOpenContext`. JXL honors the shared request;
  the private TIFF adapter refuses before source work. Remove the temporary
  compatibility method after qualifying the native adapter; no capability query,
  codec-specific storage setter or implicit fallback is required.

- Qualify the packaged JXL output allowance across 147 presentations and nine
  raw layers alongside ordinary allocation. Preserve exact samples, stride and
  existing independent-reader gates; check shared charges through concurrent
  retained outputs and after operation/input drop. Add the executing registration
  example to the guide and record scope separately from native workspace.

- Add `PackedImageOwner`, `ImageOutput::from_packed_owner` and borrowed
  `ValueSlice` for native/admitted packed storage. Preserve charges through
  mapping, policy waiting and clones without copying samples. Validate immutable
  layout/color facts and keep original backend causes behind bounded diagnostics.
  Native RGB/CMYK controls cover strided exact samples and rollback; identical
  inverted-CMYK conversion still refuses with `NeedsCms`.

### QUEUED BREAKING CHANGES

- Rename review-only `PublicationOffer::PreserveNative` to `ReturnHeld`: an
  explicitly converted image can be published through this same boundary. The
  offer promises the current held samples, not the original pre-conversion ones
  (`ab25c9d4`).

- Review-only `OpenRequest` is now `Clone` instead of `Copy`: cloned requests
  share the same retained-output storage allowance (`27cc94e3`). Native factories
  now implement only `open_with(source, CollectionOpenContext)`, replacing the
  review-only positional-argument opening method.

- Review-only `Samples::Values` now carries `ValueSlice` by value instead of
  borrowing the `Values` allocation container. This allows admitted auxiliary
  storage to keep its owner and charge. No released API is removed (`1f46337b`).

### Other changes

- Qualify packaged JXL input-size limits through the registry: actual resident
  length controls admission, both composition views retain exact native samples,
  and mixed refusals exclude the handled field. This is not workspace admission.

- Replace the duplicate JXL factory with the real optional `zenjxl::collection`
  package from its API review PR. Qualify all 147 presentation selections and
  nine raw layers through that package, retaining native bit-exact and pinned
  independent-reader gates. Keep TIFF private and unqualified resource/policy
  requirements explicit; update the source-extracted guide and receipt.

- Retain unhandled limit and decode-policy fields in typed collection-opening
  refusals. Callers can inspect bounded settings without knowing adapter-specific
  causes or parsing text; handled fields are excluded and refusals never authorize
  weaker retries. Show the exact error variants and fields in the source guide.

- Register borrowed native collections with `CodecSet::with_collection` and
  `open_collection`. Share one checked discovery/publication path; retain source
  lifetimes, cursor ownership and first-registration-wins errors. Require adapters
  to honor inherited limits/policy or refuse; preserve set, pulse and per-call
  cancellation after registry drop. Concrete TIFF/JXL adapters remain in the
  validation harness and refuse unqualified legacy configuration mappings.
- Qualify TIFF stored width, height and pixel caps before compressed-pixel reads
  and selected-output allocation. Keep over-limit inspection and smaller-child
  decode available; a tiny ROI does not bypass a stored-image cap. Accept no-op
  native defaults and exclude the explicitly encode-only output-byte field from
  decoding. Other unqualified requirements still refuse before source I/O.

- Make the unreleased `DynReadAt` protocol usable with thread-local sources.
  Request `dyn DynReadAt + Send` explicitly when moving an erased source
  between threads. Both boxed forms accept borrowed input. Native factories
  reuse this protocol; remove their duplicate private `LocalSource` trait.

- Add `CollectionOperationError::stop_reason()` (`2cdadc3c`) so callers can
  inspect the operation token's first observed stop across native, policy and
  wrapper failures. Preserve coincident source/application causes and traces,
  including failed opening and later calls on a finished operation.

- Add selected `ImageDescription` and `describe` to the shared collection and
  checked consumer after native TIFF/JXL/HTTP qualification (`76edcca8`). Keep
  source geometry distinct from rendered output and metadata completeness.
  Validate returned geometry, preserve cancellation and source-replacement
  failures, and charge repeated inspection against the discovery allowance.

- Expose signed presentation placement in the immutable shared output and
  publication facts. Validate it with the packed owner without clipping hidden
  samples; preserve typed overflow/geometry causes. Native JXL layers use that
  field, while blend/reference facts remain codec-specific. The image policy
  question borrows its diagnostic facts instead of embedding another copy.

- Promote the native-qualified collection mechanism to `zencodec::collection`
  and its checked consumer to `operation::CheckedCollection`. TIFF/SubIFD,
  JXL presentation/layer and conditional HTTP adapters now import those shared
  contracts. Preserve source-scoped identity, exact regions, operation-wide
  discovery-call accounting, traced errors and the first cancellation reason.
  Adapter factories remain private; CodecSet registration and
  native workspace admission are still separate work.

- Update the native JXL validation dependency after CMS callback failures were
  changed to return their original traced error. Recheck selected layers, TIFF
  regions and conditional HTTP source identity against that implementation.

- Qualify selected raw JXL F32 layers through the same checked consumer as TIFF
  and presentations. Freeze complete local samples with signed placement and
  source blend/reference facts; preserve native bits and allocation through
  policy. Require explicit inverted CMYK and black-plane polarity, and use
  fallible format lookup instead of panicking for unsupported representations.
  The collection remains private; source facts do not promise lossless editing.

- Qualify one operation stop token across private TIFF/JXL collection discovery
  and selected decode. Check source I/O, graph traversal, cached/exhausted cursors
  and final publication; pass the token into native JXL work. Inject cancellation
  through the observed polls of real TIFF/SubIFD and JXL operations without
  changing the common consumer signature.

- Keep frame and decoder-boundary diagnostic facts once in `DecisionContext`;
  `Selection` records only the publication or boundary action. This aligns the
  unpublished report variants with selected images and removes duplicated
  interpretation/timing from successful records.

- Add `operation::resolve_image`: the existing fallible resolver admits the exact
  held selected output or drops it on refusal, error, cancellation or unwind.
  Share `PublicationOffer` with native-frame questions instead of introducing a
  second publication option type. Native TIFF, JXL and AV1 exercise this boundary;
  privacy and workspace requirements without evidence refuse.

- Promote the native-qualified mapped output to `zencodec::image`. TIFF/JXL
  and AV1 validation import the same implementation. Preserve immutable facts,
  source/actual precision, auxiliary contribution and native ownership; expose
  typed traced region/mapping failures and reject conflicting native plane
  dimensions or code encoding. Collection discovery remains private.

- Move native frame ownership and borrowed component-plane mappings into
  `zencodec::frame`, retaining the existing media import paths as re-exports.
  Core gains no media/I/O dependency; native selected-image and AV1 consumers
  use one mapping contract. Accounted software allocations remain in media.

- Disclose whether retained spot-color samples already contributed to the
  returned color. Both native routes preserve identity and auxiliary bytes; an
  exactly representable fixture matches independent libjxl F32 output.

- Qualify subsampled auxiliary source shifts separately from emitted plane
  dimensions for all seven named JXL roles. The real-codec loop found and fixed
  native decoder issue 69; full independent decoding now validates generated
  fixtures before selected/sequential sample comparison.

- Carry complete native JXL auxiliary precision, spot/CFA parameters and selected
  timecodes through the shared output. Native encoder fixtures qualify all seven
  named auxiliary roles, half/float alpha, non-default source HDR/display facts
  and reverse-selected timecodes, with independent libjxl metadata inspection.

- Freeze interpretation with the private mapped output and validate packed
  sample/region geometry at construction. Retain native JXL source precision,
  HDR/display/preview and animation declarations separately from output color
  and metadata retention permission; qualify them with independent libjxl
  inspection and keep native AV1 ownership/timing tests passing.

- Replace associated native samples in the private collection with one mapped
  image output, exercised by TIFF, JXL and AV1. Preserve packed alpha, auxiliary
  roles and native video code-depth/ownership; keep rendering and resource gates
  explicit. Cross-check extra-channel facts with libjxl and AV1 samples with
  FFmpeg. Published crates do not gain this experimental API.

- Apply the WIC/OpenImageIO review to the private collection: attach region
  requirements to each selected image, return TIFF canvas/region geometry,
  and reject unsupported JXL regions explicitly. Preserve request-local
  failures and test differently sized pages and shared SubIFDs.

- Adopt native TIFF regions into the shared pixel owner with exact signed/wide
  sample bytes, explicit stride and original allocation ownership. Retain each
  selected directory's ICC and unapplied orientation; refuse unresolved alpha
  association and malformed/over-budget metadata. The native consumer exposed
  the external allocation constructor added in zenpixels PR 93.

- Run one private collection consumer over native TIFF and JXL with traced
  failures. Distinguish unsupported child discovery from successful exhaustion,
  preserve source-replacement causes, and execute the guide's exact selection
  example against both adapters. Native sample/interpretation differences remain
  a public-API promotion gate.

- Exercise native JXL presentation selection and dependency replay against
  sequential samples, with independent libjxl discovery. Fix the view before
  issuing identities; refuse required layers while the pinned native decoder
  cannot honor them. Keep the candidate private until the common consumer path
  and source-version/interpretation boundaries are qualified.

- Exercise operation-bound identities, lazy directory discovery and arbitrary
  nested/shared SubIFDs against native classic TIFF and BigTIFF. Keep image
  containment separate from reduced-level selection and requested composition;
  reject foreign cursors, cycles and exhausted discovery allowances explicitly.
  This candidate is not yet part of the public API.

## API guide (unreleased)

- Keep a visible Changes & why record for video, required density, the shared
  resolver and positional I/O, with each role’s consequences, extracted
  signatures, qualification evidence and open scope. Execute the new required
  density example against pinned native PNG/JPEG and checked-transcode adapters.

## Native frame policy (unreleased)

- Qualify native AV1 plane admission before publication policy. The executable
  guide configures the decoder's scoped picture-byte allowance; refused native
  allocations retain their original trace and invoke no publication callback.
  Both native allocation modes preserve all 15 tested depth/layout combinations
  under independent FFmpeg readback. Full decoder workspace remains unbounded.
- Retain decision history in a circular buffer, keeping the latest 16 records
  and chronological iteration/debugging without shifting every record for each
  video presentation. [Measured API cost](benchmarks/frame_policy_2026-10-25.md).
- Share immutable media interpretation/timing facts through `zencodec::frame`,
  retaining the existing media import paths. Extend the shared resolver with
  typed native-frame questions and bounded presentation diagnostics.
- Add `zencodec_media::policy::PolicyDecoder`, which holds each decoded frame
  until policy succeeds, preserves native samples/owners, and releases the
  decoder and unpublished frames on terminal failure or unwind. This guards
  publication after reconstruction; it adds no native workspace/privacy bound.
- Use `with_policy_set` consistently for the new frame guard and the existing
  still-image policy adapter. Native AV1 examples and independent re-encode
  qualification accompany the source-extracted guide signatures.

The required-resolution helpers also have an independent exact-rational oracle;
see [the density contract](docs/resolution-contract.md#independent-arithmetic-oracle).

Changes are grouped by package. zenmeta has its own [changelog](zenmeta/CHANGELOG.md).

## zencodec-testkit

### [Unreleased]

- Qualify required unclipped Gamma22 conversion on real AV1-derived samples.
  The regression fails against the old pixel revision and passes the fixed
  internal steps. Save linear/encoded/restored traces and independently inspect
  them, including deliberate clipping and missing-input controls.

- Migrate native JPEG, AV1/video and TIFF/JXL conversion callers from deprecated
  `forbid_lossy()` to `retain_channels()` without changing their policies. Exact
  carriage continues to use `new_preserving_samples`. Pin all native pixel
  consumers to one review revision and show the actual replacement signatures.

- Native JXL now exercises the shared `OutputPlan::new_preserving_samples`:
  unchanged rows borrow and opaque-alpha expansion preserves original sample
  bits. The pinned conversion API and guide distinguish exact representation
  changes from approximate unclipped color conversion.


### Fixed

- Compare discarded ICC/CICP against a clean same-policy encode in the testkit, distinguishing native/default color from retained optional metadata; unexpected color values and discarded descriptive metadata still fail.
- Probe metadata capability flags with separate ICC, EXIF, XMP and CICP inputs, allowing mutually exclusive color carriers while retaining the mixed-carrier authority checks. Regressions still reject dropped declared channels and hidden write capabilities.
- Declare the reference decoder's existing ICC, EXIF, XMP and CICP readers so its conformance tests actually exercise metadata capability honesty.


#### QUEUED BREAKING CHANGES

- Approved for testkit **0.2.0**: `RefDecodeJob` changed from a publicly constructible unit struct to a private-field stateful struct in the gain-map harness (`857fab1`). Implementor tests should obtain jobs from `ReferenceDecoderConfig::job()` or `ReferenceZcrDecoderConfig::job()`. This is a testkit-only break; zencodec retains its compatibility gate.

#### Changed

- Prepare the independently versioned testkit 0.2.0 release. Its zencodec requirement now starts at 0.1.27, which supplies the new contract APIs. No crate has been published.

#### Added

- `check_streaming_output_honesty` and `check_animation_streaming_output_honesty`
  check that a refused `begin` leaves the encoder unchanged: after the first
  zero-size (zero-width canvas, for animations) refusal, `begin` with the real size
  on the same encoder must report the same timing and stream the same file. Both
  checks stated the rule without testing it; a wrapper that stores the refused
  declaration (zenjpeg's behaviour before `cd8ac18b`) passed. Two new misbehaving
  wrappers in `tests/streaming_output.rs` are caught, and fail again when the new
  checks are disabled.
- `tests/dyn_streaming.rs`: the reference encoder, still and animated, streamed
  through `DynEncoder` and `DynAnimationFrameEncoder` must report the typed timing,
  have bytes pending before `finish`, and produce the typed encoder's exact bytes
  through both `consume_output` and `take_output`. Nothing exercised the dyn shims'
  forwarding; one that dropped `pending_output` would still have produced a correct
  file from `finish`.
- `check_read_at_contract`, `check_seekable_capability_honesty` and
  `check_incremental_capability_honesty` (chunk-boundary independence against the
  one-shot pixels, truncation must error); the last two run in `check_all`. The
  reference codec implements both sans-I/O paths; the minimal codec declares neither.
  The testkit now depends on `zencodec-io` for its slice driver and collect sink, so
  `zencodec-io` must publish before the testkit can.
- `TestImage::rgb8_gradient_seeded`: distinct opaque frames, for animation codecs
  without partial transparency.
- `check_streaming_output_honesty` also drains with `take_output` (into empty and
  non-empty vectors) and requires `begin` with a zero width or height to agree with
  `encode` of that size. `check_animation_streaming_output_honesty` (run by
  `check_all`, with enough frames for the reported lag) verifies
  `AnimationOutputTiming`: lag below a declared frame count, bytes pending across
  every lag window, identical bytes drained, taken and undrained, frames and
  durations that decode as the same encode without `begin` does (not compared with
  the input, so lossy animation codecs can run it), a refused second `begin`, and a
  refused `finish` short of the declared count. The reference animation encoder
  streams when the frame count is declared. Six misbehaving wrappers in
  `tests/streaming_output.rs` show each new rule is caught.
- `check_streaming_output_honesty` now enforces the rules: lag below the height;
  bytes pending across every lag window (an understated lag or a stall after one
  strip fails); full, uneven and over-sized drains give identical bytes; a second
  `begin` and `encode` after a streaming `begin` fail; an image too short to verify
  the lag fails instead of passing. `check_all` sizes its image from the default
  configuration's lag. The reference encoder refuses `encode` after `begin`. Found
  by an adversarial review (each gap reproduced, then covered by a misbehaving
  wrapper in `tests/streaming_output.rs`).
- `check_streaming_output_honesty` checks the `OutputTiming` that `begin` returns:
  a streaming timing needs `streaming_output`; once more than `lag_rows` rows are
  pushed, bytes beyond the header must be pending; `AT_FINISH` keeps
  `pending_output` empty; a `RowLevelEncode` rejection at `begin` passes. Regression
  tests (`tests/streaming_output.rs`) show it catches rows held behind a zero lag,
  bytes behind `AT_FINISH`, and streaming without the capability.
- `check_incremental_decoder`: the incremental contract for any decoder factory
  against the decoder's one-shot function, for decoders not reached through a
  `DecoderConfig` (such as `helpers::WholeFileIncremental`).
- The incremental contract's truncation rule is parity with one-shot: on a cut
  where one-shot fails, the incremental decoder must fail with a codec error;
  where one-shot succeeds (the first frame of an animation whose later frames
  are cut, a lenient BMP RLE body), it must succeed with the same pixels.
- `check_incremental_contract` (the full sans-I/O step contract, run through
  `check_incremental_capability_honesty`) and `check_streaming_output_honesty`
  (run by `check_all`); the reference encoder streams its output after `begin`
  (`d8b9e636`, `e54b4175`).

- Explicit structural-prefix and distinct-frame timing oracles require typed refusals and independent expected pixels. Existing compatibility checks remain unchanged.

- The [contract coverage matrix](docs/testkit-contract-coverage.md) records consumer evidence, independent readbacks, and remaining API/testing gaps.

- Checked policy tests compare builder, mutable and erased results, including pixels after rejection (`43061fcd`).
- Native pixel fixtures preserve stride and color context with zenpixels 0.2.14 as well as newer versions. All tri-state metadata embed-gate combinations and each decode-policy field are exercised.

- New fixture-driven encode checks accept native `PixelSlice` views, including
  strided, 16-bit and floating-point inputs, without an RGB8-only fixture type (`304a03bb`, `ab420b6c`).

- The default animation cross-path suite now checks source durations, and
  `check_animation_timing_unsupported` tests typed refusal without damaging
  buffered legacy frames on adapters that do not support rational encoding (`3ef1cfde`).

- Fixture-driven timing, checked-policy, inventory input-limit and metadata-rewrite
  conformance checks compare generic and erased dispatch and explicit expected outputs (`919dd678`).

- `check_inventory_dispatch` compares generic/dynamic results and disabled-operation
  errors. `check_all` also checks the disabled inventory contract without changing
  its published generic bounds; erased-dispatch checks remain separately callable.
- **`check_inventory`** (in `check_all` when a decoder declares `inventory`): on a valid file,
  the file plus appended junk, and every truncation, the inventory exists, validates, and never
  claims the junk as consumed. The `reference` codec implements `inventory` as the worked example.
- **`check_gain_map_roundtrip`** (in `check_all`) — the gain-map encode/decode
  contract (Phases 1, 2 and 4 of `docs/gainmap-pipeline-scope-2026-06-08.md`,
  #24): an encoder without `gain_map` must reject `with_gain_map_pixels` /
  `with_gain_map_encoded` with `UnsupportedOperation::GainMapEncode` (never a
  silent drop); a declared encoder embeds the scope audit's test matrix —
  1-channel, 3-channel, and backward-direction (HDR base) maps, sub-resolution,
  with dyadic ISO 21496-1 params and a PQ alternate CICP — and rejects a
  foreign-format encoded map; a declared decoder reports the map from `probe()`,
  surfaces a `DecodedGainMap` only on `Components` / `with_extract_gain_map`
  (never under the default `BaseOnly`, and both requests yield the same map)
  with exact params and byte-exact pixels on a lossless codec, falls back
  honestly on `ReconstructHdr` without `reconstructs_hdr` (components or
  `UnsupportedOperation`, never an SDR buffer labelled PQ/HLG), the surfaced
  map transcodes through a fresh `with_gain_map_pixels` unchanged, and (when
  `push_rows` is declared) the incremental encode path carries the map too.
  Mutation-verified against the reference (9 breakages, all caught).
- The `reference` codec now declares `gain_map` on both ends and is the worked
  example of the encode contract: `with_gain_map_pixels` validates and stores
  an 8-bit 1/3-channel map, `with_gain_map_encoded` byte-carries its own format
  and rejects any other, the wire carries pixels + ISO 21496-1 metadata (flag
  bit 6), `probe()` reports it, and decode surfaces it opt-in only.
- **`check_fidelity_honesty`** (in `check_all`) — the per-codec `Fidelity`
  contract: a declared `lossless` honours a `Lossless` request with
  `Some(Lossless)` and byte-exact pixels; a codec without `lossy` never reports
  `Lossy` (it promotes to `Lossless` and says so, or reports `None`); a codec
  with `lossy` never answers a lossy request with `Lossless` (and, with a
  `quality_range`, reports the `Lossy` target it mapped to); anything reported
  `Lossless` decodes exactly; `is_lossless()` agrees with the resolved report.
  Every `LossyTarget` arm is exercised on top of a prior `Lossless` request.
  The `reference` codec is now unconditionally lossless (`is_lossless()` =
  `Some(true)`, so a `Lossy` request promotes through the default bridge) —
  the exemplar of the promote-and-report rule (#26).
- **`check_color_authority_spec`** (in `check_all`) — the `ColorAuthority` a
  decoder names in `ImageInfo.source_color` must be the one its format's spec
  assigns for the colour fields it read back, on every metadata mix the encoder
  can produce (none / ICC / CICP / both), from both `decode()` and `probe()`.
  **`check_source_color_authority(format, &SourceColor)`** is the unit form for a
  codec's own fixtures and **`expected_color_authority(format, has_cicp, has_icc)`**
  the shared answer key (PNG/JXL/HEIC/Radiance HDR: CICP outranks ICC; AVIF:
  MIAF order, ICC `colr` outranks `nclx`; JPEG/WebP/GIF/TIFF/BMP/ICO/PNM/
  Farbfeld/QOI/TGA/DNG/RAW/PDF: always ICC; no rule for EXR/JP2/SVG/Custom).
  The shared test helper the decoder audit asked for (#11).
- **`check_color_context_consistency`** (in `check_all`) — whatever
  `zenpixels::ColorContext` a decoder attaches to its output buffers must be
  class-valid (an ICC only rides a layout its device class, header bytes 16..20,
  describes) and identical across the one-shot, streaming, and animation
  (borrowed + owned) decode paths. Lenient about *whether* a context is attached,
  so codecs that have not adopted the convention still pass.
  **`check_color_context_attached`** (opt-in) is the strict positive direction: a
  decoder that read colour back into `ImageInfo.source_color` must attach the
  authoritative field to the buffer (#25).
- The `reference` codec now attaches a class-gated context on every path
  (`reference::class_gated_context` is the runnable form of the convention's
  ranked rules) and the ICC fixture carries a real `RGB ` device class.
- **`check_native_hdr_roundtrip`** (in `check_all`) — Phase 0 of the gain-map /
  HDR delivery scope (`docs/gainmap-pipeline-scope-2026-06-08.md`): when both
  ends declare `hdr`, BT.2100 PQ and HLG must survive a `PreserveExact` round
  trip — the CICP (where both ends declare the channel), `content_light_level`
  exactly, `mastering_display` within container fixed-point precision, 16-bit
  pixels where both ends declare `native_16bit`, and a decoded descriptor
  labelled `Pq`/`Hlg` (equal to `RGB16_BT2100_PQ`/`_HLG` on the 16-bit path)
  rather than sRGB. Skipped for SDR-only codecs (#24).
- The `reference` codec declares `hdr` + `native_16bit` on both ends: its wire
  format carries CLLI (`flags` bit 4) and MDCV (bit 5) and RGB16/RGBA16 samples
  (`bpp` 6/8), and the decoded descriptor is stamped from the file's CICP
  transfer/primaries.

### [0.1.0] - 2026-07-14

First crates.io release of the conformance harness (previously an unpublished
workspace member consumed via git pins — which forced every consumer to carry a
`[patch.crates-io]` zencodec entry to avoid a duplicate-crate graph; publishing
removes that whole class).

- Public API ablated to the consumed + designed-for-adoption surface before
  first publish: the `check_*` conformance functions, `check_all`,
  `assert_uses_codec_error_envelope`, `Conformance` / `Failure` / `TestImage`,
  and the `reference` worked-example codec. Now `pub(crate)`: the `minimal`
  false-direction codec, the `fixtures` module, and
  `is_incomplete_input_category` (all unused by any external consumer;
  re-exposing later is an additive change).
- zencodec dependency requirement raised to `0.1.26` (the API the harness
  actually targets: two-level `ErrorCategory`, `Stopped`, truncation-series
  check).
- README: badge row, orientation + error-classification check descriptions,
  `minimal` reframed as internal; LICENSE files included in the package.

## zencodec-io

### [Unreleased]

#### QUEUED BREAKING CHANGES

- `FileReadAt` and `BufferedReadAt` return `StdSourceError`, preserving native
  I/O errors alongside structured `SourceError` refusals. Their old concrete
  error signatures discarded kind, OS code and custom payloads (`728b7f02`).

#### Fixed

- Drop the native controlled session on policy/native failure or unwind while retaining its report. Native JPEG qualification caught retained input after decline; whole-source baseline/progressive pixel parity and refusal before unqualified late-profile effects are tested.

- Async image drivers bound their copied request alternatives before allocation.
  Failed compute batches no longer drain newly pending output after the failure.

- Check request address overflow before serving bytes. `ReadCursor::max_window`
  now bounds prefetch and skipped-input buffering as well as contiguous requests.
- Publish fetched ranges only after the complete batch succeeds; partial source
  failures and async cancellation cannot expose unfilled cache bytes on retry.

- Buffered sources reserve retained capacity fallibly before reads, reject
  impossible exact spool requests before consuming input, and retain only the
  declared window during large forward reads. Exact-cap EOF uses one stack-byte
  probe; excess input makes the spool terminal without growing retained payload.
- `FileReadAt` uses safe positional file reads on Unix/Windows, with seek/read
  retained for other targets. Construction reads metadata without moving the cursor.
- Synchronous and async cursors reuse prefetched ranges, read only a missing
  overlapping tail, and reuse batched ranges for sequential requests. Already
  satisfied sequential requests no longer trigger a prefetch refill.
- `encode_to_writer` recognizes a row-level refusal in the shared error category
  as well as legacy typed causes, including one-shot BMP encoders.
- Clamp overclaimed consumption of fetched ranges instead of panicking on the next offer, and require `std` for the driver-probe example (`7cc3d3ec`).
- Default-feature rustdoc renders gated driver names with their required features instead of unresolved links (`7e3c8666`).

#### Added

- `ReadAtReader` bridges positional sources to synchronous `std::io::Read + Seek`
  with an independent cursor, preserved native errors and no implicit buffering.
  Native TIFF qualification exercises disk, direct and buffered range sources,
  planar tiles, pages, reduced levels and wide samples.

- `PolicyDecoder` serves actual controlled suspensions through the shared fallible resolver, preserves native failures and bounded reports, and poisons execution after callback failure or unwind. Existing transports drive the adapter.

- `ControlledStepper` exposes mandatory suspensions through the existing byte
  cursors so adapters can prepare a destination before approving pixel work.
  Repeated calls at a pause do no I/O or native work; failed replies and native
  failures release the held session even when the stepper remains alive.

- `storage::StorageBudget`, `OwnedStorage` and `SharedStorage` admit typed,
  fixed-size payload allocations before creating them and retain their charges
  through clones and subviews until the last backing owner drops. Byte and object
  limits are independent; native codec workspace and allocation overhead remain
  outside this managed-payload scope. See `docs/storage-admission.md`.

- `WorkQuota` and `DriveOptions::with_work_quota` bound async compute turns and
  ready packet loops by complete codec steps. Quota yields do not drain encoded
  output; CPU fairness remains separate from destination backpressure.

- `IoLimits`, `InputError` and `DriveOptions::with_limits` provide retained-input
  admission across synchronous and async incremental drivers. Optional range
  prefetch/gap merging fits the allowance; required spans refuse before I/O.
  Allocations are fallible, borrowed slices stay allocation-free, and source
  errors retain their own types. Cursors expose retained input capacity.

- `BufferPolicy::try_for_pattern` refuses insufficient required lookback instead
  of clamping it. `BufferedReadAt::retained_bytes` reports cached payload separately
  from consumed input. `StdSourceError::Allocation` preserves allocation refusals.
- Observational I/O contract probes and a high-expansion transcode benchmark
  for output backpressure, spool admission, and one-shot encoder fallback.
  A pinned standalone harness adds positional-file comparisons, native QOI
  batching and boundary tests, and MP4/WebM/IVF transport checks with a strict
  FFmpeg decode oracle. Private prototypes run only in exported source trees.
  Results are in `benchmarks/io_validation_2026-10-24.md`; library behavior
  is unchanged.

- `zencodec-io` (unpublished): `ByteStepper` and pull cursors (`SliceCursor`,
  `ReadAtCursor`, `ReadCursor`, `tokio_io::AsyncCursor`) so every driver serves any
  byte-stepping stage, demuxers included; the image drivers are loops over them with
  unchanged signatures, and `DriveError` / `PumpError` are aliases of the generic
  `DriveFailure` / `PumpFailure`.

- `zencodec-io` (unpublished): `encode_to_writer` (begin, 16-row-or-larger strips,
  a write after the header and each strip, one-shot fallback when `begin` declines
  row-level encode), `write_pending`, `write_pending_animation`,
  `tokio_io::write_pending_async`, and `EncoderSink::take_output`.

- Workspace crates (unpublished): `zencodec-io` and `zencodec-coroutine`
  (`async`-authored incremental decoders, `no_std`). `zencodec-io` drives a decoder
  from a slice (zero-copy), a `ReadAt` (ranges batched into one `read_many_at`
  round), a `std::io::Read` stream (with a window limit) or, under `tokio`, an
  `AsyncSource`: `decode_async`/`transcode_async` keep decoder, sink and window in
  the task and run decode batches on a `Compute` pool, so no thread is held while
  bytes are in flight; `pull_decode_on_thread` bridges pull-native codecs. Every
  driver reports a decoder that spins, asks past a known end or asks for nothing
  (`ProtocolViolation`) instead of looping. Drivers take one `how_far::Pulse` for
  cancellation and progress. The input window is zeroed once and compacts only when
  its tail is full (`34346a05`).

## zencodec-coroutine

### [Unreleased]

#### QUEUED BREAKING CHANGES

None.

#### Added

- `controlled_with_storage` admits input payload, a bounded range index and reusable row staging through the shared `StorageBudget`. Input/index admission precedes codec initialization; row admission precedes sink effects. Completion and failed policy sessions release owned buffers.

- Controlled construction and awaited interpretation boundaries block affected output until final adoption, reject changed final facts, and poison failed/unwound sessions. Legacy execution refuses controlled sessions. Row copying now reserves fallibly and refuses unapproved output before allocation. This does not add a native or whole-operation memory bound.

## Workspace

### [Unreleased]

#### QUEUED BREAKING CHANGES

- None.

#### Added

- Native JPEG qualification calls the codec-owned controlled factory through stored requests, with exact pixels over slice/range drivers and typed late-ICC refusal before sink output. Pinned native implementation: zenjpeg PR 240 (`3e9050ef`).

- CI and `just doc-default` check workspace and standalone I/O documentation with default features and warnings denied (`7e3c8666`).

#### Fixed

- The migration-page keyboard check waits for fragment navigation before
  asserting its URL, retaining the existing skip-link expectation.

- Apply accounting-scope validation to disk-only caps in the private policy
  experiment, with a failing-before/fixed-after regression. Public APIs are unchanged.
- `zencodec-coroutine` delivers rows or metadata deposited during the future's final poll before reporting completion (`7cc3d3ec`).

## zencodec-media

### [Unreleased]

#### QUEUED BREAKING CHANGES

- On the unpublished media API, `MapError::Backend` takes an owned
  `Box<dyn Error + Send + Sync>` instead of `String`, retaining concrete causes
  and location traces through mapping, adaptation and erasure (`6241cad4`).

- `PlaneData` gains accounted 8/16-bit storage variants on the unpublished media
  API. Exhaustive matches must handle the new variants (`5019a40a`).

- `erase_video_encoder` now applies strict terminal-error handling, including
  native validation errors. Callers deliberately using a backend's retryable
  validation contract can use the error-only `VideoEncoderShim` (`ffaddb6b`).

#### Added

- Add `std`-enabled checked output drivers: `output::PreparedOutput` retains
  the actual muxer/writer without publishing, and `output::Output` validates
  packet clocks/epochs and finish patches with terminal failure semantics.
  Exact timing is the default; `with_container_timing` explicitly opts into
  the native muxer's ordinary timing. `Forward` and `SeekBack` derive access
  from actual writers and preserve file prefixes. Native qualification now
  imports this shared implementation instead of keeping a private copy.


- `PumpError::source()` exposes upstream and downstream failures to standard
  error-chain inspection. Typed and erased pump regressions also retain native
  causes and terminal-session behavior.

- Accounted packet payloads and native-word planes preserve storage charges
  through packet take/clone, side data, frame clones, show-existing presentations
  and old epochs. Legacy conversion to an ordinary vector remains explicitly
  outside managed accounting.

- `FusedEncoder` enforces cancellation before backend work and refuses all work
  after failure. Both fused session wrappers remain failed across a caught
  backend/stop unwind; a successful decoder reset still reopens the decoder.

## zencodec

## [Unreleased]

- Add `CheckedCollection::decode_with` for adapter processing before the single
  publication decision. Preserve concrete transform errors, recoverable request
  refusal and terminal cancellation/unwind. The private renderer's
  `render_selected` uses the hook for TIFF/SubIFDs and JXL.

- Qualify a private combined held-image render operation over native TIFF and
  JXL: exact required color, oriented regions, retained auxiliary values and
  finished-output publication policy. The loop found and fixed a same-profile
  sample change in zenpixels-convert; no new shared render API is claimed.

### Native API validation

- Pin the required-descriptor `OutputPlan` constructor and validate exact native
  JXL premultiplication through borrowed identity and admitted strided output.
  Preserve format-only behavior and the independent color/selection gates.

### Added

- Optional `PackedImageOwner::retain_auxiliaries` and `AuxiliaryImageOwner`
  retain immutable auxiliary allocations independently of replaced color.
  Inseparable owners keep existing behavior; errors preserve their typed cause.

### Added

- Required physical resolution on checked encode requests and generic/erased
  jobs, with exact cross-unit and unknown-unit aspect comparison. Unsupported
  jobs refuse before encoding; no implicit rounding or physical-unit loss.
  [Contract and qualification scope](docs/resolution-contract.md).

- `DecodeRowSink::max_buffer_rows` exposes admitted strip capacity. Shared copy
  helpers and coroutine delivery divide native batches without growing sink
  storage or changing samples, stride, coordinates or progress-event counts.

- Checked encode adapter defaults fill only unspecified policy fields. Explicit caller requirements, including false gates, win in local and transferable preparation; unsupported defaults refuse before encoding.

- Coupled transcode choices in the shared policy resolver, with typed metadata treatment, opaque-profile retention and scoped storage facts. Configurable presets reject unknown workspace bounds and missing privacy evidence; final interpretation and fresh choice tokens are required. Adapters retain the prepared route and validate output before constructing its outcome.

- `CodecSet::with_send_encoder`, `EncodeRequest::prepare_send` and
  `PreparedSendEncode` retain thread-transfer capability through job erasure.
  Registration checks the concrete job's `Send` bound without restricting
  existing thread-affine jobs. Preparation refuses unqualified registrations
  before source consumption; shared configuration checking and exact retained
  job identity are covered by a real thread-transfer test.
- `EncodeRequest::prepare` retains checked jobs for an adapter's coupled decision.
  Ordinary encoding uses the same preparation. `PreparedEncode` exposes the
  confirmed offer and transfers the original job without reconstructing it.

- Policy decision reports retain typed question context on refusal and callback failure; controlled decoders can report payload-free interpretation failure reasons (`184f1291`).

- `DecodeRequest::start_controlled` binds required policy/output and inherited context to registered controlled jobs. Generic/erased job creation share one primitive; default refusal prevents silent fallback, and required output mismatch fails before adoption or sink effects (`fd132f55`).

- Shared `storage` module for allocation admission in codecs and authoring helpers; `zencodec_io::storage` re-exports the identical types for compatibility.

### Fixed

- `find_cause::<E>` inspects traced `At<E>` values before following their source
  links, preserving concrete native error lookup before and after erasure.
  Codec traits retain their associated error types.
- With `std`, cause lookup enters custom `std::io::Error` payloads as well.
  Native TIFF source-failure tests exposed the otherwise skipped transport
  error; nested I/O and traced payloads remain downcastable after erasure.
- The default `DynEncoder::begin` now refuses unsupported row encoding, matching
  `Encoder::begin`. This fixes handwritten dyn implementations; the ordinary
  typed-to-dyn shim already delegated correctly. Rejection preserves the
  one-shot fallback and the concrete `RowLevelEncode` cause.
- The default `Encoder::begin` refuses unsupported row encoding before any output,
  allowing `encode_to_writer` to use one-shot encoders that inherit the optional
  row methods. A strided multi-strip regression checks exact output.

### Added

- `ControlledIncrementalDecode` adds mandatory interpretation suspensions, scoped replies and fused enforcement independently of metadata events (`050069b1`). `Question::Boundary` supplies typed mechanisms and finality facts to the existing resolver; native codec qualification remains explicit.

- `operation::{DecodeRequest, EncodeRequest, OperationError}` store required
  policy, output descriptor and fidelity choices, then validate them automatically
  during execution through `CodecSet` templates. Required policies use checked
  setters, explicit descriptor preferences allow native fallback, and native
  errors remain recoverable through the traced result. Requests are reusable.
- `operation::PolicyResolver` selects prepared encoder configurations through
  typed offers and opaque question-scoped choices. Callbacks return traced native
  errors, can decline explicitly or delegate once to a configurable `PolicySet`.
  Successful requests return `Outcome<T>` with bounded decision records; selected
  native execution failures retain the report. Unsupported fidelity alternatives
  are filtered before policy, malformed lists are refused, and native execution
  is never retried automatically. This is the first production question domain;
  late metadata and transcode decisions remain separate implementation work.

- `EncodePolicy::strict` (`with_strict`, `resolve_strict`), the encode-side
  twin of `DecodePolicy::strict`: when set, an encode the target can only
  approximate fails instead of being approximated. zencodecs uses it for a
  colour description the target format cannot carry, which by default it
  converts to sRGB. `None` is lenient; additive (`EncodePolicy` is
  `#[non_exhaustive]`).
- `helpers::set_exif_color_space(data, srgb)`: rewrites the EXIF ColorSpace tag (0xA001) in place to `1` for sRGB or `0xFFFF` otherwise, never adding it, so an encode can keep the tag in step with the colour it writes (imazen/zencodec#132). Shares the in-place integer patcher with `set_exif_orientation`.

### Changed (unpublished sans-I/O surface from `d116079`)

- `IncrementalDecode` is one call, `step(input, sink, stop) -> Result<Progress, StepError>`
  (`d8b9e636`). The driver owns every input byte: `Input { offset, bytes, total_len }`
  is offered, the decoder takes a prefix (`Progress::consumed`) and reports one
  `Event` (`Need`, `Rows`, `Metadata`, `Done`; non-exhaustive, unknown variants are
  informational). After `Need`, `requested()` lists `Request::AtLeast { offset,
  min_len }`, `Request::Range` or `Request::Len`; any one satisfies it. No decoder
  needs an input buffer. Errors are `StepError::{Codec, Sink, Cancelled, Finished}`
  and terminal; `Fused` and `erase_incremental` enforce the termination rules for any
  decoder, and `DynIncrementalDecoder` is `dyn IncrementalDecode<Error = ErasedError>`.
  Removed from the earlier feed/step draft: `feed`, `feed_and_step`,
  `wrap_sink_error`, `Step`, `ByteRange::from_offset`. `IncrementalDecoderShim` stays
  public for codecs that erase without `Fused`; `erase_incremental` is the default. Removed before that: `push_chunk*`, `push_range*`,
  `requested_ranges`, `pull_batch*`, `is_complete`, `PushStatus`, `PushOutcome`.
- Sink routing: final rows always arrive once each through `provide_next_buffer` in
  increasing `y`; `provide_pass_buffer` receives only provisional rows, only for sinks
  whose `accepts_provisional` (read once, at `begin`) is `true` (`d8b9e636`).

### Added

- `zencodec::source`: `ReadAt` (with `read_at`, short only at the end, `len_hint` and
  `as_slice`), object-safe `DynReadAt` (+ `ReadAtShim`, `Box`/`&mut` impls),
  `SliceReadAt`, `SourceError`, `AccessPattern` (`most_demanding`).
- `DecodeJob::push_decoder_from_source` / `DynDecodeJob::push_decoder_from_source` /
  `CodecSet::push_decode_from_source`: `push_decoder` with the input bound to a
  `DynReadAt` instead of a slice, for pull-native cores. Default rejects with the new
  `UnsupportedOperation::SeekableDecode` (appended after released variants);
  `DecodeCapabilities::seekable` / `with_seekable` declare it.
- `helpers::WholeFileIncremental`: the sans-I/O contract for a codec core that
  needs the complete file. It reports the header as soon as the job's `probe`
  accepts a growing prefix and decodes through the job's `push_decoder` once the
  input is complete, so every codec can serve the zencodec-io drivers (and
  async sources without holding a thread) before it gains a native incremental
  decoder.
- `IncrementalDecode::access_pattern`, `DecodeCapabilities::access_pattern` /
  `with_access_pattern`; `CodecSet`'s incremental decoder reports the most demanding
  pattern of its codecs and probes without a buffer (`d8b9e636`).
- `Encoder::begin(width, height, descriptor)`, `Encoder::pending_output` and
  `Encoder::consume_output`, all provided (`d8b9e636`): an encoder that knows the size
  up front writes its header at once and streams encoded bytes as rows arrive.
  `EncodeCapabilities::streaming_output` / `with_streaming_output` declare it.
- `Encoder::begin` / `DynEncoder::begin` return an `OutputTiming` (new,
  `zencodec::encode::OutputTiming`): `AT_FINISH`, or `streaming(lag_rows)` with the
  most rows the encoder holds before their bytes are pending. Whether output streams
  depends on configuration, image and metadata (JPEG fixed versus optimized tables,
  bottom-up formats, gain-map indexes), so the static `streaming_output` capability
  now means "some configuration streams" and `begin` says whether this encoder does.
  `begin` is also where a configuration that cannot encode from rows rejects with
  `RowLevelEncode`. The default returns `AT_FINISH`. `zencodec-io`'s `EncoderSink`
  keeps the answer (`output_timing()`).
- `EncoderConfig::with_prefer_streaming` / `prefers_streaming` (provided, no-op by
  default): a codec-agnostic request for a configuration whose output streams;
  explicit codec settings win and `begin` reports the result.
- Streaming-output rules written on `Encoder::begin` and in the contract: a second
  `begin` is an error and a failed one leaves the encoder unchanged; `encode` after
  `begin` is an error; the lag is below the image height; `begin` may change
  framing, never pixels; `consume_output` clamps; after a `push_rows`/`finish`
  error a driver that sent bytes aborts its transport; `max_output_bytes` bounds
  the whole file.
- `zencodec::time`: `TimeBase` (reduced seconds per tick), signed `Timestamp` with
  exact cross-clock `compare`/`same_instant`, `rescale` with an explicit `Rounding`
  (`Nearest` rounds halves toward +infinity, `TimingQuantizer`'s rule),
  `checked_add`, exact `duration_until`, and `duration_to_ticks` /
  `ticks_to_duration`; re-exports `FrameDuration`, `TimingQuantizer`, `TimingError`.
  `TimingError::Negative` appended. `AnimationFrame` / `OwnedAnimationFrame` gain
  `pts()` / `with_pts()` (`None` unless the source records a presentation instant).
  `LimitKind` gains `PacketBytes`, `Tracks`, `HeaderBytes`, `TableEntries` and
  `QueueDepth` for container limits. Ported from PR 129's `media/src/time.rs` with
  the tie rule changed to match the quantizer.
- `Encoder::take_output` / `DynEncoder::take_output` (provided; the default copies
  pending bytes and consumes them) and `zencodec::encode::OutputBuffer`, the output
  side of a streaming encoder that codecs had each written separately: consumed
  prefix, compaction, clamped consume, a handed-out count for `max_output_bytes`,
  and a `take_into` that swaps allocations when the destination is empty.
- Animation streaming: `AnimationFrameEncoder::begin(width, height, descriptor,
  frame_count)` returns an `AnimationOutputTiming` (`AT_FINISH` or
  `streaming(lag_frames)`), with `pending_output` / `consume_output` /
  `take_output`, all provided; forwarded by `DynAnimationFrameEncoder`. The rules
  match `Encoder::begin`, counted in frames; a declared frame count is binding.
- `zencodec-media` and `zencodec-media-testkit` (new, unpublished workspace members):
  the media contracts and their conformance checks. Packets, tracks, refcounted frames
  with planar views and an immutable per-frame interpretation, codec sessions with
  explicit backpressure (`send` / `receive`, `end_input`, `reset`, `FusedDecoder`,
  erasure), a sans-I/O `Demuxer` driven by every `zencodec-io` cursor through
  `DemuxStage`, a pull-output `Muxer` with `MuxTiming` / `MuxHint`, time-based seeking,
  `MediaLimits` reporting `LimitKind`, and the animation traits over video
  (`VideoAnimationDecoder`, `AnimationVideoEncoder`, with `RenderFrame` / `CaptureFrame`
  conversion and a `Gray8` identity). The testkit ships a toy container and codec and
  six checks (`check_video_decoder_contract`, `check_video_encoder_contract`,
  `check_demuxer_contract`, `check_demuxer_seek`, `check_muxer_output_honesty`,
  `check_animation_over_video`), each validated on the toy and shown to catch a
  wrapper that breaks its rule. Design: `docs/media-design.md`; rules:
  `docs/media-contract.md`. Supersedes PR 129's `media/` experiment.
- `DecodeRowSink::accepts_provisional` and `provide_pass_buffer` (defaulted): a sink
  opts into progressive passes; final-only sinks are unchanged.
- `zencodec-testkit`: `check_incremental_contract` (windows of 1/7/64 bytes and the
  whole file against one-shot, begin once, rows once in order, `Done` repeats,
  unrequested input ignored, cancellation within one step then `Finished`,
  provisional sinks still get final rows through `provide_next_buffer`, truncation
  is a codec error, step budget) and `check_streaming_output_honesty`, both run by
  `check_all`; the reference codec implements both buffer-free (`e54b4175`).

### Deprecated

- `AnimationFrame::duration_ms()` and `OwnedAnimationFrame::duration_ms()` now
  warn because they truncate fractional milliseconds and saturate long values;
  `duration()` is the exact replacement. Constructors taking whole milliseconds
  remain available and exact for those inputs.
- Published `Metadata::filtered`, `Exif::filtered`, `Exif::to_bytes`, and
  `exif::retain` remain compatible, with zenmeta processing entry points as their
  migration targets. The implementation remains single-owned in zencodec until
  a breaking release can relocate it; basic metadata reading stays available.

### Fixed

- Treat BT.709 primaries with the BT.709 transfer as distinct from the sRGB default when planning an ICC for a target without a CICP carrier (`0e5b941d`).
- Check TIFF IFD offsets and entry arithmetic when detecting DNG/RAW, preventing out-of-bounds access or arithmetic overflow on crafted 32-bit input (`5b253be5`).

- Format sniffing no longer mistakes an ordinary truecolor TGA header for an ICO/CUR directory containing zero images.

- Unpublished `CodecSet::transcode` now refuses known animations, auxiliary
  images and descriptor downgrades with `TranscodeRequiresExplicitHandling`.
  Its documentation states that fidelity and metadata-carrier delivery are
  still best-effort under the published trait contracts.
- PR138 audit: reduce incoming destination-tick fractions before combining
  timing phases; avoid intermediate numerator overflow when carrying a tick.
  Integral ticks no longer fail solely because earlier source clocks produced
  a large phase denominator. Generic/dynamic timing forwarding is checked
  (`4ec35983`).

### Changed

- Unpublished metadata rewrite limits now carry `LimitKind`, including packet,
  cumulative rewrite, carrier-count and EXIF-entry budgets. Existing published
  limit discriminants remain unchanged. Git-pinned planners must migrate their
  string constructors together with the contract revision (`7325cfa9`).

- Inventory growth and coverage validation use fallible allocation; add typed
  allocation failures and `try_children`. Gap filling rejects a part cap before
  changing topology. The part cap is not a total byte-memory budget (`f39865b2`).

- Unpublished rewrite requests are data contracts; validation, cancellation and
  packet-budget execution move to `zenmeta::rewrite::Context`. Completed plans
  validate report bounds against an explicit input length. `PreserveExact` XMP
  retention no longer requires an editor; explicit editing still does.

- Unpublished animation timing now distinguishes ordinary `push_frame_timed`
  (reports the encoded duration) from explicit `push_frame_exact`. Add shared
  cumulative timing quantization and an honest timing capability flag. Existing
  millisecond APIs remain compatible with crates.io 0.1.26; legacy exact callers
  migrate to the explicitly named operation. No video transport is added.

### Added

- Add configuration-level `EncoderConfig::{try_with_fidelity,check_fidelity_target,select_fidelity}` and equivalent erased job selection. Ordered preferences report each rejected target, reject non-finite requests, and isolate candidate configurations. These checks confirm target selection only; actual losslessness remains content-dependent and approximate metric targets do not become measured guarantees. No automatic retry of encode failures (`815fdfdd`).

- Add `try_set_policy` and `try_with_policy` to generic encode/decode jobs, and
  `try_set_policy` to erased jobs. The default admits only an empty policy;
  codecs implement one fallible primitive used by both dispatch paths. Rejection
  leaves mutable jobs unchanged and usable. The reference encoder implements ICC/EXIF/XMP gates for one-shot and
  animation jobs; a generic/erased XMP suppression roundtrip is tested.
  Published best-effort setters remain (`4b5e4d9e`).


### QUEUED BREAKING CHANGES

- Animation sink decoding must return frame duration and index alongside output
  geometry; its current `OutputInfo` result cannot transport timing. Until that
  contract is added, testkit can verify sink pixels but cannot verify sink timing (`919dd678`).


Target for the upcoming coordinated breaking release; these removals and contract
replacements are **planned, not implemented**. The current extraction remains
compatible with crates.io 0.1.26. Implementation and signature review precede any
version bump or publication. [Target boundary and release gates](docs/zenmeta-design.md#next-breaking-release-target).

- Move the owned EXIF editing tree, tag mutation, serialization and category
  filtering out of zencodec into zenmeta, including `Exif::{new,set_artist,
  set_copyright,set_orientation,filtered,to_bytes}`, `exif::retain`,
  `Metadata::filtered` and blob-editing artist/copyright conveniences. Migrate
  direct specialist callers to zenmeta; codec signatures use bytes and
  zencodec-owned request/result contracts. Preserve lightweight orientation/color
  reading and ISO gain-map interpretation without zenmeta (`0f4814dc`, `54040cc3`).
- Replace ambiguous decode policy with independent parser compatibility,
  metadata extraction and content/operation admission. Ordinary decoding accepts
  bounded, unambiguous real-world recovery; strict parsing is explicit. Required
  rendering interpretation cannot be silently disabled by extraction policy
  (`760562a2`).
- Replace silently ignored correctness requirements with validated admission and
  reported outcomes. Unsupported required fidelity, alpha, HDR, metadata or exact
  timing must return typed errors; tuning hints may remain best-effort. Verify
  generic/dynamic and one-shot/streaming/animation parity (`760562a2`, `9f8ad523`).
- Make current pixel color/orientation authoritative at encode time, separate
  from source provenance and descriptive metadata. Replace legacy independently
  authoritative inputs only after a checked migration path exists. Require
  realizable container signaling or an explicit conversion requirement/error;
  never introduce an implicit color conversion (`760562a2`).
- Make rational duration the canonical animation contract, with ordinary
  quantization that reports actual timing and separately requested exactness.
  Retire lossy `duration_ms()` getters and millisecond-only trait requirements
  after consumer migration; retain exact `from_millis` conveniences. Specify
  zero-frame semantics and total plays across codecs; rejected frames must not
  advance timing state. Exactness is never the implicit default (`9f8ad523`).
- Replace capability promises that depend on configuration with effective
  operation admission/queries. Retain static booleans as coarse discovery where
  useful. Cover backend, depth, alpha, animation and gain-map combinations without
  a shared enum of every backend setting (`760562a2`).
- Keep output-byte and animation-duration limits; distinguish output final-size
  limits, incremental enforcement and media-duration admission from wall-clock
  limits. Retire misleading omnibus helpers only after callers migrate. Do not
  delete useful requirements because an old adapter failed to enforce them
  (`760562a2`).
- Remove deprecated threading aliases after consumers use `Sequential`/`Parallel`;
  exact thread counts remain backend-specific. Do not promise a Rayon limit
  governs internally threaded AV1 backends (`760562a2`).
- Remove deprecated ICC/descriptor compatibility helpers after migration to
  canonical zenpixels APIs, including `icc_extract_cicp`, the top-level `icc`
  module, `IccMatchTolerance`, `identify_well_known_icc`, `icc_profile_is_srgb`
  and `descriptor_for_decoded_pixels`. Preserve cheap color queries without
  zenmeta or a pipeline; do not remove `SourceColor::has_hdr_transfer` merely to
  force pipeline use (`760562a2`).
- Remove ambiguous ISO gain-map fraction/format aliases after migration to
  `from_f64_cf` and explicit `Iso21496Format` APIs; keep standardized gain-map
  contracts and small format readers/writers available without zenmeta
  (`760562a2`).
- Separate predicted output information from completed output facts where their
  contracts differ; do not globally rename `OutputInfo` without auditing its
  completed push-decode uses. Remove `ComputeEnvironment::new` only after caller
  migration to `conservative`/`host` (`760562a2`).

Inventory, rewrite and CodecSet are unpublished relative to 0.1.26: correct their
contracts before first release, without labeling that work a released API break.
Inventory stays independent of decoding configuration and default-off in codecs.
No video transport or universal metadata tree is part of this release.

### Added

- Reproducible whole-contract comparison against crates.io 0.1.26, including
  archive/VCS verification, both zencodec feature selections, and executable
  observations of rewrite/color/capability contract gaps. No library API changes;
  source baseline `717a55d7`, report linked above.

- Add `metadata_rewrite` request, report, output-plan and runtime-service contracts
  without a dependency on zenmeta or a codec. Existing published metadata APIs
  remain in place; the new contract is additive relative to 0.1.26 (`a31a9b68`).

- `LimitKind::InventoryParts` and categorization for `InventoryError`: hitting
  the part cap is a recoverable resource failure with its typed cause retained;
  inconsistent maps are internal errors. New variants append after released
  discriminants. Malformed file bytes remain reportable inventory parts (`c1c3292f`).

- Retain the exact animation timing additions from `20310944`, already used by
  PNG main: `animation::FrameDuration`, exact frame accessors/constructors,
  and static/dynamic `push_frame_timed`. Existing millisecond APIs remain.
  This coordinated integration preserves both inventory and animation APIs;
  it adds no video implementation. See `docs/animation-timing.md`.
  New unsupported-operation variants follow all released variants, preserving
  the numeric discriminants checked against crates.io 0.1.26.

- `UnsupportedOperation::Inventory` and an inventory default returning the
  existing `At<CodecError>` envelope, without new bounds on `DecodeJob::Error`.
  The inventory method itself is new relative to crates.io 0.1.26.
- Metadata additions extracted from the metadata portion of PR 130 (source
  `27cfae5d`, including PRs 123–125 and 128): `exif::Ifd`, `exif::EntryRef`,
  `Exif::entries`, and separate color, time-offset, device-ID, camera-owner and
  image-ID queries and `ExifPolicy` fields/builders. Existing type ownership stays
  in zencodec; XMP/audit implementations belong in the new zenmeta crate.

### Changed

- Unpublished inventory callers now receive `Result<Inventory, At<CodecError>>`
  (dynamic dispatch retains `BoxedError`), replacing `Result<Option<Inventory>, _>`.
  Disabled implementations return unsupported; no released inventory API changes.
- EXIF publishing presets retain validated color declarations and remove camera
  ownership independently of creator attribution. Timestamp/camera opt-ins no
  longer automatically retain UTC offsets or device/image identifiers; those have
  explicit categories. These are behavior changes from crates.io 0.1.26 even
  though the non-exhaustive policy's new fields are source-compatible.
- EXIF rewrites drop opaque MakerNotes and restrict Interop retention to validated
  color entries. Byte-exact `retain(..., KEEP_ALL)` remains the preservation path.

### Fixed

- Preserve the HDR diffuse-white luminance anchor when metadata retention keeps
  HDR fields, including `PreserveExact`, `Preserve`, `Web`, and
  `ColorAndRotation`. `Metadata::is_empty` now recognizes an anchor-only record.
  Previously filtering silently removed the anchor and could cause downstream
  color processing to use a different luminance fallback.


### Changed

- **Dependency requirements now span the published minor and the next one.**
  `zenpixels` is `>=0.2.14, <0.4.0` (was `0.2.14`, i.e. `^0.2.14` =
  `>=0.2.14, <0.3.0`); `zencodec-testkit` requires `zencodec >=0.1.27, <0.3.0`
  and `zenpixels >=0.2.10, <0.4.0`. For a `0.x` crate Cargo treats the minor as
  the major, so a plain `"0.2.14"` makes a `0.3.0` release invisible until every
  manifest is hand-edited — which is exactly what the `zencodec 0.1.26` rollout
  cost across every consumer repo. The zenpixels floor is unchanged; testkit's
  floor is raised to 0.1.27 for the new contract APIs.
  The standing rule (current-plus-next, re-derived at each release) and the
  two-copies trap it prevents are documented in `CLAUDE.md`.

### Fixed

- **Pushes to `main` now cancel their superseded CI runs.** `ci.yml` and
  `fuzz.yml` keyed their concurrency group on
  `${{ github.head_ref || github.run_id }}`. `github.head_ref` is populated only
  for `pull_request` events, so on a push it was empty and the group fell through
  to `github.run_id` — unique per run, so no two pushes ever shared a group and
  `cancel-in-progress` could never fire. Now keyed on `${{ github.ref }}`, which
  is set for both event types, so PR cancellation is unchanged and consecutive
  pushes supersede each other. The two workflows keep distinct groups via the
  `${{ github.workflow }}` prefix, so they still never cancel one another.


### Added

- **`inventory` module, `DecodeJob::inventory` / `DynDecodeJob::inventory`** (default
  typed `UnsupportedOperation::Inventory` error) and `DecodeCapabilities::with_inventory`: a byte-exact map of a file's parts, each
  with a `Disposition` saying what the decoder does with it; `validate()` checks the parts tile
  the input, `unconsumed()` lists what no decoder reads. `Part` is read through getters (`range()`, `disposition()`, `label()`, `detail()`, ...) so its storage can change in minor releases. Additive. Guide: `docs/inventory.md`.
- `CodecSet` / `CodecSetError` — multi-codec registry: register decoder /
  encoder configs one line each and get detect→decode, `probe`, push /
  streaming / animation decode, and format-keyed encode through one
  `Send + Sync + 'static` handle whose operations all take `&self` — build it
  once and share it app-wide (`LazyLock` / `OnceLock` / `Arc`). Detection
  derives from the registered decoders in `ImageFormatRegistry::common()`
  priority order (custom formats after, in registration order); set-level
  limits / stop / policies are stamped onto every job; `encode_with(Fidelity)`
  clones the registered encoder template per call; `decode_job` / `encode_job`
  expose the stamped jobs for per-operation control (0edaf64).
- `CodecSet::transcode(input, target, fidelity, metadata_policy, color_policy)`
  — the one-call proxy operation: decode → carry the source's metadata → encode.
  Reads the source's ICC / EXIF / XMP / CICP / HDR / orientation off the decode
  result (via `Metadata::from(&ImageInfo)`, with orientation reconciliation),
  re-embeds under the `MetadataPolicy` (`Web` strips privacy / `PreserveExact`
  keeps all), and applies a `ColorEmitPolicy` for color signaling. Single-image,
  no pixel processing. Precision-preserving: decodes at the source's native
  descriptor and encodes it verbatim when the target accepts it. Known animation,
  multi-image or auxiliary-image sources, and a descriptor the target cannot
  accept, require explicit handling and return `TranscodeRequiresExplicitHandling`.
  There is no implicit lower-precision re-decode or pixel conversion. Fidelity
  and metadata-carrier delivery remain best-effort under the published traits.
- `CodecSet::estimate_encode` / `estimate_decode` — by-format resource
  estimate (peak memory / wall-time / core-scaling) forwarding to the
  registered codec's `estimate_{encode,decode}_resources`, so a `CodecSet`
  covers estimation too; `NoEncoder` / `NoDecoder` when the format has no
  registered codec. `estimate_decode_of(data, compute)` probes the input
  first (format + dimensions) and estimates in the decoder's native output
  format — the bytes-based convenience, as one-call as `probe`.
  `estimate_encode_of(format, pixels, compute)` is the encode analog — it
  reads the dimensions and format straight off the pixel slice you already
  hold, no `ImageCharacteristics` to build by hand.
- `ComputeEnvironment::conservative()` / `ComputeEnvironment::host()` — explicit
  estimate-environment constructors. `conservative()` is the single-core
  baseline (the old `new()` behavior, now explicitly named); `host()` (std)
  detects the running machine (`available_parallelism()` cores +
  `SimdTier::CurrentHost`; RAM left unknown — std has no portable query).
- One-shot provided methods on the config traits: `DecoderConfig::decode`,
  `DecoderConfig::probe`, and `EncoderConfig::encode` — single-line
  config→result use with default job settings (08dabea).
- Job-level one-shot encode: `EncodeJob::encode(pixels)` /
  `DynEncodeJob::encode(pixels)` — configure a job (metadata, limits, policy),
  then encode in one call instead of the two-step `job.encoder()?.encode(pixels)`
  / `job.into_encoder()?.encode(pixels)`. Provided methods (adding them broke no
  implementor); the job-level analog of `EncoderConfig::encode`, which encodes
  with *default* settings.
- `zencodec::prelude` — one-import bundle of every encode/decode trait,
  generic and dyn (0edaf64).
- testkit: `CodecSet` behavior suite against the reference codec — roundtrip,
  registration-scoped detection, custom-format detect→decode, static
  `LazyLock` sharing across threads, template cloning, typed errors (a4fec14).
- testkit: `ReferenceZcrDecoderConfig` + `ZCR_FORMAT` — the reference decoder
  registered under a self-detecting format (matches the codec's own `ZCR1` wire
  magic) instead of `ImageFormat::Pnm`. Drop-in for `ReferenceDecoderConfig` so
  detection-based decode (`decode` / `probe` / `estimate_decode_of`) works end to
  end; removes the ~35-line custom-format scaffolding each detect→decode test and
  example previously hand-rolled.
- testkit: `tests/usage.rs` — worked, runnable examples of the common
  `CodecSet` usages (encode / decode / probe / app-wide sharing / fidelity +
  metadata via the job's direct `encode` / pixel-format request / strip
  streaming / estimate) plus two `zenpixels-convert`-assisted paths: adapting a
  foreign pixel format into an encoder's supported set (`adapt_for_encode`) and
  converting a decoded buffer to a caller-chosen format (`convert_to`).

### Deprecated
- `ComputeEnvironment::new()` — construction should be explicit. Use
  `conservative()` (the single-core baseline, identical behavior) or `host()`
  (std, detect the running machine). Removal is queued for the next 0.x minor.

### Added
- CI: a `Fuzz` workflow that compile-gates the `fuzz/` workspace on every push
  and PR. `fuzz/` is excluded from the root manifest so `cargo fuzz` can resolve
  it standalone, which also meant no CI job ever compiled its five targets —
  they could rot silently against an API change and nothing would say so. The
  gate runs `cargo check --all-targets` on stable rather than `cargo fuzz build`
  (nightly + per-target sanitizer codegen) because every rot found in the
  workspace-wide audit was a plain type or resolution error. A second job
  replays the committed crash seeds. The five targets did still compile when the
  gate was added.

- Docs: `docs/IMPLEMENTING.md` "Fidelity: honour what you can, report what you
  did" — the `with_fidelity` / `resolved_target_fidelity` contract per declared
  capability (the table `check_fidelity_honesty` enforces) + a checklist entry
  (#26).
- Docs: `ImageInfo::is_progressive` now has a definition (a *refinement* order —
  a coarse whole image renderable before the last byte — never a spatial
  partition) and a per-format table settling the open design call from the
  decoder audit: JPEG SOF2/6/10/14, Adam7 PNG, interlaced GIF, a JPEG XL frame
  with `num_passes > 1`, and a layered (`a1lx`) AVIF/HEIC item are progressive;
  WebP, TIFF (strips/tiles) and the raster formats never are.
  `docs/IMPLEMENTING.md` gains a "Source Colour Fields" section (authority per
  format, header-sourced `bit_depth`/`channel_count`, the progressive rule) and
  matching checklist entries (#11).
- Docs: the decoded-buffer `ColorContext` convention — decoders SHOULD attach
  `SourceColor::to_color_context()` to every emitted buffer, class-gated (ICC
  device class ↔ buffer colour model), with the ranked fallbacks (class-matching
  profile rides as-is → derivable profile becomes a CICP-only context → keep a
  layout the profile describes), synthesized descriptions for derived outputs,
  and re-attach-at-every-emission. Carrier table (descriptor / context /
  `source_color`) settles who owns which axis. `docs/IMPLEMENTING.md` "Colour on
  the Decoded Buffer" + `docs/correctness-model.md` (#25).

### Fixed
- `AnimationFrame::to_owned_frame` dropped the frame's `ColorContext` when
  copying pixels into the owned buffer, so the default `render_next_frame_owned`
  path yielded context-free frames while the borrowed path carried one. The copy
  now carries the context (#25).
- Docs: `ColorEmitPolicy`'s "Lowering the plan" rustdoc and
  `docs/color-emit-model.md` claimed the pixel-encode path lowers a
  `ColorEmitPlan` "through `zenpixels_convert`'s atomic
  `finalize_for_output_with`". It does not — no codec or pipeline crate calls
  it. Both now describe the real per-codec, by-convention lowering and state
  plainly that the caller-supplied `Metadata` is never cross-checked against
  the pixel buffer (#119).
- `tests/fuzz_regression.rs` could pass vacuously: it returned early when
  `fuzz/regression/` could not be read and only *printed* the replay count, so a
  renamed, moved, or emptied seed directory reported green while replaying
  nothing. It now hard-fails on an unreadable directory and asserts every seed
  named in `EXPECTED_SEEDS` is present — the 12 committed seeds each guard a
  fixed bug, so retiring one is a deliberate edit rather than a silent loss of
  coverage. Verified by hiding a seed and watching the suite fail.

## [0.1.26] - 2026-07-14

### Added
- **`Fidelity::zensim_b` / `LossyTarget::ApproxZensimB(f32)`** — a fourth
  one-shot lossy target, aiming at a zensim score (≈0–100, higher is better) in
  a single calibrated pass. Pinned to `zensim::ZensimProfile::B` specifically
  (the deterministic linear generation-B profile — a closed-form lasso fit,
  byte-reproducible with no training seed) rather than "whichever zensim
  profile is latest," so the target stays stable across zensim patch releases
  the same way `ApproxSsim2` / `ApproxButteraugli` do. zensim was previously
  noted as deferred ("no reliable metric yet") pending exactly this kind of
  deterministic, non-MLP profile; `B` (shipped in zensim 0.3.0) is what
  unblocks it. Additive: new enum variant on the already-`#[non_exhaustive]`
  `LossyTarget`, new constructor, no signature changes. The default
  `with_fidelity` fallback treats it like `ApproxSsim2` (passes the raw 0–100
  score through to `with_generic_quality`) since zencodec has no built-in
  zensim-to-generic-quality calibration curve — codecs override to honor it
  natively, same as the other metric targets.
- **zencodec-testkit: error-envelope conformance + whereat-trace checks** —
  `check_decode_error_envelope` (runtime: drives a decoder through dyn-dispatch
  erasure on rejected input, asserts the `ErrorCategory` *and* codec name survive
  to the `BoxedError`) and `assert_uses_codec_error_envelope` (compile-time,
  zero-cost: bounds a codec's encode/decode `type Error` to `At<CodecError>`, so a
  Pattern-A native-enum error type is a *compile error*). The testkit's `reference`
  codec is a deliberate Pattern-A foil that **fails** the runtime check — proving
  it has teeth — while `minimal` (the envelope exemplar) passes both. Opt-in; not
  in `check_all` (the foil must stay free to be Pattern A). Adds trace coverage
  too: a codec error's `At` trace preserves every `.at()` hop's file:line up the
  stack and through `BoxedError` erasure (no frame lost or collapsed, each
  attributable to its crate via `Location::file()`), and `At::at_crate` crate
  boundaries are recorded and surfaced by `full_trace()`.
- **Unified error taxonomy: `ErrorCategory` enum + `CategorizedError` trait** —
  a codec-agnostic way to classify any codec error for routing (HTTP status,
  retry, logging) without naming the concrete enum (zencodec#99, superseding the
  cancellation-only sketch). `ErrorCategory` (`#[non_exhaustive]`, `Copy`) is
  **origin-first, two-level**: `Image(ImageError)` (the bytes are the problem —
  `Malformed` / `UnexpectedEof` / `Unsupported(UnsupportedImageKind::{Type,
  Feature})`), `Request(RequestError)` (the caller's request is the problem —
  `Invalid(InvalidKind::{Parameters,Buffer,State})` /
  `Unsupported(UnsupportedOperation)` / `CmsRequired`), `Resource(ResourceError)`
  (`Limits(LimitKind)` / `OutOfMemory`), `Policy(PolicyKind)` (valid input a
  configured policy declined — `PolicyKind::{Decode,Encode}` mirrors the crate's
  existing `DecodePolicy`/`EncodePolicy` split, so the call site already knows
  which one), `Stopped(enough::StopReason)` (stopped via the `Stop` token — the
  reason *is* the payload, no re-derived enum), `Io(CodecIoKind)`, and
  `Internal(InternalKind)` (`InternalKind::Bug` — a broken invariant in the
  codec's own logic, never retryable — vs `InternalKind::Dependency` — an
  unclassified error surfaced from a sub-component/foreign library; both still
  mean "500" for routing, the split is for telemetry/triage: it lets a generic
  consumer that only carries the category forward, without a per-codec downcast,
  tell "we have a code defect" from "we have a classification gap" from
  production traffic). The set is distilled from an inventory of every zen codec's
  error enum, then reshaped so the top level answers "who owns the fault" before
  any sub-kind: `Image` is untouchable by the caller (a *different* codec might
  handle the bytes); `Request` is the caller's to fix. This is also what makes
  the whole `Image(_)` arm exactly the incomplete-input-tolerable set for
  `zencodec-testkit::check_decode_truncation_series` (zencodec#112) — one
  `matches!(cat, ErrorCategory::Image(_))` instead of enumerating four flat
  variants. Sub-enums carry `From` shortcuts into `ErrorCategory` (and the leaf
  kinds `UnsupportedImageKind` / `InvalidKind` / `LimitKind` shortcut into their
  parent too), so a codec's `category()` arm reads `ImageError::Malformed.into()`
  rather than spelling the outer wrapper. `CategorizedError: Any { fn
  codec_name(&self) -> Option<&'static str>; fn category(&self) -> ErrorCategory
  }` is **opt-in** (not blanket-impl'd, not added to the `Error` bound), so
  adopting it is additive and back-compatible. The `Any` supertrait keeps a `dyn
  CategorizedError` downcastable (both methods take `&self`, so the trait is
  dyn-compatible); `codec_name()` is required, returning `None` for the
  non-codec cause types. A blanket impl forwards through `whereat::At<E>` so a
  located error keeps its category; zencodec's own cause types implement it
  (`LimitExceeded` → `Resource(Limits(kind))`, `UnsupportedOperation` →
  `Request(Unsupported(self))`, `enough::StopReason` → `Stopped(self)` — a
  direct passthrough, not a lossy remap), making a codec's mapping a one-line
  delegation per arm.
  Adds `LimitKind` (the value-free discriminant of `LimitExceeded`, via
  `LimitExceeded::kind()`) plus two structural variants with no corresponding
  `LimitExceeded` payload: `Scans` (a progressive-decode scan-count ceiling) and
  `DecompressionRatio` (a compression-bomb ceiling, security-relevant — distinct
  from an absolute `Memory` cap). No new dependencies. The testkit `reference`
  codec adopts the trait; `reference_cancellation_is_classifiable` proves
  end-to-end classification through a real cancelled operation. The category set
  is backed by a per-codec error inventory + right-sizing analysis in
  `docs/error-taxonomy-inventory.md` (with `docs/error-types-ecosystem.md`
  surveying the surrounding crates) — both carry a superseded-shape note pointing
  at this reshape and `docs/spec.md` for the current form.
- **zencodec-testkit: `check_decode_truncation_series` EOF/truncation conformance
  check** (zencodec#112) — feeds a known-good encoded image, builds a
  deterministic series of truncation lengths (small absolute header sizes unioned
  with fractions of the full length), and for each prefix runs a **full decode
  through the dyn-erased boundary** (`DynDecodeJob::push_decode` into a throwaway
  sink — so pixel-body truncation is exercised, not just what a `probe` reads).
  Asserts every resulting `ErrorCategory` is in the allowed incomplete-input set
  via the new `is_incomplete_input_category` (see the taxonomy reshape above —
  the whole `Image(_)` arm). Catches the real bug class where a codec reads a
  length field out of truncated data and OOMs, or funnels a truncation into
  `Internal` (5xx for what should read as a 4xx client error) — exactly the
  misattribution the origin-first taxonomy exists to make checkable in one line.
- **`CodecError` envelope + `CodecErrorExt::{codec_error, error_category}`** — the
  carrier that makes a category (and the originating codec) survive **type
  erasure**, the gap `CategorizedError` alone can't close (after erasure you hold a
  `dyn Error`, not a `dyn CategorizedError`). A codec returns it as
  `whereat::At<CodecError>` — a single *concrete* type, so a consumer recovers the
  category **and** the codec name from any `Box<dyn Error>` / `anyhow::Error` /
  mapped wrapper.
  - **Register-fittable.** `CodecError` is a one-word handle (`Box<Repr>`), so
    `At<CodecError>` is two words (16 bytes) and `Result<_, At<CodecError>>` is two
    words too (the box pointer's niche absorbs the discriminant) — returned in
    registers, not spilled to the stack. The box is what makes that possible: the
    detail is a *fat* `Box<dyn Error>` (16 bytes alone), so an unboxed envelope
    would exceed the 16-byte ABI threshold and spill regardless of how thin the
    other fields are. Trade: one cold-path alloc per error; `new` is no longer
    `const`. A word-relative `error_types_stay_small` test guards it.
  - Constructors: `new(codec, category)` (complete error, **no detail**),
    `from_native(detail)` (bare envelope; reads *both* the category and the codec
    name from the detail's `CategorizedError`), `of(located: At<E>) -> At<CodecError>`
    (the located form — maps the inner error via whereat's trace-preserving
    `map_error`, keeping the `At` on the outside), `from_parts(codec, category,
    boxed)`, and `with_codec(self, Option<&'static str>)` — a builder to stamp or
    clear the codec name on an envelope built without one. Accessors `category()` /
    `codec() -> Option<&'static str>` +
    `detail() -> Option<&dyn Error>`. `Display` = `"{codec}: {detail-or-category}"`
    (adds a `Display` impl on `ErrorCategory`); `source()` = detail when present.
  - **`of` forces location at the type level**: it takes an already-located `At<E>`,
    so a codec that skipped whereat can't call it — the omission is a compile error,
    not a silently trace-less error. (`from_native` + whereat's
    `map_err_at(CodecError::from_native)` converts a bare `Result<_, E>` in one step.)
  - The codec name lives on the trait as the **required** `codec_name(&self) ->
    Option<&'static str>` method — no default, so every implementor answers it; the
    cause types return `None`. It is a `&self` method rather than an associated const
    precisely so the trait stays **dyn-compatible**: with the new `Any` supertrait a
    `dyn CategorizedError` can be formed *and* downcast to its concrete type (an
    associated const forbids the trait object outright). `from_native`/`of`
    `debug_assert` the name is `Some`, so building an envelope from a bare shared
    cause type (e.g. `UnsupportedOperation`) fails loudly in dev; wrap such causes in
    your codec's error type first.
  - **`ErrorCategory::Io(CodecIoKind)`** carries a `std::io::ErrorKind` under the new
    opt-in `std` feature, empty under `no_std` (stable variant shape either way, so
    matching `Io(_)` is portable), anticipating a future `core::io::ErrorKind`.
  - `CodecErrorExt::codec_error() -> Option<&CodecError>` recovers the envelope by
    downcast (`At<CodecError>` or a bare `CodecError`); `error_category()` is the
    shortcut. Both defaulted (additive). Recovery also tolerates a single
    consumer-applied `Box` layer (`Box<At<CodecError>>` / `At<Box<CodecError>>` /
    `Box<CodecError>`).
  - Adoption is one impl per codec — `impl From<MyError> for At<CodecError>` (body
    `CodecError::of(e.start_at())`, with `fn codec_name(&self) { Some("mycodec") }`
    on the error's `CategorizedError` impl) — after which `?` auto-wraps existing
    fallible internals; reject stubs use `Unsupported<At<CodecError>>`.
  - The testkit `minimal` codec is converted to this pattern;
    `minimal_envelope_category_survives_dyn_erasure` proves the category **and the
    codec name** are recovered after the dyn shim erases the error to `BoxedError`
    (and that the native-enum `reference` path returns `None` once erased — the
    contrast that motivates the envelope).
  - All additive vs 0.1.25 (which has neither type) — these are new items; a
    `cargo-semver-checks` run gates the eventual release (zencodec#99, zencodec#103).
- **`StreamOffset(pub u64)`** — a shared, downcast-able newtype for the one piece
  of context worth carrying generically: *where in the encoded input* a decode
  failed. A codec attaches it to its `At<CodecError>` trace with `.at_data(||
  StreamOffset(pos))` (rather than a per-locus `CodecError` field); a generic
  consumer recovers it via `err.contexts().find_map(|c|
  c.downcast_ref::<StreamOffset>())` — even after erasure to `Box<dyn Error>` — with
  no codec-specific type named. Per-variant detail (dims, expected/actual, table
  indices) stays in the native error; richer loci (marker / box fourcc / NAL /
  frame index) ride the same `at_data` channel, with only the byte offset
  standardized as a type today. Additive.
- **Re-export `whereat::{At, ErrorAtExt, ResultAtExt}`** (and `pub use whereat;`) at
  the crate root, so a codec can name `zencodec::At<zencodec::CodecError>` — the
  recommended `Error` type — and reach `start_at()` / `.at()` without a direct
  `whereat` dependency. No new coupling: `At` was already in the public API via the
  blanket `impl CategorizedError for At<E>`. Additive.

- **Gain-map encode** on `EncodeJob`: fallible `with_gain_map_pixels` /
  `with_gain_map_encoded` returning the codec's own `Self::Error` (reusing the owned
  `DecodedGainMap` / `GainMapSource`). The default rejects with the new
  `UnsupportedOperation::GainMapEncode` — so a codec that can't embed fails *loudly*,
  not silently; codecs that can embed validate the gain map and return their own error
  (e.g. an invalid channel count) for a bad one. Additive (no trait break): the default
  builds `Self::Error` via the encoder's `reject` under a method `where` clause.
- `GainMapInfo::bit_depth` (+ `with_bit_depth`, default 8) for 10/12-bit AVIF/JXL gain
  maps; `DecodedGainMap::{width,height,channels}` accessors (buffer-derived, so the
  1-vs-3-channel encode decision can't desync from the pixels).
- Research: gain-map re-compression rate–distortion study fixing the fidelity default
  at the **~q90 SSIM2 knee** — `docs/gainmap-fidelity-rd-2026-06.md` (+ raw data under
  `benchmarks/gainmap-fidelity/`).

### Changed
- Docs: README overhauled to the shared zen\* README conventions — CI badge drops the
  `branch=` param and gains a `label`, an MSRV (1.88) badge is added, a `## Quick start`
  with a `[dependencies]` block leads, in-repo doc links are absolutized, and the canonical
  crosslink footer is rendered. The crates.io README is now a generated, badge-free
  `README.crates.md` (`readme = "README.crates.md"`; `include` ships it instead of `README.md`).

### Fixed
- **32-bit (i686/wasm32) EXIF parse could panic in debug builds** — `rd16` /
  `rd32` computed `offset + 2` / `offset + 4` with unchecked arithmetic on
  offsets that come straight from attacker-controlled u32 fields (e.g. the
  TIFF header's `ifd0_off`); on 32-bit targets an offset near `u32::MAX`
  overflows `usize` — `attempt to add with overflow` under
  `debug_assertions`, silent (behaviorally-benign, still-rejecting) wrap in
  release. Unreachable on 64-bit, so the all-64-bit fuzz farm could never
  find it: caught by the i686 CI lane replaying the
  `metadata_filtered_issue_111_farm_artifact` regression seed within an hour
  of that seed landing. Both helpers now use `checked_add` (reject, not
  overflow); the regression seed stays as the permanent guard, and
  `tests/fuzz_regression.rs` now names each seed as it runs so a failing
  seed is identifiable from CI output alone.
- `CodecIoKind`'s doc comment linked to the `std`-gated `kind` accessor
  (`[`kind`](Self::kind)`) from always-compiled text, so a `no_std` doc build —
  which is what docs.rs runs, since `std` is not a default feature — hit an
  unresolved intra-doc link. Gate the linking doc line to `std`, and add
  `[package.metadata.docs.rs] all-features = true` so docs.rs builds with `std`
  and documents the `kind()` accessor + `From<std::io::*>` impls.
- EXIF duplicate-pointer dedup was gated on whether extraction *resolved* a
  usable sub-IFD (`gps_ifd.is_some()`/`exif_ifd.is_some()`) rather than on
  whether `take_pointer` *removed* an entry from IFD0. A GPS/Exif pointer tag
  whose value was offset-shaped but failed to parse as a sub-IFD left a stale
  duplicate entry behind; a rewrite then relocated that entry's out-of-line
  bytes to a valid offset, and the next parse resolved it as a real sub-IFD —
  flipping presence (`None` → `Some`) purely from being rewritten. Same
  file:line:col signature as the closed zencodec#30, so the fuzz farm's
  sig-dedup ledger silently swallowed ~10k fresh crashes as "already known."
  Fix: gate the duplicate sweep on `*_taken.is_some()` instead of
  `*_ifd.is_some()`. Regression:
  `fuzz/regression/exif_roundtrip_gps_drift_dup_unresolved_pointer`. (2ed32cf7)
- `Metadata::filtered`'s orientation-reconcile path (`retain_reconciled` /
  `set_orientation_with`) could corrupt structural TIFF bytes when the source
  Orientation tag had a malformed `count` (fuzzer-found: 256 or 3). The
  in-place patch reuses the parsed entry's `value_offset`, which is safe only
  when the value is inline (well-formed `count == 1`); a malformed count makes
  the value out-of-line, turning the offset into an attacker-controlled `u32`
  that can point anywhere in the blob — observed corrupting the TIFF header's
  `ifd0_off` and, separately, a sibling entry's descriptor. Broke
  `retain_reconciled`'s idempotence contract (same policy applied twice
  produced different bytes). Same file:line:col signature as the closed
  zencodec#96/#97, so the fuzz farm's sig-dedup ledger silently swallowed the
  recurrence (~2,421 `metadata_filtered` + ~22,017 `exif_author` crashes) for
  2.5 weeks. Fix: refuse the in-place patch unless the value is provably inline
  (`count * type_size <= 4`), matching the existing non-integer-carrier
  fail-safe. Regressions:
  `fuzz/regression/metadata_filtered_orientation_entry_overlap`,
  `fuzz/regression/metadata_filtered_orientation_header_overlap`. (bea2f94c)
- A *duplicate* Interop (0xA005) pointer tag inside the Exif sub-IFD was never
  swept the way duplicate GPS/Exif sub-IFD pointers are (zencodec#30/#107):
  `take_pointer` only ever removed the FIRST occurrence, so a second 0xA005
  entry survived as ordinary data and got consumed one occurrence at a time on
  each subsequent parse — `exif_ifd`'s entry count silently shrank by one on
  every round-trip until empty. Invisible to `orientation`/`has_gps`/
  `has_thumbnail` (nothing reads Interop content), so no existing fuzz target
  assertion could catch it; found by code audit while investigating the
  fuzz-farm-filed zencodec#114/#115 (which turned out to already be fixed —
  see their closing comments, not a new bug). Fix: apply the same "gate the
  duplicate sweep on removal" pattern to Interop. Regression:
  `fuzz/regression/exif_roundtrip_dup_interop_pointer`.
- zencodec#113 (fuzz-farm "RECURRED" filing of the `exif_author` fixpoint
  signature `49851dd8d959a652`, x86_64 — previously #96/#108) does NOT
  reproduce on current main: replayed all 15,773 x86_64 + 6,276 arm64 mirrored
  crash artifacts through the `exif_author` logic in release and
  debug-assertions+overflow-checks builds, and the exact meta.json-cited
  artifact plus both full mirror directories through the real libFuzzer
  binary (`-runs=0`) — zero failures. Root cause of the false filing: R2
  shows the x86_64 farm box uploaded nothing after 2026-07-02T00:27Z (its
  binary predates the 2ed32cf7 fix — the BuildId in #113's trace matches the
  07-02 meta.json), but a local mirror re-sync on 07-12 refreshed file
  mtimes, and triage-crashes.sh's re-open heuristic compares the *local
  mtime* against the prior issue's closedAt — its "s5cmd sync preserves the
  R2 object's timestamp" design assumption is false (s5cmd v2.3.0 has no
  timestamp-preserve option). The heuristic now queries the R2 object's
  actual `LastModified` (fixed workstation-side in zenfuzz-farm, outside
  this repo). Cited artifact kept as a permanent regression seed:
  `fuzz/regression/exif_author_issue_113_farm_artifact`.
- zencodec#111 (the `metadata_filtered` idempotence recurrence of #97, fixed
  by bea2f94c and filed as its documentation record) verified CLOSED against
  current main: replayed all 1,855 x86_64 + 673 arm64 mirrored crash
  artifacts — including the ones the still-pre-fix farm box kept uploading
  until 11:10Z, two hours *after* the fix landed — through the
  `metadata_filtered` logic in release and debug-assertions+overflow-checks
  builds, plus the exact meta.json-cited artifact and both full mirror
  directories through the real libFuzzer binary (`-runs=0`): zero failures.
  Closed a coverage gap found during verification: `tests/fuzz_regression.rs`
  never mirrored the `metadata_filtered` target, so the
  `metadata_filtered_*` regression seeds added by #97/bea2f94c were not
  exercising `Metadata::filtered` idempotence on stable CI — added a
  `run_metadata_filtered` mirror so every seed now runs through it. Cited
  artifact kept as a permanent regression seed:
  `fuzz/regression/metadata_filtered_issue_111_farm_artifact`.

## [0.1.25] - 2026-06-23

### Added
- **`Fidelity` API** (`encode::{Fidelity, LossyTarget}` +
  `EncoderConfig::with_fidelity` / `resolved_target_fidelity`):
  a single encode-fidelity request. Two variants ship:
  - **`Lossless`** — mathematically exact.
  - **`Lossy(LossyTarget)`** — a one-shot target: `ApproxSsim2(score)`,
    `ApproxButteraugli(distance)` (max-norm), or `CodecSpecificQuality(q)`.

  `with_fidelity` defaults to bridging the legacy `with_lossless` /
  `with_generic_quality` setters, so every codec behaves sensibly today; codecs
  override to honor targets natively, and `resolved_target_fidelity` reports what
  was actually honored. Scope is **blind single-pass**. A fail-fast
  `try_with_fidelity` → `FidelityMatch` (raised/lowered/translated/unsupported
  verdict) is **designed but deferred** — its content-dependent semantics aren't
  settled (a config-time verdict can't express GIF's content-dependent lossless or
  the HDR/bit-depth question); preserved as a reserved block in `src/fidelity.rs`
  + imazen/zencodec#104.

  A third variant — **`LosslessMode`**: lossless *coding* (predictive, no DCT
  ringing) of pixels pre-quantized within a budget, which would make the
  *container* the variant and open the screen-content path (crisp + small in a
  lossless container) — is **designed but deferred** until its budget model (L∞
  vs perceptual) is concrete. The design is preserved as a reserved block in
  `src/fidelity.rs` + `docs/near-lossless-design.md`. Closed-loop targeting, a
  standardized generic-`Quality` arm, and the butteraugli **3-norm** are likewise
  reserved as commented names so they add later without renaming. Additive — the
  two trait methods have default impls (no major bump). (#12)

### Documentation
- Clarified `ResourceLimits::max_memory_bytes` and the `enforces_max_memory`
  capability: documented the two enforcement strengths the flag spans — **live
  cumulative tracking** (every significant allocation charged against a running
  budget with RAII release; byte-accurate; only a minority of codecs) vs
  **pre-flight estimate** (peak predicted from the header and rejected before
  allocating; most codecs) — and that `enforces_max_memory == false` means the
  codec does not guard memory at all (bound such input via `max_pixels` /
  `max_width` / `max_height`).

### Fixed
- Fixed a broken intra-doc link to the deferred `NearLosslessBudget` (fidelity
  refactor leftover) that failed `cargo doc -D warnings`.

## [0.1.24] - 2026-06-21

### Added
- **`AllocPreference`** (`#[non_exhaustive]` enum — `CodecDefault` / `Fallible` / `Infallible`) +
  `ResourceLimits::prefer_fallible_allocations` field + `with_prefer_fallible_allocations(..)` builder:
  a per-request caller preference for whether a codec sizes untrusted-input buffers via the fallible
  `try_reserve` path (graceful OOM) or the faster infallible `vec!` path. `CodecDefault` (the default)
  lets each codec choose and preserves existing behaviour; `for_untrusted_input()` now presets
  `Fallible`. Additive — the policy travels with `ResourceLimits`; codecs consume it in a follow-up.
- `estimate::SimdTier` (`Unknown`/`CurrentHost`/`Wasm`/`Wasm128`/`Neon`/`X86V1`–`X86V4`, archmage /
  `x86-64-vN`-aligned) + `ComputeEnvironment::with_simd_tier` / `simd_tier()` —
  an optional SIMD-tier hint on the compute environment, so a caller that
  detects a tier (e.g. via archmage tokens) can pass it for tier-aware
  estimates. Self-contained (no archmage dependency); the doc shows the
  trivial token→tier mapping.
- `DynEncoderConfig::estimate_encode_resources` /
  `DynDecoderConfig::estimate_decode_resources` — the dyn-dispatch wrappers now
  forward the resource estimate, so codec-agnostic `&dyn DynEncoderConfig`
  callers get the same `ResourceEstimate` as the generic path. Both default to
  `ResourceEstimate::unknown()`, so adding them stays semver-additive (no major bump).
- **Unified resource estimation** (`estimate` module): `ResourceEstimate`
  (all-`Option` fields — estimated + upper-bound peak memory, `wall_ms`,
  `cpu_ms` (all `u64`), and threading — with an `unknown()` all-`None` value for
  codecs that do not model resources),
  `ThreadingInformation` (CPU-core scaling — `max_efficient_threads: Option<u32>`,
  the knee of the scaling curve; `None` = parallel with an unknown knee that
  scales to all cores — with `effective_threads(cores)`), and two sealed/growable
  (`#[non_exhaustive]`, private-fields + accessors) builder inputs:
  `ComputeEnvironment` (hardware + conditions of computing — cores now;
  RAM/SIMD/load expandable) and `ImageCharacteristics` (dimensions +
  `PixelDescriptor` + frame count; content class/HDR tier expandable). Default
  trait methods `EncoderConfig::estimate_encode_resources` and
  `DecoderConfig::estimate_decode_resources` take
  `(&ImageCharacteristics, &ComputeEnvironment)` and return `unknown()` by
  default; codecs override with their calibrated `heuristics` (or
  `ResourceEstimate::conservative(image).at_cores(cores)`).
  `ResourceEstimate::at_cores` rescales wall time (linear to the knee); peak and
  CPU time are unchanged. All four structs are accessor-only with full names
  so they grow without breaking callers. Additive — existing impls keep compiling.

## [0.1.23] - 2026-06-18

### Fixed
- **Construct the empty `ColorContext` via `ColorContext::default()`** instead of
  a struct literal — zenpixels 0.2.14 sealed `ColorContext` as
  `#[non_exhaustive]`, so the no-signaling fallback `ColorContext { icc: None,
  cicp: None }` no longer compiles. `default()` is the no-arg constructor for
  that case; bumps the `zenpixels` dependency to `^0.2.14` (`default()` was
  added in 0.2.14).
- EXIF authoring/serialization is now a true fixpoint for a dangling thumbnail
  pointer (fuzz zencodec#96). A thumbnail offset/length pair pointing out of
  bounds wasn't captured but was kept as data entries in IFD1; a rewrite
  relocated the IFD so the stale offset landed in-bounds, and the next parse
  captured a phantom thumbnail (`out != out2`, tags flipping SHORT→LONG, output
  inflating). The thumbnail offset/length tags are now stripped on parse
  unconditionally (they're structural — synthesized by `to_bytes` only when a
  thumbnail is actually present), mirroring the sub-IFD pointer handling.
  Regression: `exif::tests::dangling_thumbnail_pointer_does_not_survive_96` +
  `fuzz/regression/exif_author_fixpoint_96`.
- EXIF `parse → to_bytes → parse` no longer drops GPS/thumbnail presence when the
  source carries a duplicate sub-IFD pointer tag (fuzz zencodec#30). A second
  `0x8825`/`0x8769` entry left in IFD0 after the real pointer was extracted is now
  stripped on parse (only when that pointer is synthesized on write), so it can't
  shadow the synthesized pointer on re-parse. A short/unusable pointer that wasn't
  extracted is still preserved as data.
- `Metadata::filtered` is idempotent again under orientation-reconciling policies
  (`ColorAndRotation`, etc.) (fuzz zencodec#97). The reconcile path now sets the
  orientation entry's `count` to 1 when it rewrites the value; a malformed source
  `count` left next to a 1-element value serialized as a dangling out-of-line
  offset that re-parse dropped, so `filtered` went `Some` → `None`.

### Changed
- `ResourceLimits::for_untrusted_input()` raises `max_pixels` from 100 MP to
  120 MP so common ~108 MP camera photos pass the recommended untrusted-input
  policy (`max_total_pixels` stays 200 MP). No API change.
- Doc cleanup to match that 120 MP default: the `ResourceLimits` example now
  uses `with_max_pixels(120_000_000)`, and the `for_untrusted_input()` doc
  comment now states the `max_pixels` cap as 120 MP (it still claimed 100 MP).
  Docs only — no behavior change.

### Added

- **`exif_author` fuzz target** — drives the EXIF *write* API
  (`Exif::new` / edit-after-parse + `set_orientation` / `set_copyright` /
  `set_artist`) and asserts authored output always parses, reads back the
  set values faithfully (NUL-truncation semantics included), and is a
  canonical serializer fixpoint. Mirrored as `run_author` in
  `tests/fuzz_regression.rs` so regression seeds replay on stable. Smoke
  run: 11.6M execs / 90 s, zero findings.

### Documentation

- README "Untrusted input" section now shows the **server-critical surface
  worked end-to-end**, not just named: how to construct `ResourceLimits` (preset
  / `none()` + `with_*` / direct public-field set), the units of every field, the
  unlimited-by-default caveat, how to build and fire a `StopToken` from another
  thread via `almost_enough::Stopper`, and — the gap an insulated external-dev
  usability test flagged — a full decode-untrusted-bytes example that actually
  **attaches** both to the job (`job().with_limits(..).with_stop(..).decoder(..)`).
  Also pins the `with_generic_quality` scale as `0.0..=100.0` in the Quick Example.

### Changed

- Public-API snapshots migrated to the shared `zenutils-apidoc` (format v3:
  three disjoint files — supported surface / feature additions / hidden;
  trait rosters; auto-trait exception encoding). The 150-line local
  generator became a 3-line shim; the `serde_json` dev-dependency and CI's
  `cargo-public-api` binary install are gone. zencodec-testkit's surface is
  now snapshotted too (it is publish-ready per its manifest). The tooling
  is local-only by construction: the dependency lives in the
  workspace-excluded `apidoc/` runner package, so plain `cargo test` and
  every CI job (including `--all-features` ones) never compile the apidoc
  tree or run rustdoc builds — regenerate via `just api-doc` / `just fmt`.

## [0.1.22] - 2026-06-11

### Added

- **`Exif::set_orientation(Orientation)`** — insert-or-replace the IFD0
  Orientation tag, completing the from-scratch authoring path
  (`Exif::new` → `set_orientation` / `set_copyright` / `set_artist` →
  `to_bytes`). An existing SHORT/LONG entry keeps its TIFF type; a malformed
  non-integer carrier is replaced by the canonical 1-count SHORT; a tag-less
  blob gains one. Covers the "inject Orientation / stamp Copyright" pipeline
  case from #18 and the field set zenjpeg's `ExifFields` builder writes.
  Additive.
- **`OrientationHint::bakes() -> bool`** — the gate codecs use to choose between
  the preserve path (leave pixels in stored orientation, report stored dims + the
  intrinsic `Orientation` tag) and the bake path (transform pixels, report display
  dims + the applied orientation). `true` for everything except `Preserve`.
  Replaces the identical `will_auto_orient` match each codec adapter was
  hand-rolling. Additive.

## [0.1.21] - 2026-06-09

### Added

- **`zencodec-testkit` crate (new workspace member, unpublished)** — a
  conformance harness codec crates run against their own `EncoderConfig` /
  `DecoderConfig`. Ships `check_metadata_no_leak` (a retention policy must never
  leak what it discards — decodes the output and re-parses the embedded EXIF to
  prove GPS/thumbnail/rights the policy dropped are actually gone),
  `check_cross_path_pixel_equivalence` (one-shot vs `push_rows` vs streaming vs
  push-sink produce identical pixels), `check_animation_cross_path_equivalence`
  (the three animation decode paths — borrowed, owned, push-sink — yield identical
  frames, where canvas-aliasing/frame-ordering bugs hide),
  `check_orientation_roundtrip` (an orientation survives a keeping policy exactly
  once — no loss, no double-application), and `check_capability_honesty` —
  comprehensive, bidirectional: every declared capability (`push_rows`,
  `encode_from`, animation, streaming, `lossless`, `cheap_probe`, the
  `icc`/`exif`/`xmp`/`cicp` channels, `native_alpha`) must work, and every
  undeclared optional path must decline with `UnsupportedOperation`; all
  violations are reported together. `check_all` runs every check with default
  inputs. Includes two codecs validated against the harness in-crate: a faithful
  `reference` (round-trips pixels *and* metadata, declares/honors every
  capability) and a `minimal` one (one-shot only, declares every optional
  capability false) exercising the false-direction branches, plus a hand-built
  GPS/thumbnail EXIF fixture. The repo is now a Cargo workspace (`zencodec` root +
  `zencodec-testkit` member); `zencodec`'s published package is unaffected. (23d4046)
- **Cross-codec color-emission policy** —
  `resolve_color_emit(&SourceColor, &EncodeCapabilities, ColorEmitPolicy) -> ColorEmitPlan`,
  a pure `no_std` decision of which color carriers (ICC vs CICP) to write for a
  target, with no CMS and no codec dependencies. The `color` module is private;
  the types are re-exported at the crate root (`zencodec::ColorEmitPolicy`, …).
  - `ColorEmitPolicy { Compatibility, Balanced (default), Compact, Verbatim, Custom(ColorEmitFields) }`;
    `ColorEmitPlan { cicp: Option<Cicp>, icc: IccDisposition }`;
    `IccDisposition { KeepSource, SynthesizeFrom(Cicp), Drop }`. Handles the
    grayscale/CMYK terminal states and never emits a redundant `SynthesizeFrom(sRGB)`.
    (Names carry the emit direction so they can't be confused with the decode-side
    `SourceColor`.)
  - `ColorEmitFields::new` makes `ColorEmitPolicy::Custom` constructible downstream.
  - `EncodeCapabilities` gains `cicp_is_valid_carrier` (standardized carrier —
    JXL/AVIF/HEIC `nclx`, PNG `cICP`) and `cicp_safe_sole_carrier` (safe CICP-only
    — JXL, AVIF, HEIC, whose `nclx`/CICP is spec-mandated and reader-authoritative)
    (+ `with_*`); `IccRetention` gains `DropIfCicpRepresentable`,
    `DropIfCicpSafeSoleCarrier`. `Balanced` keeps a synthesized ICC companion when
    the CICP carrier is not sole-safe (PNG `cICP`). The plan lowers through
    `zenpixels_convert`'s `finalize_for_output_with`; a `SynthesizeFrom`
    materializes via the transfer-aware `synthesize_icc_for_cicp` — a bundled
    `const` profile, or (with `cms-moxcms`) generated — never a mis-tagged TRC,
    never a silent drop.
  - `EncodePolicy` carries the color-carrier emission policy: `color:
    Option<ColorEmitPolicy>` (+ `with_color`, `resolve_color`), so encode and
    transcode select the ICC-vs-CICP carrier through it. Its docs reframe the
    legacy `embed_*` flags as a coarse best-effort codec gate, and point at
    `EncodeJob::with_metadata_policy` for reliable field-level retention.
    `MetadataPolicy` is now `Copy`.
  - `helpers::set_exif_orientation` rewrites a blob's EXIF orientation tag inline
    (offset-preserving) so a baked-upright pixel buffer and its embedded tag can't
    disagree (the double-rotation hazard). Applied by the pipeline, not by the
    color resolver.
  - `exif::ByteOrder` is module-scoped (a TIFF/EXIF header detail), not re-exported
    at the crate root.
  - Design + rejected alternatives: `docs/color-emit-model.md`. (23d4046, bbe4c7e, 3fb841e)
- **EXIF string-field editing** — `Exif::set_copyright` / `set_artist` set (insert
  or replace) the IFD0 rights tags, materialized through the existing canonical
  `Exif::to_bytes` (offsets recomputed, byte-exact fixpoint preserved). The new
  `exif::TextEncoding` (re-exported at the crate root) lets the caller pick the
  TIFF field type explicitly: `Ascii` (Exif 2.x, type 2 — carries UTF-8 bytes
  de-facto, most compatible) or `Utf8` (Exif 3.0 / CIPA DC-008-2023, type 129 —
  spec-conformant Unicode, thinly read). Explicit over auto-upgrade because
  auto-promoting non-ASCII to type 129 would silently produce strings most
  readers can't parse. `Entry` value bytes are now `Cow` so parsed entries stay
  zero-copy while edited ones are owned; the `copyright()` / `artist()` /
  `*_bytes()` accessors now borrow `&self`. EXIF tag/type numbers in the parser
  are named constants (no bare hex), and the `ExifPolicy` timestamps category is
  `datetimes` (plural — it covers DateTime / Original / Digitized / OffsetTime* /
  SubSecTime*). (f4b9f1b)
- **Explicit metadata-retention policy at embed time (compile-time enforced)** —
  retention is a *transient* choice made when handing metadata to the encoder, not
  state stored on `Metadata`. New blessed entry points:
  `EncodeJob::with_metadata_policy(meta, MetadataPolicy)` and
  `DynEncodeJob::set_metadata_policy(meta, MetadataPolicy)` filter the record via
  `Metadata::filtered` *before* it reaches the codec, so a codec only ever embeds
  what the policy kept. The pre-existing `EncodeJob::with_metadata` /
  `DynEncodeJob::set_metadata` are now `#[deprecated]`: they propagate metadata
  without a retention choice, so the compiler **warns at every such call site** —
  a compile-time nudge toward `with_metadata_policy`, **not** a semver break
  (existing code still compiles, and codecs still *implement* `with_metadata` as
  the primitive the wrapper routes through; deprecation warns callers, not
  implementors). `MetadataPolicy` has **no `Default`** — callers name a policy
  explicitly (`Web` recommended, privacy-safe). No field was added to `Metadata`
  (`size_of` stays 104 on 64-bit) and its bytes stay untouched until embed, so
  bring-your-own-EXIF-library round-trips still see the originals. (73c5799)
- **EXIF privacy hardening for partial-strip policies** — `MakerNote` (0x927C) is
  dropped whenever `gps` **or** `camera` is stripped (it can embed GPS/serials and
  can't be selectively scrubbed); `SubIFDs` (0x014A, an unmodeled sub-IFD pointer)
  is dropped on a rewrite rather than left dangling; IFD1 (thumbnail-directory)
  entries are filtered by the same per-category rules as IFD0 (a keep-thumbnail
  policy previously kept their Make/Model/DateTime); and `exif::retain` now fails
  **safe** for a >4 GiB blob under a stripping policy (drop, not pass-through). The
  `Web`/`ColorAndRotation` presets were already safe — these close gaps for
  hand-rolled `Custom` policies. (d8a2fae)
- **From-scratch EXIF construction** — `Exif::new(TextEncoding)` (+ `Default`,
  which uses `Ascii`) starts an empty little-endian tree, completing the
  `parse`/`new` → edit → `to_bytes` flow so you can build a blob with no source:
  `Exif::new(TextEncoding::Ascii)` → `set_copyright(…)` → `to_bytes()` (raw TIFF;
  the codec adds the APP1 `Exif\0\0` framing). The `TextEncoding` is required — the
  Exif 2.x ASCII (type 2) vs Exif 3.0 UTF-8 (type 129) choice is a blob property
  used by `set_copyright`/`set_artist` (type 129 is read by almost nothing, so it
  can't be a silent default). (b7acd9f, 73c5799)
- **`Metadata::with_copyright(&str)` / `with_artist(&str)`** — one-liner rights
  stamping that builds an EXIF blob if there is none and merges into a parseable
  existing one (keeping other tags), replacing an unparseable one. Written ASCII
  (Exif 2.x, most compatible); for UTF-8/Exif 3.0 or other tags, build via
  `exif::Exif` + `with_exif`. (1051288)

- **Field-level metadata retention** — `Metadata::filtered(&MetadataPolicy)`,
  the shared filter for re-encode / recompress pipelines: keep what a
  downstream image needs, strip the rest, without callers hand-parsing EXIF.
  - `MetadataPolicy`: `PreserveExact` (keep all, incl. a redundant sRGB ICC),
    `Preserve` (keep all but drop a redundant sRGB ICC), `Web` (recommended —
    ICC non-sRGB + EXIF orientation/rights + CICP/HDR; drop the rest of EXIF
    and all XMP), `ColorAndRotation` (only what places pixels: ICC non-sRGB +
    CICP/HDR + orientation), and `Custom(MetadataFields)`.
  - `MetadataFields` (`#[non_exhaustive]`, `with_*` builders): `icc:
    IccRetention` (`#[non_exhaustive]`; `Drop` / `KeepNonSrgb` / `Keep` —
    three-way sRGB handling), `exif: ExifPolicy`, and `xmp` / `cicp` / `hdr:
    Retention`.
  - `exif::Retention` (`#[non_exhaustive]`; `Keep` / `Discard`, query via
    `keeps`/`discards`) — explicit per-field intent, no `bool`-direction
    ambiguity.
  - Every disposition type (`MetadataPolicy`, `IccRetention`, `Retention`) and
    every record (`Metadata`, `MetadataFields`, `ExifPolicy`) is
    `#[non_exhaustive]` with builder construction, so new policies, ICC modes,
    EXIF categories, retention fields, and `Metadata` fields land additively —
    the surface never needs a semver-major break (see the module's *Forward
    compatibility* docs).
- **Structured EXIF** (`zencodec::exif`) — `Exif<'a>` parses a TIFF/EXIF blob
  into a borrowing IFD tree (zero-copy; thumbnails/values are never copied),
  `Exif::filtered(&ExifPolicy)` prunes by category, and `Exif::to_bytes`
  re-serializes a valid TIFF with recomputed offsets. `ExifPolicy`
  (`#[non_exhaustive]`, `with_*` builders) has seven categories: `orientation`,
  `rights`, `thumbnail`, `gps`, `datetimes`, `camera`, `other` — so e.g.
  "drop only the thumbnail" or "strip GPS" is one field. `exif::retain` is the
  `Cow` entry point: borrows the source unchanged when nothing is dropped
  (so `Metadata::filtered` is a cheap `Arc` clone), allocates only on a real
  rewrite. Bounds-checked, no panics on untrusted input; preserves byte order
  and `Exif\0\0` framing. (`helpers::parse_exif_orientation` now delegates
  here.)
  - Hardened (adversarial review + 80M+ fuzz executions across four targets):
    the serializer **deduplicates aliased out-of-line values** so a malformed
    IFD pointing many entries at one blob can't amplify the rewrite ~1000×
    (DoS); Copyright/Artist accessors read both **ASCII (type 2) and UTF-8
    (type 129, Exif 3.0)** per CIPA DC-008 (a UTF-8-typed field was previously
    dropped as unknown), expose raw bytes (`copyright_bytes` / `artist_bytes`)
    alongside the lossy-UTF-8 text view, and a pruning rewrite preserves field
    bytes **and TIFF type** verbatim (never transcoded — neither corrupted nor
    "corrected"); EXIF categories were corrected per the spec's tag tables —
    the Exif-IFD creator/owner *name* tags (CameraOwnerName 0xA430, Photographer
    0xA437, ImageEditor 0xA438) are attribution (`rights`, kept by a copyright
    policy — they were previously stripped as "other"), and firmware / editing-
    software / unique-ID tags are device identity (`camera`); the thumbnail
    length tag is read as SHORT *or* LONG (real cameras use SHORT — was silently
    dropping valid thumbnails);
    structural sub-IFD pointers too short to hold an offset are preserved
    (peek-before-remove) instead of dropping the sub-IFD; and `retain` passes a
    >4 GiB blob through untouched rather than risk `u32` offset truncation.
  - Robust error model: `Exif::parse` returns `None` on structural failure but
    **gracefully skips** an individual unreadable / unknown-type / out-of-bounds
    entry (and salvages a truncated entry table) — one bad or future-typed
    entry no longer discards the whole IFD; `retain` **fails safe** (drops EXIF
    it can't parse under a stripping policy rather than leaking it through); and
    `to_bytes` is **canonical** (a byte-exact fixpoint), so filtering is
    idempotent (a fuzz-found non-idempotence, now a regression seed).
  - Test infrastructure: differential tests against `kamadak-exif`
    (`tests/exif_differential.rs`), four libFuzzer targets (`fuzz/` — parse,
    roundtrip, filter, and `Metadata::filtered`), a stable regression harness
    with a committed crash seed (`tests/fuzz_regression.rs`), and a zero-copy
    benchmark over 1 KiB–1 MiB thumbnails (`benches/exif_filter.rs`).
- `ThreadingPolicy::resolve_thread_count()` — cross-codec shared helper that
  translates a [`ThreadingPolicy`] to the integer thread count that
  native-threaded encoder libraries (rav1e/ravif, dav1d/rav1d, libwebp, etc.)
  accept. Returns `1` for `Sequential`, `0` (auto) for `Parallel` and every
  other variant. Replaces hand-written `policy_to_threads` helpers in
  individual codec crates (Cluster B Class 1 dedup).
- `ResourceLimits::for_untrusted_input()` (with `safe_default()` alias) — a
  safer starting point than `ResourceLimits::default()` for services
  accepting bytes from the network or end users. Caps: 100 MP per frame,
  200 MP across an animation, 16384×16384 max dims, 1 GiB memory, 256 MiB
  input, 65536 frames, 1 hour duration. `ResourceLimits::default()`
  continues to mean "no limits" for backwards compatibility (bc2790d).

### Changed

- `metadata::parse_exif_orientation` now delegates to the canonical
  `helpers::parse_exif_orientation`. The previous local implementation was
  a looser duplicate that read the orientation value as `u16` regardless
  of TIFF type, missing `TIFF_LONG` (type 4) values for big-endian inputs
  and lacking the IFD entry-count cap and tag-sort early-exit DoS
  protections present in the helper (141238f).
- `DynDecodeJob` and `DynEncodeJob` shim setters now `debug_assert!` when
  called after the inner job has been consumed by an `into_*` method,
  catching the (structurally unreachable) misuse path loudly in tests and
  dev builds. Release behaviour is unchanged (silent no-op). Trait
  signatures are unchanged (a5b782e).

### Documentation

- Module-level docs in `policy.rs` now recommend `DecodePolicy::strict()`
  as the starting point for untrusted input, paired with
  `ResourceLimits::for_untrusted_input` (468073d).

## [0.1.20] - 2026-04-21

### Added

- `ISO_21496_1_URN` public constant — the 28-byte `urn:iso:std:iso:ts:21496:-1\0`
  namespace string that prefixes gain-map payloads in JPEG APP2 (and any other
  URN-namespaced container) (945b694).
- `ISO_21496_1_PRIMARY_APP2_BODY` public constant — the full 32-byte JPEG
  APP2 body (URN + `min_version=0, writer_version=0`) that the primary image
  of a canonical Ultra HDR JPEG carries to advertise ISO 21496-1 awareness.
  Goes directly inside an APP2 segment after the `FF E2` marker + length
  header; detected by exact bytes match (945b694).
- `Iso21496Format::JxlJhgm` variant — canonical name for the bare ISO 21496-1
  payload (no version byte, no URN). Produces identical bytes to the
  deprecated `JpegApp2` variant; naming parallels `AvifTmap` (each variant
  named for the container that consumes those exact bytes) (945b694).
- `Iso21496Format::JpegApp2BodyWithUrn` variant — produces and accepts the
  full JPEG APP2 body: URN + bare payload. Does NOT include the JPEG `FF E2`
  marker or `u16 BE` length word (those remain the caller's JPEG syntax
  responsibility). Handled by `parse_iso21496_fmt` / `serialize_iso21496_fmt`
  with no separate `_with_urn` helpers (945b694).
- `Iso21496Format` discriminants pinned with explicit `= 0..3` values plus a
  `const _: () = assert!(...)` block, so accidental reorders/removals trip at
  compile time instead of silently shifting `as u8` results (945b694).
- `GainMapParseError::UrnMismatch` variant, returned when parsing under
  `Iso21496Format::JpegApp2BodyWithUrn` and the input does not begin with
  `ISO_21496_1_URN` (945b694).
- `gainmap::serialize_iso21496_fmt_into(params, format, &mut Vec<u8>)` —
  append-to-buffer partner for `serialize_iso21496_fmt`. Lets callers embed
  the payload inside a larger output buffer without an intermediate `Vec`
  (e.g., building a JPEG APP2 marker + length + body in one alloc) (945b694).

### Deprecated

- `Iso21496Format::JpegApp2` — misleading name. The bytes it produces are the
  bare ISO 21496-1 payload (no URN), not a standalone JPEG APP2 body. Use
  `JxlJhgm` for the same bytes under a clearer name, or `JpegApp2BodyWithUrn`
  for the full APP2 body including the URN prefix. Kept at its original
  discriminant `0` so existing `as u8` casts keep working; it and `JxlJhgm`
  are distinct variants that happen to serialize to identical bytes (Rust
  does not allow two variants to share a discriminant) (945b694).

### Fixed

- Formatting of the `ISO_21496_1_PRIMARY_APP2_BODY` constant declaration
  collapsed onto one line and a stray trailing blank line after a private
  helper removed, so `cargo fmt --check` is clean (41f7162).

## [0.1.19] - 2026-04-16

### Added

- Auto-parse `Orientation` tag from EXIF blob in `Metadata::with_exif` (631b1fe).
- `ThreadingPolicy::Sequential` and `ThreadingPolicy::Parallel` variants,
  plus the `is_parallel()` helper — one method, one decision for codec
  implementors (db098aa, 25b1b78).

### Changed

- `ThreadingPolicy` default switches from `Unlimited` to `Parallel`
  (semantically equivalent; `is_parallel()` returns `true` for both)
  (db098aa).
- Codec threading guidance documented end-to-end: the `pool.install()`
  pattern, server shared-pool pattern, sequential mode, native-threaded
  codec caveats, and implementor guidelines with code examples (5ba7519,
  c91ff32).

### Deprecated

- `ThreadingPolicy::SingleThread`, `LimitOrSingle`, `LimitOrAny`,
  `Balanced`, and `Unlimited` — rayon-based codecs can't reliably cap
  threads from the inside; only the caller can, via `pool.install()`.
  Callers should migrate to `Sequential` or `Parallel`. Old variants
  still work through `is_parallel()` (db098aa).

### Fixed

- README lists `Iso21496Format` in the gainmap module table (574de90).
- `cargo doc --no-deps` now emits zero warnings: cross-crate references
  use fully-qualified `zenpixels::ColorContext` paths, and
  crate-private `MAX_IFD_ENTRIES` / `resolve_color` symbols use plain
  code spans instead of intra-doc links (574de90).

## [0.1.18] - 2026-04-15

### Fixed

- Re-export `helpers::descriptor_for_decoded_pixels_v2` alongside v1.
  0.1.17 added the v2 function as `pub fn` inside the private `icc`
  submodule but only re-exported v1 from `helpers/mod.rs`, leaving v2
  inaccessible to downstream crates. Callers migrating off the
  deprecated v1 path can now reach v2 via `zencodec::helpers`.

## [0.1.17] - 2026-04-15

Authority-aware color resolution. New `descriptor_for_decoded_pixels_v2`
replaces the deprecated `descriptor_for_decoded_pixels` with a wider
correction target type, spec-compliant authority handling, and a
composable `resolve_color` primitive.

### Added

- **`SourceColor::to_color_context()`** (17afe6c) — authority-aware
  conversion to `zenpixels::ColorContext`. When `color_authority` is
  `Cicp`, drops `icc_profile`; when `Icc`, drops `cicp`. Downstream
  `ColorContext::as_profile_source()` then returns the right source
  with no separate authority parameter.
- **`helpers::descriptor_for_decoded_pixels_v2`** (cb4a419 + 9ff4ace)
  — drops the deprecated `IccMatchTolerance` placebo parameter.
  `corrected_to` widens from `Option<&zenpixels::Cicp>` to
  `Option<&zenpixels::ColorProfileSource<'_>>` so callers can describe
  correction targets that aren't CICP-expressible (arbitrary
  primaries+transfer pairs, named profiles like Adobe RGB v2-gamma,
  custom ICC profiles).
- **`helpers::resolve_color`** (9ff4ace) — underlying
  `(ColorPrimaries, TransferFunction)` resolution without descriptor
  scaffolding. Separates color identity resolution from pixel-format
  commitment; callers can inspect the result (e.g., refuse to encode
  `(Unknown, _)` without user confirmation) before building a
  `PixelDescriptor`. Used once per decode, then composed with
  per-format descriptors — replaces the pattern of running the
  priority chain N times per codec.

### Fixed

- **`descriptor_for_decoded_pixels` now respects `color_authority`**
  (9ff4ace) — when both CICP and ICC fields are populated, the
  authoritative one wins. Previously CICP always took precedence,
  which silently violated the spec for codecs that declare ICC
  authoritative (JPEG, PNG with iCCP chunk, WebP, TIFF). The old
  function is deprecated but keeps the fix via delegation to `_v2`.

### Deprecated

- **`helpers::descriptor_for_decoded_pixels`** — requires the
  deprecated `IccMatchTolerance` enum with no alternative in 0.1.x.
  Migrate to `descriptor_for_decoded_pixels_v2`.

### Changed

- Bump `zenpixels` dependency from `0.2.7` to `0.2.8`. No API impact
  on zencodec consumers — the new zenpixels release ships
  zenpixels-convert-side additions (`PluggableCms`, `RowTransformMut`,
  fused matlut kernels, `ConvertOptions::clip_out_of_gamut`) that
  zencodec doesn't depend on directly.

## [0.1.16] - 2026-04-14

### Changed

- Bump `zenpixels` to 0.2.7 with the `icc` feature enabled. All ICC
  identification now delegates to `zenpixels::icc`, which ships a superset
  of the web-corpus table (163 RGB + 18 grayscale profiles vs. our 118+14,
  with intent-safety masks cross-validated against moxcms and lcms2) (9bdb797).
- `icc_extract_cicp` → deprecated shim around `zenpixels::icc::extract_cicp`.
- `helpers::identify_well_known_icc`, `helpers::icc_profile_is_srgb` →
  deprecated shims around `zenpixels::icc::{identify_common, is_common_srgb}`.
- `helpers::IccMatchTolerance` → deprecated placebo. `identify_common` uses
  `Tolerance::Intent` internally; sub-Intent variants are indistinguishable
  at 8-bit and 10-bit output. All in-tree callers already pass `Intent`.

### Removed

- `src/helpers/icc_table_{rgb,gray}.inc` — superseded by the tables shipped
  in `zenpixels::icc`.
- `scripts/mega_test.rs`, `scripts/verify_via_moxcms.rs`,
  `scripts/fetch-profiles.sh` — superseded by `zenpixels/scripts/icc-gen`
  (a proper superset with lcms2 cross-validation) and the `icc-fetch` recipe
  in `zenpixels/justfile`.
- `examples/verify_via_moxcms.rs`, `examples/gen_moxcms_profiles.rs` —
  superseded by `zenpixels/scripts/icc-gen`.

## [0.1.15] — unreleased (skipped)

In-tree version bump only. Contained the zenpixels 0.2.2 → 0.2.6 bump
(d00efca) and a minor clippy fix (31cca1f). Shipped as part of 0.1.16.

## [0.1.14] - 2026-04-12 — YANKED

Yanked because the `zenpixels 0.3.0` dependency bump was premature —
zenpixels 0.3.0 was not yet released on crates.io. Superseded by 0.1.16,
which tracks `zenpixels 0.2.7`.

### Added

- `icc_extract_cicp()` lightweight CICP-tag extractor for ICC v4.4+
  profiles (1176ec1). Cross-validated against moxcms (0f853c5) and the
  saucecontrol/Compact-ICC-Profiles corpus (c514fc1).
- `ColorAuthority` re-export from zenpixels; `SourceColor` now tracks
  whether ICC or CICP is authoritative for CMS transforms (1176ec1).
- Normalized ICC hash table with 132 web-corpus-verified profiles (12c20d2).

### Changed

- MSRV lowered from 1.93 to 1.88 (PR #9, 1938d25).

## [0.1.13] - 2026-04-07

### Added

- `ImageFormat::Jp2`, `Dng`, `Raw`, `Svg` format detection (02dd783).
- `ResourceLimits::max_total_pixels` — cap for the sum of all frame
  pixel counts across an animation (86dffb6). `max_pixels` remains
  per-frame; docs clarified (0d430a6).

## [0.1.12] - 2026-04-01

### Added

- `serialize_iso21496_jpeg` / `parse_iso21496_jpeg` — ISO 21496-1 gain
  map payloads embedded as JPEG APP2 segments (3e2437f).

### Changed

- ISO 21496-1 gain map API renamed for spec accuracy: continued-fraction
  encoding for rationals (966e1b2), standardized flag and field names
  (745851b, 5af86f3). Back-compat shims kept for one release with
  `#[deprecated]` attributes (bf6c7fa).
- Bump `zenpixels` / `zenpixels-convert` 0.2.0 → 0.2.2 (5fbf5ee).
- Bump `archmage`, `magetypes`, `enough`, `whereat`, `linear-srgb`
  and related patches (2f3f1fb).

### Fixed

- ISOBMFF `box_size` handling and silent no-op documentation; assorted
  panic removals from untrusted input paths (PR #7, f4383c3).
- Clippy warnings: unused import, `type_complexity` (cc152b8).

## [0.1.11] - 2026-03-30

### Added

- `parse_exif_orientation()`: spec-compliant EXIF orientation parser (TIFF 6.0,
  EXIF 2.32). Handles raw TIFF and APP1-prefixed input, both endiannesses,
  SHORT and LONG types, with bounds-checked reads and DoS-capped IFD scanning.
  24 tests. Replaces 3 independent implementations across zenjpeg, zenwebp,
  and zencodecs.

### Changed

- Collapsed 21 per-format test functions into 1 table-driven test (22 rows).
  Same coverage, fewer monomorphizations, faster test compilation.

## [0.1.10] - 2026-03-30

### Added

- `descriptor_for_decoded_pixels()`: derives accurate `PixelDescriptor` from source
  color metadata (CICP, ICC profile, or sRGB default) instead of hardcoding sRGB.
  Codecs should use this when building `DecodeOutput` or `OutputInfo`.
- `identify_well_known_icc()`: hash-based ICC profile identification against 45
  known profiles (sRGB, Display P3, BT.2020, BT.709) from Compact-ICC, skcms/Google,
  ICC.org, colord, Ghostscript, HP, Facebook, Kodak, and libvips. ~100ns per lookup.
- `IccMatchTolerance` enum: `Exact` (±1 u16), `Precise` (±3), `Approximate` (±13),
  `Intent` (±56). Every table entry stores measured max u16 TRC error verified against
  its authoritative EOTF for all 65536 input values.
- `icc_profile_is_srgb()`: convenience sRGB detection using `Intent` tolerance.
- `ImageFormat::Pdf`, `ImageFormat::Exr`, `ImageFormat::Hdr`, `ImageFormat::Tga`
  format variants and definitions.
- 65 regression tests for ICC identification and descriptor derivation covering
  all format scenarios (JPEG, PNG, WebP, AVIF, JXL, HEIC, GIF, BMP, TIFF).
- `scripts/fetch-profiles.sh` and `scripts/mega_test.rs` for reproducible TRC
  verification against ICC profiles stored in R2.

### Changed

- Split `helpers.rs` into `helpers/mod.rs` + `helpers/icc.rs` submodule.
  All public re-exports preserved — no breaking change.

### Fixed

- Removed Artifex esRGB from sRGB identification (it's linear scRGB, not sRGB).
- TGA format detection hardened to match zenbitmaps footer-based probing.

## [0.1.6] - 2026-03-28

### Fixed

- `ImageInfo::PartialEq` now includes the `resolution` field (was silently skipped,
  causing two values with different resolutions to compare as equal).
- 10 broken rustdoc intra-doc links (`codec_details` on dyn trait objects,
  `ImageFormatRegistry::with`, `PixelDescriptor` qualification, `Any::downcast_ref`/`Deref` paths).

### Added

- Missing derives on public types: `PartialEq` on `Metadata`, `Clone`/`PartialEq`/`Eq` on
  `DecodeCapabilities`, `Clone`/`PartialEq` on `EncodeCapabilities`, `PartialEq` on
  `GainMapParseError`.

### Changed

- Bumped `zenpixels` dependency from 0.2.0 to 0.2.1 (gamut matrices, serde support,
  embedded ICC profiles, bug fixes).
- README: added badges, ecosystem cross-links, limitations section, MSRV declaration;
  fixed dead guide links and stale `from_magic()` reference.

## [0.1.5] - 2026-03-26

### Changed

- `DecoderConfig::job(self)` now consumes `self` (was `&self`). Uses GAT + method
  lifetime to avoid forcing `'static` on the config.

### Added

- `DecodeJob::with_extract_gain_map()` — opt in to gain map extraction during decode.
- Default impl for `DynDecodeJob::set_extract_gain_map`.

## [0.1.4] - 2026-03-26

### Changed

- Added `Send` supertrait to `DynEncoder` (required for cross-thread encoder dispatch).

## [0.1.3] - 2026-03-25

### Added

- `GainMapSource` — raw gain map data extracted from container (pre-decode).
  Carries raw encoded bitstream + format + ISO 21496-1 metadata + recursion
  depth counter for safe nested decode. Accessible via
  `zencodec::gainmap::GainMapSource`.
- `DecodedGainMap` — decoded gain map pixels + metadata (post-decode).
  Cross-codec normalized type. Accessible via
  `zencodec::gainmap::DecodedGainMap`.
- Both types are `#[non_exhaustive]` with `new()` constructors.

### Changed

- Documented supplement decode convention: detection is always cheap
  (container metadata), pixel decode is opt-in. `ImageInfo.supplements`
  flags describe what's available, not what's decoded.
- Updated `docs/spec.md` with three-layer decode output model
  (ImageInfo, SourceEncodingDetails, Extensions type-map) and
  supplement access conventions.

## [0.1.2] - 2026-03-25

### Added

- `ImageInfo.is_progressive` field — true for progressive JPEG (SOF2),
  interlaced PNG (Adam7), interlaced GIF. Detectable from headers during
  cheap probe.
- `ImageInfo.with_progressive()` builder method.

## [0.1.1] - 2026-03-24

### Changed

- Drop unnecessary `imgref` feature from zenpixels dependency.
- Add magic byte detection audit example.

## zencodec-render

### [Unreleased]

#### QUEUED BREAKING CHANGES

- None; new unpublished companion API.

#### Added

- Shared `Renderer`, `RenderRequest` and `ColorTarget` for selected or decoded
  images: full oriented sRGB RGBA8 canvas by default, explicit required output
  and regions, aligned auxiliary retention and one final publication decision.
  Geometry/color implementations moved from the native prototype; tests now
  import the shared crate. Conversion remains outside core codec dependencies.
  The review converter requires Rust 1.89; the core MSRV is unchanged.
