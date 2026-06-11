# zencodec Public-API Ablation Report

**Date:** 2026-06-11
**Snapshot commit:** 5fb8fc33 (`refactor(api-doc): exclude underscore-prefixed features from all-features snapshot`)
**Snapshot:** `docs/public-api/zencodec.txt`, default-features section, **2,231 items**
**Grep template:** `grep -rn --include='*.rs' '<symbol>' /home/lilith/work/zen/<crate> ... | grep -v '/target/'`
**Scan scope:** `/home/lilith/work/zen/*` excluding `zencodec/` itself, `.jj/`, `target/`, `docs/public-api/`
**Bar:** flag count must be <10% of 2,231 (< 224 items) — well met.

---

## Summary

| Class | Count | % of 2,231 |
|-------|------:|-----------|
| A-class flags (`#[doc(hidden)]` / `#[deprecated]` — already marked) | 5 | 0.22% |
| B-class flags (pub→pub(crate) or remove) | 0 | 0.00% |
| Spec-coverage gaps (in snapshot, absent/incomplete in spec.md) | ~30 | ~1.3% |
| **Total flagged** | **5** | **0.22%** |

Conservative outcome: zencodec's surface is largely intentional. All five A-class
flags are items already marked `#[deprecated]` by a prior maintainer and carrying
zero external callers — they are ready for `#[doc(hidden)]` pending the next
breaking release that removes them.

---

## A-class flags — `#[doc(hidden)]` candidates

