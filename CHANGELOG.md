# Changelog

All notable changes to zencodec are documented here.

## [Unreleased]

### QUEUED BREAKING CHANGES
<!-- Breaking changes that will ship together in the next 0.x minor release.
     Add items here as you discover them. Do NOT ship these piecemeal — batch them. -->
- Rename `OutputInfo` → `PredictedOutputInfo` (`decode::PredictedOutputInfo`): it is the decoder's
  *predicted* output shape, not a measured fact — the name should say so.
- Ablate `ThreadingPolicy` to what codecs can actually honor — keep `Sequential` and `Parallel`,
  remove the deprecated variants (`SingleThread`, `LimitOrSingle`, `LimitOrAny`, and the two
  deprecated aliases that map to `Parallel`). Rayon codecs cannot reliably cap thread count from
  inside, so those modes were never real.
- Ablate `ResourceLimits` to the caps codecs actually enforce (per the 2026-06-20 enforcement
  audit): **remove** the fields no codec honors — `max_animation_ms` (a duration, not a resource;
  2 callers) and the running `max_output_bytes` cap (no encoder honors it mid-stream) — and the
  never-called `check_image_info` / `check_output_info` omnibus helpers. Keep the load-bearing caps
  (`max_pixels`, `max_memory_bytes`, `max_width` / `max_height`, `max_input_bytes`, `max_frames`),
  `threading`, and `prefer_fallible_allocations`. (Non-breaking follow-up, done separately: wire
  `max_total_pixels` + a per-cap `enforces_*` honesty flag.)
- Remove `icc_extract_cicp` re-export and the top-level `icc` module.
  Callers should use `zenpixels::icc::extract_cicp`, which returns a typed
  `Cicp` instead of a `(u8, u8, u8, bool)` tuple.
- Remove `helpers::IccMatchTolerance`, `helpers::identify_well_known_icc`,
  and `helpers::icc_profile_is_srgb`. Callers should use
  `zenpixels::icc::{identify_common, is_common_srgb}` which return the
  richer `IccIdentification` (adds `valid_use: IdentificationUse` so
  callers can distinguish metadata-only matches from matrix+TRC-safe
  substitution). `descriptor_for_decoded_pixels` will drop its
  `IccMatchTolerance` parameter — it is currently a placebo.
- Remove `helpers::descriptor_for_decoded_pixels` (deprecated in 0.1.17).
  Callers migrate to `descriptor_for_decoded_pixels_v2` which drops the
  placebo `IccMatchTolerance` and widens `corrected_to` to
  `Option<&ColorProfileSource>`.
- Remove `gainmap::Fraction::from_f64` and `gainmap::UFraction::from_f64`
  (deprecated since 0.1.12). Callers should use `from_f64_cf`, which
  produces canonical continued-fraction encodings matching libultrahdr.
- Remove `gainmap::parse_iso21496` and `gainmap::serialize_iso21496`
  (deprecated since 0.1.12). Callers should use `parse_iso21496_fmt` /
  `serialize_iso21496_fmt` with an explicit `Iso21496Format` (AvifTmap
  vs. JpegApp2) to avoid the format ambiguity that motivated the rename.
- Remove `SourceColor::has_hdr_transfer()` — moves to a pipeline-level
  utility that consults `ColorProfileSource` and `HdrPolicy` together
  rather than inspecting raw CICP/ICC fields.

### Added
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
  cancellation-only sketch). `ErrorCategory` (`#[non_exhaustive]`, `Copy`):
  `MalformedImage`, `UnexpectedEof`, `UnsupportedImageType`,
  `UnsupportedImageFeature`, `UnsupportedPixelFormat`, `UnsupportedOperation`,
  `CmsRequired`, `PolicyRejected`, `Cancelled`, `TimedOut`,
  `LimitsExceeded(LimitKind)`, `OutOfMemory`, `Io`, `InvalidParameters`,
  `InvalidBuffer`, `InvalidState`, `Internal` — the set distilled from an
  inventory of every zen codec's error enum (the "unsupported" axis splits into
  type / image-feature / pixel-format-negotiation / API-operation; `CmsRequired`
  covers e.g. zenwebp's ICC-synthesis-unavailable; `PolicyRejected` covers valid
  input a configured policy declined, e.g. zenjxl's progressive-rejected;
  `InvalidBuffer` is a wrong-geometry pixel buffer and `InvalidState` is API
  misuse / wrong-sequence). `CategorizedError: Any { fn codec_name(&self) ->
  Option<&'static str>; fn category(&self) -> ErrorCategory }` is **opt-in** (not
  blanket-impl'd, not added to the `Error` bound), so adopting it is additive and
  back-compatible. The `Any` supertrait keeps a `dyn CategorizedError` downcastable
  (both methods take `&self`, so the trait is dyn-compatible); `codec_name()` is
  required, returning `None` for the non-codec cause types. A blanket impl forwards
  through `whereat::At<E>` so a located error keeps its category;
  zencodec's own cause types implement it (`LimitExceeded` →
  `LimitsExceeded(kind)`, `UnsupportedOperation` → `UnsupportedOperation` /
  `UnsupportedPixelFormat`, `enough::StopReason` → `Cancelled`/`TimedOut`), making
  a codec's mapping a one-line delegation per arm.
  Adds `LimitKind` (the value-free discriminant of `LimitExceeded`, via
  `LimitExceeded::kind()`). No new dependencies. The testkit `reference` codec
  adopts the trait; `reference_cancellation_is_classifiable` proves end-to-end
  classification through a real cancelled operation. The category set is backed
  by a per-codec error inventory + right-sizing analysis in
  `docs/error-taxonomy-inventory.md` (with `docs/error-types-ecosystem.md`
  surveying the surrounding crates).
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
