# Cross-codec color & metadata: asymmetries and defaults

Status: **design analysis / proposal** (not yet implemented). Researched 2026-05-31
against the zen workspace at the commit on `feat/metadata-policy`. Findings were
produced by parallel source reads of all codec crates plus zenpixels, then
adversarially verified — corrections from that pass are folded in below and the
unverified items are listed explicitly in the **Confidence ledger** at the end.

## 1. TL;DR

The hard part is already done, in the right place. Color authority (which of
ICC/CICP wins) is modeled by `zenpixels::ColorAuthority` and resolved by
`SourceColor::to_color_context()`; the actual pixel+color conversion and the
"emit metadata that matches the converted pixels" step is `zenpixels-convert`'s
`finalize_for_output() -> EncodeReady { PixelBuffer, OutputMetadata }`; and
`zencodec` already advertises per-channel carry capability
(`EncodeCapabilities::{icc, cicp, exif, xmp, hdr, gain_map, native_alpha, …}`)
and field-level retention (`MetadataPolicy` / `MetadataFields` / `ExifPolicy`).

What is **missing** is the *seam* that reconciles all three against a concrete
target: nothing takes `(source color + metadata + gain map + orientation)` and a
target's `EncodeCapabilities` and decides keep / derive / synthesize / bake /
tone-map / drop. That logic is today scattered across codecs or left to the
caller. `negotiate.rs` reconciles **pixel layout** (`negotiate_pixel_format`) but
has no color/metadata analogue.

The proposal: add that seam to `zencodec` as a *plan-producing* (no-`std`, no-CMS)
function — the mirror of `negotiate_pixel_format` — plus a handful of missing
capability flags and an **encode-side warnings channel** (without which every
"warn on lossy transcode" default is unimplementable).

## 2. Layered architecture — who owns what

