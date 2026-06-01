# Changelog

All notable changes to zencodec are documented here.

## [Unreleased]

## [0.1.21] - 2026-05-29

### Added

- **Color-signaling production policy** (`zencodec::color`) — `resolve_color_emit`
  reconciles a `SourceColor` against a target's `EncodeCapabilities` under a
  `ColorPolicy` (`Compatibility`/`Balanced`/`Compact`/`Verbatim`/`Custom`) and
  returns a `ColorPlan { cicp, icc: IccDisposition }`: derive CICP from an ICC and
  drop the redundant profile only where CICP is the format's authority and safe
  as the sole carrier (`EncodeCapabilities::{cicp_is_format_authority,
  cicp_safe_sole_carrier}` — JXL today), else keep/synthesize the ICC; grayscale
  and CMYK suppress CICP. `IccRetention` gains `DropIfCicpRepresentable` /
  `DropIfCicpSafeSoleCarrier`. Deliberately minimal surface (HDR/gain-map
  dispositions + warnings notes deferred; `ColorPlan` is `#[non_exhaustive]`).
- **Field-level metadata retention** — `Metadata::filtered(&MetadataPolicy)`,
  the shared filter for re-encode / recompress pipelines: keep what a
  downstream image needs, strip the rest, without callers hand-parsing EXIF.
  - `MetadataPolicy`: `PreserveExact` (keep all, incl. a redundant sRGB ICC),
    `Preserve` (keep all but drop a redundant sRGB ICC), `Web` (**default** —
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
  `rights`, `thumbnail`, `gps`, `datetime`, `camera`, `other` — so e.g.
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
    (type 129)** per Exif 2.32 / CIPA DC-008 (a UTF-8-typed field was previously
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

### QUEUED BREAKING CHANGES
<!-- Breaking changes that will ship together in the next 0.x minor release.
     Add items here as you discover them. Do NOT ship these piecemeal — batch them. -->
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