All five are already `#[deprecated]` in source. Action: add `#[doc(hidden)]` to
suppress from docs.rs. Removal must wait for the next breaking release (tracked in
CHANGELOG's "QUEUED BREAKING CHANGES"). Do not remove in a patch/minor.

| # | Symbol | External callers (as of this scan) | Notes |
|---|--------|------------------------------------|-------|
| 1 | `icc::icc_extract_cicp` | **0** | `#[deprecated(since="0.1.16")]`. Entire `pub mod icc` exists only because removing it is a breaking change (see CLAUDE.md). Wrapper returns raw `Option<(u8,u8,u8,bool)>` tuple; replacement is `zenpixels::icc::extract_cicp`. |
| 2 | `helpers::IccMatchTolerance` | **0** (1 comment-only hit in zenjpeg) | `#[deprecated(since="0.1.16")]`. Placebo enum — `Exact/Precise/Approximate/Intent` had no behavioral effect. Removed from `descriptor_for_decoded_pixels_v2` signature. The zenjpeg hit is a comment in an `#[allow(deprecated)]` annotation, not an actual use. |
| 3 | `helpers::identify_well_known_icc` | **0** | `#[deprecated(since="0.1.16")]`. Superseded by `descriptor_for_decoded_pixels_v2`; the v2 function handles ICC profile identification internally. |
| 4 | `helpers::icc_profile_is_srgb` | **0** via `zencodec::helpers` | `#[deprecated(since="0.1.16")]`. The 14 grep hits for this name in `zencodecs` (zenpipe) are that crate's own `icc_profile_is_srgb` (FNV-1a hash table implementation — a completely different function). |
| 5 | `helpers::descriptor_for_decoded_pixels` | **0** | `#[deprecated(since="0.1.17")]`. Old signature with `IccMatchTolerance` placebo param. All codec call sites verified to use `descriptor_for_decoded_pixels_v2`. |

---

## B-class flags

None. Every non-deprecated item with zero or few external hits is either:
- Named in `docs/spec.md` (KEEP by definition), or
- Part of a logical group where removal would break the public contract, or
- Uncertain enough that the conservative bar excludes it.

---

## Spec-coverage gaps

Items present in the snapshot that are absent from or incomplete in `docs/spec.md`.
These are **documentation gaps**, not API mistakes. Flagged here for spec maintenance,
not for removal. No action required on the crate; action is to update `docs/spec.md`.

### `gainmap` module — almost entirely unspec'd

The gain-map API was added in 0.1.19-0.1.21 after the current `spec.md` was written.
The following items appear in the snapshot with no corresponding spec entry:

- `gainmap::Fraction` / `gainmap::UFraction` — rational number types for gain-map math
- `gainmap::GainMapSource` — enum distinguishing JFIF APP2/EXIF/XMP sources
- `gainmap::GainMapParseError` — error type returned by `parse_iso21496`
- `gainmap::parse_iso21496` / `parse_iso21496_fmt` — ISO 21496-1 metadata parsing
- `gainmap::serialize_iso21496` / `serialize_iso21496_fmt` / `serialize_iso21496_fmt_into` — serialization counterparts
- `gainmap::Iso21496Format` — enum of ISO 21496-1 container formats
- `gainmap::ISO_21496_1_PRIMARY_APP2_BODY` / `ISO_21496_1_URN` — magic byte constants

Spec currently only lists: `GainMapChannel`, `GainMapParams`, `GainMapInfo`, `GainMapPresence`,
`GainMapRender`, `GainMapDirection`, `DecodedGainMap`. The parsing/serialization utilities,
error type, `Fraction`/`UFraction`, `GainMapSource`, and `Iso21496Format` are not covered.

### `ImageFormat` — spec names a non-existent method; variants incomplete

- Spec says `ImageFormat::from_magic(data)` exists as a method on `ImageFormat`.
  **It does not.** Format detection is `ImageFormatRegistry::detect()`. The spec is stale.
- Spec's variant list omits: `Dng`, `Exr`, `Hdr`, `Jp2`, `Pdf`, `Qoi`, `Raw`, `Svg`, `Tga`
  (all present in the snapshot).

### `ImageInfo` — field list incomplete in spec

Spec's `ImageInfo` field summary is partial. Fields present in snapshot but not in spec:
- `is_progressive: bool`
- `gain_map: GainMapPresence`
- `resolution: Option<Resolution>`
- `sequence: Option<ImageSequence>`
- `supplements: Option<Supplements>`

### Types with no spec coverage

- `Resolution` struct + `ResolutionUnit` enum
- `ImageSequence` enum
- `Supplements` struct
- `ColorAuthority` re-export (from zenpixels)
- `StopToken` / `Unstoppable` re-exports (from almost_enough/enough)

### `SourceColor` methods not spec'd

- `SourceColor::color_authority() -> ColorAuthority`
- `SourceColor::has_hdr_transfer() -> bool`
- `SourceColor::to_color_context()`

### `ColorEmitPolicy` — method not spec'd

- `ColorEmitPolicy::fields()` — returns `ColorEmitFields` bitmask

### `ThreadingPolicy` variants — partial spec coverage

Spec lists: `Balanced`, `Unlimited`, `SingleThread`, `LimitOrAny`, `LimitOrSingle`.
Snapshot also has: `Parallel`, `Sequential` (not in spec).

### `LimitExceeded` variants — partial spec coverage

Spec does not list `LimitExceeded::TotalPixels`.

### `DecodeCapabilities` flags — partial spec coverage

Spec does not list: `gain_map`, `multi_image`, `reconstructs_hdr`, `streaming`, `stop`.

### `DecodeJob` gain-map methods — not spec'd

- `DecodeJob::with_extract_gain_map`
- `DecodeJob::with_gain_map_render`
- `DynDecodeJob::set_extract_gain_map`
- `DynDecodeJob::set_gain_map_render`

---

## Top-10 digest

1. **A1 `icc::icc_extract_cicp`** — already deprecated, 0 callers, add `#[doc(hidden)]`.
2. **A2 `helpers::IccMatchTolerance`** — already deprecated, 0 callers, add `#[doc(hidden)]`.
3. **A3 `helpers::identify_well_known_icc`** — already deprecated, 0 callers, add `#[doc(hidden)]`.
4. **A4 `helpers::icc_profile_is_srgb`** — already deprecated via zencodec::helpers, 0 callers, add `#[doc(hidden)]`. (Name collision with zencodecs' own function is benign — different crate.)
5. **A5 `helpers::descriptor_for_decoded_pixels`** — already deprecated, 0 callers, add `#[doc(hidden)]`.
6. **Gap: spec `from_magic`** — spec names `ImageFormat::from_magic(data)` but it does not exist; spec needs correction to `ImageFormatRegistry::detect()`.
7. **Gap: gainmap module** — 15+ items unspec'd; `spec.md` gain-map section predates the ISO 21496-1 parsing/serialization additions.
8. **Gap: `ImageInfo` fields** — spec's field list is a partial summary; `is_progressive`, `gain_map`, `resolution`, `sequence`, `supplements` not listed.
9. **Gap: `ThreadingPolicy` variants** — spec omits `Parallel` and `Sequential`.
10. **Gap: `DecodeCapabilities` flags** — gain-map, streaming, stop flags not in spec.

---

## What was NOT flagged (conservative exclusions)

- All trait machinery, GATs, associated types (`EncodeJob::Enc`, `DecodeJob::Dec`, etc.)
- All capability structs (`EncodeCapabilities`, `DecodeCapabilities`) — spec'd
- All error types (`UnsupportedOperation`, `CodecErrorExt`, `LimitExceeded`)
- Re-exported Cicp/ContentLightLevel/MasteringDisplay from zenpixels — spec'd and intentional
- `enough`/`Unstoppable`/`StopToken`/`almost_enough` re-exports — spec'd
- `copy_decode_to_sink` (15 external hits), `copy_frame_to_sink` (5 hits) — well-used
- `parse_exif_orientation` (38 hits), `set_exif_orientation` (15 hits) — essential helpers
- `GainMapSource` (60 hits), `parse_iso21496` (17 hits), `serialize_iso21496` (19 hits) — active use
- `descriptor_for_decoded_pixels_v2` — current replacement API, spec'd in helpers section
- `Fraction`/`UFraction` — consumed by ultrahdr + zenavif-parse
- The full `exif::*` surface — struct-typed, intentionally public
- `IccRetention`, `MetadataFields`, `MetadataPolicy`, `ColorEmitPolicy`, etc. — spec'd in 0.1.21