This split is correct and should be preserved. The CLAUDE.md rule ("use zenpixels
pixel types but never re-export them; color-metadata value types may be
re-exported") already encodes most of it.

| Layer | Crate | Owns | Key types |
|---|---|---|---|
| Pixel buffer color | `zenpixels` | The color state of the bytes in hand | `PixelDescriptor` (transfer, primaries, `SignalRange`, `AlphaMode`), `ColorAuthority`, `ColorContext`, `Cicp`, `ContentLightLevel`, `MasteringDisplay`, `Orientation` |
| Color conversion / CMS | `zenpixels-convert` | Converting pixels + emitting matching color metadata | `ConvertPlan`, `Provenance`, `ConversionCost`, `OutputProfile`, `OutputMetadata { icc, cicp, hdr }`, `finalize_for_output() -> EncodeReady` |
| Source description + retention + capability | `zencodec` | What the *file* said; what to keep; what a target *can carry* | `ImageInfo` / `SourceColor` / `EmbeddedMetadata`, `Metadata` / `MetadataPolicy` / `MetadataFields` / `ExifPolicy`, `EncodeCapabilities` / `DecodeCapabilities`, `gainmap::*` |
| Per-codec carrier mapping | each codec crate | Mapping semantic blobs ↔ container slots | e.g. JPEG APP1/APP2, PNG `eXIf`/`iCCP`/`cICP`, AVIF `colr`/`Exif` items |

`zencodec` stays `no_std` + no-CMS. It produces **descriptions and plans**; the
actual pixel work (tone-map, gamut, premultiply, dither) executes one layer up in
`zenpixels-convert` / `zenpipe`.

## 3. What already works (the foundation)

- **ICC-vs-CICP authority.** `SourceColor` holds `cicp: Option<Cicp>` and
  `icc_profile: Option<Arc<[u8]>>` *simultaneously* plus
  `color_authority: ColorAuthority`. `to_color_context()` drops the
  non-authoritative field, falling back to whichever is present
  (`info.rs:335`). The per-format authority rules are pinned as tests
  (`info.rs:1246–1334`): JPEG = ICC-only; AVIF-`nclx` = CICP; AVIF-`rICC` = ICC
  (CICP kept for roundtrip); PNG-`cICP` = CICP, PNG-`iCCP`-only = ICC; JXL-enum =
  CICP, JXL-ICC = ICC.
- **HDR transfer detection.** `SourceColor::has_hdr_transfer()` checks CICP
  (16/18) then scans the ICC `cicp` tag — does not require a full ICC parse.
- **Field-level retention.** `MetadataPolicy` (`Web` default / `Preserve` /
  `PreserveExact` / `ColorAndRotation` / `Custom`) → `MetadataFields` with
  `IccRetention` (3-way Keep/KeepNonSrgb/Drop), `ExifPolicy` (7-category pruning
  via zero-copy `Cow` — borrowed passthrough, owned only on actual prune), and
  **separate** `cicp` vs `hdr` retention so the SDR-flatten case is expressible.
  The `Metadata::filtered` doc (`metadata.rs:183`) already reasons about the
  gain-map ↔ HDR-signaling coupling hazard.
- **Capability advertisement.** `EncodeCapabilities` already exposes
  `icc / cicp / exif / xmp / hdr / gain_map / native_alpha / native_16bit /
  native_f32 / native_gray` plus effort/quality/thread ranges; the structs are
  `#[non_exhaustive]` with getter methods, so new flags are non-breaking.
- **Encode-side color finalization.** `zenpixels-convert::finalize_for_output()`
  atomically converts pixels and emits `OutputMetadata { icc, cicp, hdr }` that
  *matches* the converted pixels, with `Provenance` enabling lossless
  round-trip detection (e.g. f32-widened-from-u8-JPEG → u8 is lossless).

## 4. Capability matrix (corrected)

`R/W` per channel. `N` = native, `via` = via container/sidecar, `exif` = via EXIF
tag, `part` = partial, `-` = none. Decode-only crates show `R/-`. Corrections
from the adversarial verification pass are marked **⚠**.

| Codec | ICC | CICP | prim+TF | EXIF | XMP | MDCV | CLLI | gain-map | orientation | alpha | HDR-depth |
|---|---|---|---|---|---|---|---|---|---|---|---|
| zenjpeg | N/N | -/- | via-icc | N/N | N/N | -/- | -/- | N/N (UltraHDR/MPF) | exif/exif | -/- (no alpha) | part (8-bit DCT) |
| zenpng | N/N | N/N | part (cICP+cHRM) | N/N | N/N | N/N (mDCV) | N/N (cLLi) | -/- | exif/exif (not applied) | N/N (tRNS) | part (8/16; HDR pixels SDR-only ⚠) |
| zenwebp | N/N | -/- | -/- | N/N | N/N | -/- | -/- | -/- | exif/exif (not applied) | N/N (straight) | -/- (8-bit) |
| zenjxl | N/N | **-/- ⚠** | N/N | via/via | via/via | -/- | part R (MaxCLL approx) | N/N (jhgm) | N/N | N/N (assoc flag) | N (no depth-signal ctrl) |
| zenjxl-decoder | part/- | part/- (synth) | N/- | via/- | via/- | part/- | part/- | N/- (jhgm) | N/- | N/- | N/- |
| zenavif | N/N | N/N (nclx) | N/N | N/N | N/N | N/N (mdcv) | N/N (clli) | N/N (tmap ISO 21496-1) | N/N (irot+imir) | N/N (premul flag) | N/N (8/10/12) |
| zenavif-parse | N/- | N/- | N/- | N/- | N/- | N/- | N/- | N/- (tmap) | N/- | N/- | N/- |
| zengif | -/- | -/- | -/- | -/- | -/- | -/- | -/- | -/- | -/- (PAR only) | N/N (1 index→alpha) | -/- (8-bit palette) |
| image-tiff/zentiff | N/N | -/- | part R | N/N (sub-IFD) | N/N (Tag 700) | -/- | -/- | -/- | N/N (not applied) | N/N (ExtraSamples) | N/N (1–64 bit, no HDR) |
| heic | N/- | N/- (nclx) | N/- | N/- | N/- | N/- | N/- | N/- (Apple aux+tmap) | **baked/- ⚠** (applied; reports Identity) | N/- (aux not decoded) | N/- (8–16; gainmap→8) |
| ultrahdr | N/N | **sRGB-hardcoded ⚠** | part | part R (not extracted) | N/N (hdrgm:) | -/- | -/- | N/N (ISO 21496-1) | -/- (ignored) | part | -/- (SDR base; HDR via gainmap) |
| zenbitmaps | -/- (BMP v5 ICC skipped) | part (computed) | part R | -/- | -/- | -/- | -/- | -/- | part R | N/N (straight) | part (8/16/32; no HDR) |
| zenraw | -/- | -/- (OutputPrimaries enum, unsignaled) | part | N/- | N/- | -/- | -/- | N/- (Apple MPF, not applied) | N/part (applied, tag→1) | -/- (RGB only) | part (sensor→u16/f32) |

**Verification corrections to the matrix:**

- **zenjxl CICP write = none (not partial).** `build_jxl_metadata`
  (`zenjxl/src/codec.rs:517`) only processes `icc_profile`/`exif`/`xmp` and never
  reads `meta.cicp`; `JXL_ENCODE_CAPS` does not call `.with_cicp(true)`. CICP is
  only parsed on the *decode* path. A JXL re-encode does **not** preserve a CICP
  description except as the ICC the encoder derives from enum color.
- **UltraHDR base CICP is hardcoded `Cicp::SRGB` on decode**
  (`ultrahdr/.../codec.rs:302`), not read from EXIF. If an encoder set a
  Display-P3/BT.2020 base, decode would still report sRGB — a read/write
  asymmetry, not a "via_exif" path.
- **zenavif is not lossless for gain maps.** ISO 21496-1 rational fields
  (min/max/gamma/offsets/headroom) are continued-fraction-approximated by
  `Fraction::from_f64_cf` (`gainmap.rs:685`), and `prefer_8bit` downscales
  10/12→8-bit. Both are lossy; `zenavif/tests/metadata_roundtrip.rs` covers
  EXIF/XMP/CICP/rotation but **not** gain-map fidelity.
- **zenpng latent coupling bug (harmless today).** On gamut downcast
  (`encode.rs:526`) `cICP`/chromaticities/source-gamma are cleared but
  `mastering_display`/`content_light_level` are **not**, and the metadata writer
  emits `mDCV`/`cLLi` unconditionally (`encoder/metadata.rs:127`). It can't fire
  yet because the only recognized source gamut is SDR Display-P3 (PQ/HLG are
  rejected, `gamut.rs:214`) — but it is a corruption waiting for HDR gamut
  support. Worth a guard now.
- **zenjxl MaxCLL is an approximation that should warn.** It maps JXL
  `intensity_target` → `MaxCLL` when >255 nits with `MaxFALL=0`
  (`codec.rs:1340`). `intensity_target` is a tone-mapping target, not CEA-861.3
  content light level; faithful behavior is to surface it *and warn*.

## 5. Asymmetry classes (verified)

1. **Color authority: ICC-only vs CICP-only vs both vs neither.** JPEG/WebP/TIFF/
   UltraHDR carry ICC only; GIF/zenbitmaps carry neither (implicit sRGB);
   PNG/AVIF/HEIC/JXL carry both with one authoritative. Sharpest mismatch:
   ICC-only ↔ CICP-native. Modeled by `ColorAuthority` + `to_color_context()`.
2. **HDR transfer representation: native CICP enum vs ICC-encoded vs
   gain-map-only vs unrepresentable.** PQ/HLG signaled as CICP 16/18, or inside
   an ICC `cicp` tag, or implicitly via a gain map over an SDR base, or not at
   all (JPEG baseline). `has_hdr_transfer()` exists; `EncodeCapabilities.hdr()`
   conflates "carries transfer" / "carries gain map" / "carries CLLI/MDCV".
3. **Gain map: container-carried vs sidecar-JPEG vs unrepresentable.** Shared
   payload (`GainMapParams`), per-format carrier (AVIF `tmap`, JXL `jhgm`, JPEG
   MPF+XMP `hdrgm:`). PNG/WebP/GIF/TIFF cannot carry one. **Dropping a gain map
   is trivial, not tone-mapping** — a gain-map image's base is already a complete
   rendition (`GainMapDirection::BaseIsSdr`, the common UltraHDR/AVIF case: base
   is SDR, sRGB-signaled, `base_hdr_headroom=0`). To get SDR you just keep the
   base and drop the map (`apply_gainmap` at `display_boost=1.0` is the identity —
   `weight=0 → gain=1`, confirmed in `ultrahdr-core/src/gainmap/apply.rs`). The
   base's color signaling is *already* SDR, so there is nothing to rewrite and no
   double-tone-map hazard. The only non-trivial drop is the rare
   `BaseIsHdr`/subtractive map (base is HDR, alt is SDR — "typical for JXL", scarce
   in the wild): to reach SDR you *apply* the stored gain ratio (still not
   tone-mapping). This is categorically different from transfer-function HDR (#2).
4. **Orientation: EXIF tag vs container box (irot/imir) vs baked-into-pixels vs
   none.** Codecs also differ on whether decode *applies* it. HEIC always bakes
   and reports `Identity` with **no "was-baked" marker** (verified) — a
   double-rotation hazard.
5. **Alpha: straight vs premultiplied vs single-index vs none, declared vs
   inferred.** `AlphaMode {Straight, Premultiplied, Opaque, Undefined}` rides the
   `PixelDescriptor`. AVIF reads a premul flag but the encoder requires it
   *declared* (does not infer from pixels). No capability flag distinguishes
   premultiplied support from alpha support.
6. **EXIF/XMP carrier asymmetry + XMP-only metadata.** Different container slots
   per format; some carry only one. UltraHDR's primary metadata is XMP `hdrgm:`.
   MakerNote (0x927C) and Interop-IFD (0xA005) have offset-rewrite hazards
   (byte-exact only via keep-all).
7. **MDCV/CLLI presence asymmetry.** Native only in AVIF/HEIC/PNG. JXL has
   MaxCLL-only (approx). `MetadataFields.hdr` is one switch over both CLLI and
   MDCV — mismatching formats that carry them independently.
8. **Bit depth / HDR pixel depth.** AVIF/HEIC 8/10/12; PNG/TIFF 8/16 (PNG path
   SDR-only today); JPEG/WebP/GIF 8-bit. `native_16bit` alone can't express
   AVIF's 10/12-bit.
9. **Lossy ICC→CICP derivation gap.** Exact for the recognized-profile corpus and
   ICC v4.4+ `cicp` tags; returns `(Unknown, Unknown)` otherwise. No parametric
   fallback. Setting `(Unknown,Unknown)` on a target is worse than assumed-sRGB.

## 6. The core gap: a capability-aware metadata seam

```
decode → ImageInfo { source_color, embedded_metadata, gain_map, orientation, resolution }
                                  │
                  zenpixels-convert::finalize_for_output(…)
                                  │           → EncodeReady { PixelBuffer, OutputMetadata{icc,cicp,hdr} }   (COLOR handled, but capability-BLIND)
                                  ▼
        ┌─────────────────── MISSING SEAM ───────────────────┐
        │  reconcile(OutputMetadata + Metadata.filtered(policy)│
        │            + gain map + orientation,                 │
        │            target EncodeCapabilities)                │
        │     → EmbedPlan + pixel-op requirements + warnings   │
        └─────────────────────────────────────────────────────┘
                                  ▼
                       codec.encode(pixels, plan)
```

`OutputMetadata` happily emits both ICC and CICP regardless of whether the target
container has slots for them; `Metadata::filtered` is source-driven and
target-blind and cannot see the gain map; `EncodeCapabilities` knows the target
but is wired to neither. Nobody composes the three.

## 7. Representation proposal (concrete)

Keep the layer split and the re-export rules. All struct additions are
non-breaking (`#[non_exhaustive]` + getters).

**7.1 Split the conflated capability booleans** (`capabilities.rs`):
- `hdr()` → `can_carry_transfer_hdr()` (PQ/HLG CICP), `can_carry_content_light_level()`,
  `can_carry_mastering_display()`; keep `hdr()` as a deprecated OR-alias.
- `gain_map()` → `can_encode_gain_map()` / `can_decode_gain_map()`; OR-alias kept.
- Add `can_roundtrip_orientation()` (true when an EXIF tag or container rotation
  box survives without baking; false for GIF/zenbitmaps and for HEIC-decode which
  bakes).
- Add `native_alpha_premultiplied()` distinct from `native_alpha()`.
- Add `max_bit_depth: Option<u8>` (or `native_10bit()`/`native_12bit()`), so
  AVIF's 10/12-bit is expressible.

**7.2 `SourceColor` helpers** (`info.rs`):
- `fn hdr_carrier(&self) -> HdrCarrier { None, CicpTransfer, IccEncoded, GainMap,
  StaticMetadataOnly }` so transcode branches on *how* HDR is signaled, not just a
  bool. The `GainMap` variant reads `ImageInfo.gain_map: GainMapPresence` — wire
  it through, don't duplicate the gain map into `SourceColor`.
- `debug_assert` in `to_color_context()` that authority matches a present field
  (authority=Cicp with `cicp=None` is a codec bug).

**7.3 Orientation provenance** (`info.rs`): add `orientation_was_baked: bool` (or
promote the existing `OutputInfo::orientation_applied` concept to a queryable
`ImageInfo` flag) so the HEIC "baked, reports Identity, no marker" gap is closed
and re-encode never double-rotates.

**7.4 The reconciler** (`negotiate.rs`): leave `negotiate_pixel_format` purely
physical. Add a *separate* function:
```
fn reconcile_color(source: &SourceColor, target: &EncodeCapabilities) -> ColorPlan
enum ColorPlan { KeepIcc, KeepCicp, DeriveCicpFromIcc, SynthesizeIccFromCicp,
                 BakeToTargetSpace(Cicp), ToneMapToSdr, DropWithWarning(Reason) }
```
plus a `NegotiationMode { Lenient, Strict }` so `Strict` rejects lossy color
reinterpretation instead of silently falling back to `available[0]` (the current
documented loss hazard). zencodec emits the *plan*; `zenpixels-convert` executes
any pixel op (synthesize ICC, tone-map, bake).

**7.5 Split `MetadataFields.hdr`** into `clli: Retention` and `mdcv: Retention`
(keep an `hdr` convenience setter that sets both), matching AVIF/HEIC/PNG reality.

**7.6 Gain-map disposition**: a helper
`prepare_gain_map_for(target, gm) -> GainMapPlan { Rewrap(Iso21496Format),
DropKeepSdrBase, ApplyToRecoverSdr }`. The common path is `DropKeepSdrBase` — the
base is already the SDR rendition, so dropping the map is a no-op on pixels and
signaling (no tone-map). `ApplyToRecoverSdr` is only for the rare `BaseIsHdr`
map. There is **no `ToneMapAndDrop`** — gain-map drop never tone-maps (that's the
transfer-HDR path, rule 10, a separate mechanism).

**7.7 Add an encode-side warnings/lossy-report channel** (see §9 — this is a
prerequisite, not an option).

## 8. Default rules — "the right thing by default"

| # | Source → target situation | Default | Why |
|---|---|---|---|
| 1 | ICC source, target carries both | Keep ICC authoritative; *also* derive CICP (v4.4 tag or corpus) as roundtrip bonus | Never lose the ICC; CICP helps CICP-preferring consumers |
| 2 | CICP source, target ICC-only (JPEG/WebP/TIFF) | Synthesize ICC from CICP primaries+TF; pixels unchanged | No CICP slot exists; assumed-sRGB silent loss is unacceptable |
| 3 | CICP source, target CICP-native | Pass CICP through verbatim; no ICC synthesis | Lossless and cheap |
| 4 | Unrecognized ICC, target CICP-only-native | Bake conversion to a target-native space (sRGB/P3) in convert, tag matching CICP, warn | Unknown ICC can't reduce losslessly; `(Unknown,Unknown)` is worse than sRGB |
| 5 | Orientation present, target preserves it | Pass through, translating EXIF 0x0112 ↔ irot/imir; don't bake | Lossless, keeps it editable |
| 6 | Orientation present, target has no slot, **or** caller asked Correct, **or** web default | Bake into pixels (lossless DCT for JPEG), set Identity, mark baked | If the channel can't survive, applying it is the only faithful option |
| 7 | Decoder already baked (HEIC, zenraw) | `orientation == Identity`; encode must not re-apply | Closes the double-rotation gap |
| 8 | Gain map, target can carry it | Re-wrap `GainMapParams` into target `Iso21496Format`, re-encode gain image, keep `alternate_cicp/icc` | Shared payload; only carrier + gain-image codec change |
| 9 | Gain map, target cannot carry it | **Drop the map, keep the base verbatim** (`BaseIsSdr`: base is already the SDR rendition, sRGB-signaled — no pixel math, no signaling change). Only `BaseIsHdr` (rare) applies the stored gain to recover SDR. No tone-mapping either way | The base is a complete rendition; `apply_gainmap@boost=1` is the identity. Gain-map drop ≠ transfer-HDR tone-map |
| 10 | **Transfer-function** HDR (PQ/HLG *pixels*), target SDR-only | Tone-map to SDR, rewrite CICP transfer to SDR, drop CLLI/MDCV, warn | Here the *pixels* are HDR-encoded; a PQ tag on tone-mapped pixels mis-signals; colorimetric clip is a corruption. Distinct from #9 |
| 11 | Premultiplied source, target straight-only | Convert premul→straight in convert before encode | Mis-declared alpha corrupts edges |
| 12 | Alpha source, target has no alpha (JPEG) | Composite over a defined background (default opaque), warn — don't silently drop | Silent drop changes transparent-region pixels |
| 13 | Higher-depth source, lower-depth target | Negotiate to highest target depth ≥ source; reduce with error-diffusion, never truncate | Truncation banding is a precision-loss corruption |
| 14 | Default metadata retention on web transcode | `MetadataPolicy::Web`: keep ICC (drop redundant sRGB), EXIF orientation+rights, CICP+HDR; drop GPS/datetime/camera/thumbnail + all XMP | Privacy + bloat reduction while preserving color + attribution |

## 9. Second-tier asymmetries & open questions (from the completeness critic)

These are real, mostly unmodeled, and should be triaged before the design is
called complete. Several are "wrong pixels" issues and therefore non-negotiable
under this repo's zero-tolerance rule.

- **Encode-side warnings channel is missing (prerequisite).** `ImageInfo.warnings`
  exists on decode; `EncodeOutput`/`OutputInfo` have none (verified). Every
  "warn" default above is unimplementable until an encode-side
  warnings/lossy-report channel exists. **Add this first.**
- **Chroma siting (half-pixel shift).** `Cicp` carries only
  `{primaries, transfer, matrix, full_range}` — **no `chroma_sample_position`**
  (verified). JPEG sampling factors ↔ AVIF/HEIF `chroma_sample_position` ↔ JXL
  mismatch shifts chroma by half a pixel = wrong pixels, with no channel to carry
  the siting. Needs a field (would ride alongside `Cicp`) and a rule.
- **Full-range vs limited-range YCbCr.** `Cicp.full_range` exists but
  `negotiate_pixel_format` explicitly ignores `signal_range` — a full↔limited
  mismatch crushes blacks/whites silently. Same severity as the color-authority
  mismatch; needs the `Strict` mode.
- **Resolution / DPI roundtrip.** `Resolution` is on `ImageInfo` (read) but **not
  in `Metadata`**, so transcode has no carrier. JFIF density (in/cm/aspect-only) ↔
  PNG `pHYs` (integer pixels/meter — cannot represent 72 dpi exactly) ↔ TIFF
  rational ↔ EXIF 0x011A/B are four incompatible encodings, and EXIF resolution
  can disagree with the container's. No precedence rule.
- **Rendering intent.** ICC and PNG `sRGB` chunk both carry an intent byte; CICP
  and JXL have no slot. Lost silently on ICC→CICP or sRGB-chunk→JPEG transcode.
  No field holds it.
- **Grayscale has no CICP.** A gray ICC (single TRC) can't be expressed as CICP
  (RGB/matrix-centric); gray-ICC-only → CICP-native is strictly unrepresentable,
  and "bake to sRGB" would needlessly colorize. Gray+alpha → formats without
  2-channel native is also unhandled.
- **CMYK / N-channel.** `channel_count` is a bare `u8` with no colorspace tag;
  RGB/CMYK/Lab/multispectral are indistinguishable. CMYK JPEG / separated TIFF
  have no defined transcode behavior.
- **ICC v2 vs v4.** The `cicp` tag is v4.4+ only; a v2 profile can never
  self-describe CICP even for a known space. The derivation path differs by
  version — unstated in the ICC→CICP rule.
- **C2PA / JUMBF provenance.** Lives in JUMBF boxes *and* XMP, is signed, and
  **any** pixel re-encode invalidates it. The correct default is to **drop** an
  invalidated manifest, not preserve it — the inverse of the usual instinct. No
  JUMBF carrier in the model.
- **Depth maps & segmentation mattes.** `Supplements.{depth_map, segmentation_mattes,
  auxiliary}` are flagged but get no transcode class, despite the same
  drop-or-rewrap structure as gain maps. HEIF stores alpha as an auxiliary image
  ("aux not decoded") — HEIF-aux-alpha → PNG must promote it to a real alpha
  channel.
- **Per-frame color in Multi/Animation.** `SourceColor` is canvas-level; APNG /
  multi-page TIFF whose frames declare different ICC/CICP collapse to the
  primary's color on transcode, silently recoloring the rest. No per-frame
  channel, no warning.
- **Background color.** Rule 12 composites over white/black but no field carries
  the source's declared background (PNG `bKGD`, GIF background index) to inform
  the choice.

## 10. Test coverage — what exists, what to build

**Today (all in-`zencodec`, all in-memory):**
- `tests/exif_differential.rs` — 100+ well-formed blobs vs the kamadak-exif
  oracle (orientation SHORT/LONG, copyright, artist, both byte orders).
- `tests/fuzz_regression.rs` — walks `fuzz/regression/*`, asserts
  filter→serialize→parse→re-filter idempotence.
- `fuzz/fuzz_targets/{exif_parse,exif_filter,exif_roundtrip,metadata_filtered}.rs`
  — serializer re-parsability, 7-category policy bitmap, accessor preservation,
  filter idempotence.
- `benches/exif_filter.rs` — validates the zero-copy `Cow` contract.
- `tests/comprehensive.rs` — trait surface + Metadata-builder/orientation
  plumbing, but only against a mock animation codec + PNM. It validates the
  in-memory types, **not** real color/metadata serialization through any format.

**The critical gap: there are ZERO cross-codec color/metadata round-trip tests.**
The only real metadata round-trip in the whole stack is
`zenavif/tests/metadata_roundtrip.rs` (single codec, AVIF→AVIF, and it skips
gain-map fidelity). Every default rule in §8 is currently **unverified**.

**To build** — a workspace-level integration suite (a new crate that can depend on
the real codecs; `zencodec` itself has no codec to round-trip through):
1. Cross-codec pipeline: encode one source to JPEG/PNG/WebP/AVIF/JXL, assert which
   of {orientation, ICC, EXIF-by-category, CICP, CLLI, MDCV} survives each hop and
   that the §8 rules fire (CICP-only AVIF→JPEG synthesizes ICC; ICC JPEG→AVIF
   derives CICP).
2. Authority reconciliation: both ICC+CICP+authority → `to_color_context()` drops
   the right field; transcode keeps the non-authoritative field when the target
   can carry it.
3. Orientation *application* (not just tag serialization): `Correct` rotates
   pixels and reports Identity; a baked decoder (HEIC) does not double-rotate.
4. HDR round-trip: CLLI/MDCV through PNG/AVIF/HEIC; JXL MaxCLL-only path warns;
   PQ→SDR tone-map rewrites CICP and drops CLLI/MDCV.
5. Gain-map cross-codec: AVIF `tmap` ↔ JPEG UltraHDR ↔ JXL `jhgm` re-wrap of
   shared `GainMapParams`; **drop-keeps-SDR-base** path (assert the base survives
   byte-for-byte and stays sRGB-signaled — no tone-map); `BaseIsHdr` apply path;
   `alternate_cicp/icc` survival; compact-vs-full serialization (always full).
6. ICC→CICP derivation matrix: recognized → exact; unrecognized → bake-to-space,
   **not** `(Unknown,Unknown)`.
7. XMP whole-segment Keep/Discard through PNG iTXt / JPEG APP1 / AVIF item / WebP.
8. `negotiate` strict mode rejects lossy color reinterpretation vs the current
   silent fallback.
9. Premultiplied↔straight alpha and depth-reduction-with-dithering through real
   codecs.

## 11. Confidence ledger

**Source-verified (high confidence):**
- The layer split, `SourceColor`/`to_color_context`, `MetadataPolicy`/`MetadataFields`,
  `ExifPolicy` `Cow` contract, `EncodeCapabilities` flag set, `negotiate.rs` being
  pixel-format-only, `zenpixels-convert::finalize_for_output`/`OutputMetadata`
  shape — read directly.
- zenjxl does **not** write CICP (refutes the matrix's earlier "partial").
- HEIC unconditionally bakes orientation and reports Identity with no marker.
- UltraHDR base CICP is hardcoded sRGB on decode (not an EXIF read).
- zenavif gain-map fractions + 10/12→8-bit are lossy (refutes "zero lossy").
- zenpng gamut-downcast clears `cICP` but not `mDCV`/`cLLi` (latent, dormant).
- `Cicp` has no `chroma_sample_position`; encode path has no warnings channel.
- zenjxl MaxCLL is an `intensity_target` approximation, not normative CLLI.

**Reported by readers, not independently re-verified (treat as likely, confirm
before coding):** the finer per-codec cells for `image-tiff`/`zentiff`,
`zenbitmaps`, `zenraw`, `heic` alpha/aux, and `zenwebp` decode caps. The
second-tier items in §9 are mostly *absences* (easy to confirm by grep) rather
than behaviors.

**Open product decisions (need your call):** whether the reconciler/`ColorPlan`
lives in `zencodec` (plan-only) with execution in `zenpixels-convert` (recommended)
vs. a new `zentranscode` crate; whether the cross-codec test suite is a new
workspace member or lands in `zenpipe`; default background color for alpha-flatten;
and the C2PA drop-vs-preserve policy (legal implications).
