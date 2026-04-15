# Changelog

All notable changes to zencodec are documented here.

## [Unreleased]

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

- `SourceColor::to_color_context()` — authority-aware conversion that
  drops the non-authoritative field so `ColorContext::as_profile_source()`
  returns the right source without a separate authority parameter (17afe6c).

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
