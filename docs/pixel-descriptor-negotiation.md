# Pixel-descriptor probe certainty & source-aware negotiation

**Provenance:** 8 parallel source surveys of the sibling codec crates,
2026-07-14, against zencodec @ `edc02c7` (`codecset`) and each codec's local
`main`. Every claim below is traced to `file:line`; see §7. This doc backs the
single-decode, precision-preserving transcode work (`CodecSet::transcode`).

---

## TL;DR

1. **There are two different "source descriptors", and only one is authoritative.**
   - The **decoder-native** descriptor: the exact `PixelDescriptor` the decoder
     will produce for *this* image, derived from the parsed header (bit depth
     incl. float-ness, channels, alpha, transfer/primaries). Every decoder
     computes this internally. It is exact.
   - The **probe-reconstructed** descriptor: what you can rebuild from
     `ImageInfo.source_color`'s scalar fields (`bit_depth: Option<u8>`,
     `channel_count`, `cicp`, …). Best-effort, lossy, and for some codecs
     absent or undefined.
2. **Negotiate in the decoder, using the native descriptor — never
   reconstruct-from-probe-then-negotiate.** The probe loses information the
   negotiation needs (float-vs-int, RAW's decoder-decided output, GIF's
   unpopulated depth). JXL already does exactly this and is the reference.
3. **Lift JXL's `choose_pixel_format` / `can_produce_losslessly`
   (`zenjxl decode.rs:391-475`) into one shared
   `negotiate_pixel_format(source, preferred, available)`.** It is the validated
   "widen-only, never narrow, native fallback that never flattens" algorithm.
   Today's shared helper (`zencodec negotiate.rs:49`) takes no `source` at all —
   that is the whole bug.
4. **Encode `supported_descriptors()` must mean the *fidelity envelope*, not the
   accept-and-convert set.** WebP, GIF, JPEG and AVIF all advertise `*F32`
   (and JPEG/PNG advertise 16-bit) descriptors they silently downconvert. Feeding
   those to "preserve precision" is pure memory bloat with silent loss.

---

## 1. Can every probe emit a *certain* source descriptor?

**No — but the codecs that can't are exactly the ones where it doesn't block
negotiation**, because negotiation runs in the decoder off the native descriptor,
not off probe. Probe's `source_descriptor` is a transparency bonus with per-codec
confidence. Three tiers:

| Tier | Meaning | Codecs |
|------|---------|--------|
| **1 — Certain** | Full descriptor from header (or structurally fixed); safe to expose at probe | farbfeld, QOI, PNM, HDR/RGBE, **PNG**, AVIF*, HEIC† |
| **2 — Certain-for-common, caveated** | Right for the common case; a named axis is missing or assumed | JPEG, WebP, JXL, BMP, TGA |
| **3 — Cannot emit at probe** | No source descriptor is produced / definable pre-decode | GIF, RAW/DNG |

\* AVIF: certain for real-world HDR (bit depth from `av1C`, transfer/primaries
from `nclx`), but falls back to sRGB when the `colr` box is absent.
† HEIC: `probe()` is certain on depth + PQ/HLG transfer; ICC + `clli`/`mdcv`
need the heavier `probe_full()`.

**Per-codec probe verdict** (can probe populate `ImageInfo.source_descriptor`?):

| Format | Verdict | depth | channels | alpha | color/HDR | Notable gap |
|--------|---------|-------|----------|-------|-----------|-------------|
| farbfeld | ✓ fixed | 16 ✓ | 4 ✓ | ✓(always) | sRGB assumed | primaries assumed |
| QOI | ✓ | 8 ✓ | ✓(header) | ✓(header) | colorspace byte ✓ | — |
| PNM | ✓ | 8/16/32 ✓ | ✓ | ✓ | PFM linear ✓, else sRGB | no ICC/CICP in format |
| HDR/RGBE | ✓ fixed | 32(f32) ✓ | 3 ✓ | ✗(none) | linear ✓ | primaries assumed |
| **PNG** | ✓ **strong** | 1/2/4/8/16 ✓ | ✓ | **✓ all types incl tRNS (pre-IDAT)** | cICP+iCCP ✓, HDR ✓ | gAMA/cHRM parsed-but-dropped; `cLLI` casing bug |
| AVIF | ⚠→✓ | 8/10/12 ✓(av1C) | ✓ | ✓(auxl) | nclx CICP ✓, clli/mdcv ✓ | nclx-absent→sRGB; no diffuse_white |
| HEIC | ⚠/✓_full | 8/10/12 ✓(hvcC) | ⚠(mono not surfaced) | ✓(aux URN) | CICP ✓; ICC+clli/mdcv only in probe_full | mono→reports 3ch |
| JPEG | ⚠ | 8/12 ✓ | 1/3/4 ✓ | ✓(=false) | ICC raw ✓; no CICP | color-model/subsampling/gain-map computed-but-dropped |
| WebP | ⚠ | 8 ✓(invariant) | ⚠(3-v-4 only) | ✓ | ICC ✓; no CICP | **blind to grayscale** (stored as YUV/ARGB) |
| JXL | ⚠ | count ✓ | ✓ | ✓ | cicp+ICC ✓ | **float-vs-int lost** (inferred `bits==32`); no diffuse_white/mastering at probe |
| BMP | ⚠ | bpp ✓(per-pixel) | ⚠(post-palette) | ⚠(32-bit assumed) | sRGB assumed | bpp≠per-channel |
| TGA | ⚠ | ⚠(color-mapped under-reports) | ⚠ | ⚠(32-bit assumed) | sRGB assumed | — |
| GIF | ✗ | **None** | **None** | ⚠(full frame-walk; true-on-fail) | sRGB assumed | emits **no** descriptor; `cheap_probe=false` |
| RAW/DNG | ✗ | ⚠ **estimated** sensor depth | ✗ None | ✗(none) | ✗ | **decoder-decides output**; sensor≠output |

**RAW is the instructive outlier:** its sensor depth (12/14/16, itself only
*estimated* from the white level) is never the output precision. Output is RGB16
or RGBF32 chosen by `OutputMode` + `preferred` at decode time. So for RAW the
"source descriptor" can only mean the **decoded-faithful** descriptor (RGB16 /
RGBF32), never the mosaic. That is the general rule: **source descriptor = the
most faithful decoded representation, not the container encoding.**

---

## 2. Descriptor capability matrix

What each codec can **produce** (decode) and its true **fidelity ceiling**
(encode) — the honest max it stores, after seeing through accept-and-downconvert.

| Format | Decode produces (max) | Encode fidelity ceiling | Alpha | HDR | Gray |
|--------|----------------------|-------------------------|-------|-----|------|
| **JXL** | U8/U16/**F32** × {Gray,GrayA,RGB,RGBA} | 8-/16-int + **F32** + HDR (intensity_target) + wide-gamut/PQ/HLG | ✓ | ✓ | ✓ |
| **PNG** | 8/**16**-bit + F32(passthru) | **16-bit** (u16) + cICP HDR; F32→8 | ✓ | ✓(16+cICP) | ✓ |
| **farbfeld** | **RGBA16** (fixed) | **RGBA16 bit-exact** | ✓(always) | ✗ | (widened in) |
| **AVIF** | 8/**16**(10·12→u16) | **10-bit** + HDR(nclx+clli+mdcv); 16→10, F32-SDR→8 | ✓ | ✓(10-bit) | mono |
| **HEIC** | 8/**16**(10→RGB16) | *(decode-only)* | ✓ | ✓ | (mono internal) |
| **RAW/DNG** | RGB16 / **RGBF32** (config) | *(decode-only)* | ✗ | linear | — |
| **HDR/RGBE** | **RGBF32_LINEAR** (fixed) | RGBE (~8-bit mantissa+shared exp — **f32 not bit-exact**) | ✗ | ✓ | ✗ |
| **PNM** | 16(gray)·F32; **color-16→8** | 8-int **or** F32 (RGBA-F32 drops α); **no u16 out** | ✓ | PFM linear | ✓ |
| **JPEG** | 8-bit (F32 promoted; **no 16**) | **8-bit, 1/3ch, NO alpha, NO HDR** | ✗(enc) | ✗ | ✓ |
| **WebP** | 8-bit (RGB8/RGBA8/BGRA8) | **8-bit** sRGB(+ICC) | ✓ | ✗ | enc-in only |
| **GIF** | 8-bit (forced RGBA8) | **8-bit indexed ≤256, 1-bit alpha** | 1-bit | ✗ | ✗(palette) |
| **BMP** | 8-bit (RGB/RGBA/Gray) | **8-bit RGB/RGBA** (no gray, no 16) | ✓ | ✗ | dec-only |
| **TGA** | 8-bit (RGB/RGBA/Gray) | **8-bit** | ✓ | ✗ | ✓ |
| **QOI** | 8-bit (RGB/RGBA) | **8-bit lossless** (no gray) | ✓ | ✗ | ✗(enc) |

Precision anchors: **farbfeld (16-bit RGBA)** and **HDR/RGBE + JXL/PNM (f32)**
are the "high precision must survive" end; **GIF/WebP/JPEG/BMP/TGA/QOI (8-bit)**
are the caps. AVIF is a 10-bit HDR pivot; note **12-bit is silently capped to
10-bit** (ravif supports `Twelve`, the zenavif wrapper never invokes it).

---

## 3. The accept-and-downconvert (bloat) traps

Descriptors an encoder **advertises** in `supported_descriptors()` but does **not**
preserve — feeding them high precision wastes memory and silently loses data.
This is why the encode envelope must be redefined as *fidelity*, not *accept*.

| Format | Advertises (encode) | Actually preserved (fidelity envelope) | Lie |
|--------|--------------------|----------------------------------------|-----|
| WebP | +RGBF32,RGBAF32,GRAYF32 | 8-bit only | 3× F32 → 8-bit at input |
| GIF | +RGBF32,RGBAF32,GRAYF32 | 8-bit indexed | 3× F32 → 8-bit at input |
| JPEG | +RGB16,RGBA16,GRAY16,+3×F32,+RGBA/BGRA | 8-bit, no alpha | 16/F32 → 8-bit; alpha dropped |
| PNG | +RGBF32,RGBAF32,GRAYF32 | 16-bit (u16 real) | F32 → 8-bit (defeats HDR; route HDR via u16) |
| AVIF | +RGB16,RGBA16,+F32-SDR | 10-bit | 16→10; F32-SDR→8 |
| JXL | — | 8/16-int + F32 (all real) | **none (honest)** |
| farbfeld | — | RGBA16 (real) + widen-in | **none (honest)** |

**Consequence for the navigator:** cap the target's usable precision at its
fidelity ceiling *before* matching the source. A 16-bit source → WebP must
negotiate to 8-bit (WebP's real ceiling), not be handed an f32 buffer WebP just
downconverts — that is the "no bloat" guarantee, and it only works if the
envelope tells the truth.

---

## 4. How decoders handle `preferred` today — 5-way inconsistent

The single biggest structural finding: there is **no shared negotiation path**.

| Codec(s) | `preferred` handling |
|----------|----------------------|
| **JXL** | ✅ source-aware: `choose_pixel_format` (widen-only, never narrow, native fallback). **The reference.** |
| **AVIF, HEIC** | source-aware-ish: build a bit-depth-ordered `available` list (16-bit-first when >8), then the shared `negotiate_pixel_format`. Correct because the *list* is source-ordered. |
| **PNG** | shared **precision-blind** `negotiate_pixel_format`: native on empty `preferred`, but a non-matching non-empty list **force-converts to RGB8 → flattens 16-bit/HDR**. ← the bug, live. |
| **JPEG** | own `select_decode_descriptor`: source-aware on gray-vs-color only, **bit-depth-blind**. |
| **GIF** | own `negotiate_format`: fixed priority RGB8>BGRA8>RGBA8, **forces RGBA8** default. |
| **WebP** | own `negotiate_format`: only the lossless RGBA→BGRA swizzle; else ignores. |
| **zenbitmaps ×6** | `decoder()` **ignores `preferred`** entirely (source-driven output); `preferred` only reaches the `push_decoder` sink path via `copy_decode_to_sink`. |

A single source-aware `negotiate_pixel_format(source, preferred, available)`
collapses all of these into one correct path. AVIF/HEIC keep their source-ordered
`available` (or drop the manual ordering once the helper is source-aware). PNG's
flatten disappears. JPEG/GIF/WebP replace their bespoke logic. zenbitmaps gain
`preferred` support for free.

---

## 5. The navigator — one source-aware helper, run in the decoder

### 5.1 Where negotiation runs

**Inside the decoder**, off the *native* descriptor it just parsed — not
reconstructed from probe. Justification is empirical: probe drops float-ness
(JXL), can't define output pre-decode (RAW), and doesn't populate depth at all
(GIF). The decoder always has the exact native descriptor (JXL proves the model).

`ImageInfo.source_descriptor: Option<PixelDescriptor>` is added as a **best-effort
transparency signal** — populated at probe where Tier-1/2 certain, `None` for
Tier-3 — but negotiation never depends on it.

### 5.2 The scorer (lifted from JXL, generalized)

One function, used identically on both sides (decoder picks output, encoder picks
input) — both ask "how well does candidate match source?":

```
negotiate_pixel_format(source, preferred, available) -> PixelDescriptor:

  native = source                      # the decoder's exact native descriptor

  # 1. Honor caller preference, first LOSSLESS producible (JXL's rule):
  for want in preferred:
      if can_produce_losslessly(native.channel_type, want.channel_type)   # widen-only
         and layout_compatible(native.layout, want.layout)                 # no channel drop
         and (want is not gray  or  native is gray)                        # gray-source guard
         and want.transfer matches native (or unknown)                     # no transfer remap
         and want.primaries ⊇ native.primaries:                            # no gamut clip
          return the matching `available` entry

  # 2. No preference (or none lossless) -> native precision+layout. NEVER flatten.
  return best_available_matching(native)   # widen-only to nearest available; native if present

can_produce_losslessly(native, target):    # verbatim from JXL decode.rs:465
  U8  -> target in {U8, U16, F32}
  U16 -> target in {U16, F32}
  F32 -> target == F32
```

Two properties this guarantees, both from the user's constraints:
- **No downconvert loss / HDR survives:** step 1's precision gate is widen-only;
  step 2 falls back to native, never to a lossy default. A 10-bit PQ source
  against `preferred=[RGB8]` returns native U16-PQ, not flattened RGB8.
- **No bloat:** because it is *widen-only, first-acceptable*, it never picks a
  wider candidate than the source needs unless the caller explicitly asked. To
  stop upconvert bloat on the encode side, feed `available` = the target's
  **fidelity envelope** (§3), so a 16-bit source → WebP sees only 8-bit
  candidates and lands at 8-bit — no f32 buffer.

### 5.3 The transcode path collapses

`CodecSet::transcode` reduces to a single decode:

```
decode_preferring(input, encoder.fidelity_envelope())   # one decode
   -> decoder negotiates its native desc against the envelope, source-aware
carry Metadata::from(&info)  ->  encode
```

No double-decode, no membership pre-check. The decoder does the precision match
because only it holds the exact source.

---

## 6. Trait & helper changes (grounded in the surveys)

**Shared (`zencodec`):**
- `negotiate_pixel_format(source, preferred, available)` — new source-aware
  signature (deprecate the `(preferred, available)` one at `negotiate.rs:49`).
  Body = §5.2, lifted from JXL. Add `can_produce_losslessly` as a public helper.
- `best_encode_format(source, supported)` already takes `source`
  (`negotiate.rs:94`) — make its ranking precision/bloat-aware (least-precision
  covering candidate), not first-format-match.

**Decoder side:**
- `decoder(data, preferred)` (+ `push_/streaming_/animation_frame_`) keep their
  signatures; contract doc updated to "closest-to-source, widen-only, native
  fallback, never flatten." Each decoder calls the new helper with its native
  descriptor. **Only PNG, JPEG, GIF, WebP, zenbitmaps change** (adopt the shared
  helper); JXL/AVIF/HEIC are already correct.
- `ImageInfo.source_descriptor: Option<PixelDescriptor>` — best-effort, populated
  at probe per the Tier table (§1). Transparency only.
- **Probe fidelity fixes surfaced by the survey** (separate, non-blocking):
  PNG `cLLI`→`cLLi` casing; PNG gAMA/cHRM/sRGB → `SourceColor`; JXL float flag at
  probe; JPEG color-model/subsampling; AVIF nclx-absent OBU fallback + diffuse_white.

**Encoder side:**
- Redefine `supported_descriptors()` = **fidelity envelope** (what the encoder
  *preserves*), ordered by preference. WebP/GIF/JPEG drop their `*F32` (and JPEG
  its 16-bit) from the fidelity list; those remain accept-and-convert conveniences
  inside `encode()`. AVIF's ceiling is 10-bit (its 16-bit entries are convert).
  JXL/PNG/farbfeld lists are already honest.
- Contract: encode at the input's precision — never silently upconvert.

**Rollout:** additive (deprecate, don't remove); testkit
`check_precision_negotiation` feeds 8-bit and 16-bit fixtures and asserts the
output descriptor matches the source (no flatten, no bloat) — catches the
PNG-vs-JXL divergence and any regression.

---

## 7. Per-codec source references

- **JPEG** `zenjpeg/zenjpeg/src/codec/decode.rs:185-207,353-370,1020-1064`,
  `encode.rs:209-222,288-290,960-1014`, probe `decode.rs:309-320`→`info.rs:11-46`.
- **PNG** `zenpng/src/codec.rs:45-71,2612-2645,2696-2746`; probe
  `codec.rs:1830-1940`, tRNS `ancillary.rs:187-201`, cLLI bug `ancillary.rs:222`.
- **WebP** `zenwebp/src/codec.rs:323-355,1500-1528,2136-2249,949-1039`; probe
  `detect.rs`, float traps `tests/float_input_descriptors.rs`.
- **GIF** `zengif/src/codec.rs:171-187,454-456,1058-1060,1225-1544`; probe walk
  `detect.rs:173,267-360`, `cheap_probe=false` `codec.rs:234`.
- **AVIF** `zenavif/src/codec.rs:245-308,1665-1693,2265-2322,2579-2649`; ravif
  `ravif/ravif/src/av1encoder.rs:76-80,1111,1263-1313`.
- **JXL** `zenjxl/src/decode.rs:391-475` (the reference algorithm),
  `codec.rs:105-124,1714-1730,1930-2061`.
- **HEIC** `heic/src/codec.rs:99-105,206,554-577,827,1681-1849`; decode-only.
- **RAW/DNG** `zenraw/src/zencodec_impl.rs:253-254,416-466`,
  `decode.rs:363-422,1002-1007`; decode-only, decoder-decides output.
- **zenbitmaps** `bmp_codec.rs:31-36,325-375`, `pnm_codec.rs:46-54,177-201`,
  `farbfeld_codec.rs:33-38,310-325`, `hdr_codec.rs:29,357-367`,
  `tga_codec.rs:34-38,373-393`, `qoi_codec.rs:31-32,403-416`; all
  `decoder()` ignore `preferred` (source-driven).

---

# PHASE 2 — convert paths, multi-page, and color management

**Provenance:** 4 further surveys (zenpixels-convert; TIFF/PDF multi-page; the
color-emit decision layer; moxcms), 2026-07-14. This section **revises §5–§6**:
the "lift JXL's `choose_pixel_format` into a new shared helper" plan is superseded
— a richer classifier already exists and should be *relocated*, not reinvented.

## 8. Fidelity is a per-image PIPELINE, not a single descriptor pick

A transcode is a chain, and end-to-end fidelity is the **composition** of three
arrows (worst link dominates), each with its own per-image `DescriptorSupport`:

```
source ─[decode]→ decoded ─[convert / CMS]→ adapted ─[encode]→ output
        arrow 1            arrow 2                    arrow 3
                                              (+ color signaling: a parallel
                                               metadata channel on arrow 3)
```

The middle arrow — **convert (`zenpixels-convert`)** — is a first-class fidelity
actor the Phase-1 survey didn't examine. The negotiator's job is not "pick one
descriptor" but "choose the path that minimizes total loss," per page.

## 9. The classifier already exists — relocate, don't reinvent

`zenpixels-convert` already contains a **provenance-aware, descriptor-only fidelity
classifier** far richer than JXL's decode-only slice or zencodec's naive
`negotiate_pixel_format`:

- **`conversion_cost_with_provenance(from, to, prov) -> ConversionCost{effort, loss}`**
  (`negotiate.rs:493`). **`loss == 0` IS the lossless predicate.**
- **`LossBucket::from_model_loss`** (`pipeline/path.rs:41`): `Lossless ≤10 ·
  NearLossless ≤50 · LowLoss ≤150 · Moderate ≤400 · High`.
- Supporting: `PixelDescriptor::layout_compatible`, `descriptors_match`
  (`output.rs:520`), `requires_cms`, `ConvertStep::Identity`.

### The fidelity map (loss = 0 ⇒ lossless)

| Conversion | Class | Condition |
|---|---|---|
| BGRA↔RGBA swizzle, add-alpha, Gray→RGB/RGBA replicate | **Lossless** | always |
| drop-alpha | Lossy(50) | **Lossless iff alpha Opaque/Undefined** |
| RGB→Gray (Y′ luma) | Lossy(500) | exact only if R==G==B |
| precision widen U8→U16→F32 | **Lossless** | `target_bits ≥ origin_bits` |
| precision narrow | Lossy | **Lossless iff provenance origin already fit** |
| transfer sRGB↔Linear / gamma | **Lossless** (f32) | always |
| transfer **PQ/HLG** | Lossy(300) | *blanket over-charge* — decode is actually lossless |
| gamut map | Lossless | **iff `dst.contains(src)`** (widening); clip = Lossy(80–200) |
| alpha premul/unpremul | NearLossless(5/10) | division rounding |
| Oklab round-trip (f32) | **Lossless** | known primaries |
| HDR tone-map | Lossy | irreversible; `hdr-experimental` only |

The **provenance round-trip rule** is the important subtlety: narrowing F32→U8 is
`Lossless` when the data *originated* as U8 (widened earlier in the chain). This is
exactly what a decode→convert→encode pipeline needs to avoid double-counting.

### Where it belongs

The classifier reads **only descriptor fields** (`primaries, transfer,
channel_type, layout, alpha, signal_range`) + a `Provenance` derived from the
source — it never touches `ColorContext`, so it is **`no_std`-able**. Today it's
trapped in the `std`/CMS convert crate, which zencodec (no_std) can't depend on.

**Recommendation: move the classifier down into `zenpixels`** (its natural home —
the fidelity relationship between two `PixelDescriptor`s is a `PixelDescriptor`
concern), e.g. `zenpixels::fidelity::conversion_cost(from, to, prov)` or
`PixelDescriptor::fidelity_to(target, prov)`. Then **one** classifier serves
zencodec's decode/encode negotiation, every decoder, and zenpixels-convert's
`best_match`. zencodec's naive `negotiate_pixel_format` becomes a thin call into it
(or is deleted). This is a *move + small API*, not a from-scratch build.

### Two-tier: descriptor classifier + CMS resolution (the ICC caveat)

The classifier is **blind to custom-ICC-vs-custom-ICC**: two different ICC profiles
sharing `(primaries, transfer)` score as identity/lossless, even when moxcms would
gamut-clip between them (`color_profile_source()` is a `const fn` returning only a
`PrimariesTransferPair` — it structurally cannot carry ICC bytes). So fidelity is
two-tier:
- **Tier 1 — descriptor classifier** (no_std, fast): exact for named/CICP
  colorimetry — the common case.
- **Tier 2 — CMS `ColorContext` check** (std, moxcms): required only when both
  sides carry custom ICCs with matching descriptors. Resolve at the CMS boundary.

## 10. `StorageFidelity` is still needed — the classifier can't derive it

The per-image classifier (arrow-2 and the source↔candidate part of arrows 1/3)
does **not** replace the static encode tag, because a codec's *internal*
downconvert is invisible in the descriptor: WebP advertises `RGBF32` but stores
8-bit. That is a codec-implementation fact, not a descriptor relationship.

So the two-arrows split stands:
- **Static, encode-side** — `StorageFidelity { Exact, Lossless, Downconverts }` on
  each `SupportedDescriptor` in the encode list (candidate→codec-storage;
  source-independent). Filters the candidate set to the fidelity envelope.
- **Computed, both sides / all arrows** — the classifier returns the per-image
  `DescriptorSupport` (source→candidate). This is the "native/lossless changes per
  image" result; it must never be baked into a static list.

Decode `supported_descriptors()` stays `&[PixelDescriptor]` (nativeness is
source-relative → computed). Only the encode list gains the static tag.

## 11. Multi-page (TIFF, PDF) — the pipeline runs PER PAGE

- **Pages genuinely differ.** TIFF IFDs each independently declare
  depth/channels/photometric/alpha/ICC (image-tiff does *not* inherit tags across
  IFDs — `image.rs:180`); page 1 gray-8, page 2 RGB-16, page 3 CMYK, page 4 RGBA
  are all valid. PDF pages differ in dims + source color spaces.
- **But both decoders flatten.** TIFF keeps 8/16/f32 gray/RGB/RGBA, **collapses
  CMYK/YCbCr/Lab/Palette → RGB(A)** (CMYK is absent from `TIFF_DECODE_DESCRIPTORS`).
  PDF renders **every page to RGBA8** (hayro→vello_cpu). TIFF is at
  `zenextras/zentiff` (fully wired decode+encode); PDF at `zenextras/zenpdf`
  (decode-only, `Custom` format).
- **`MultiPageDecoder` is vaporware** — zero code; only a sketch in
  `docs/multi-image-design.md:368` (`page_info(index) -> ImageInfo`,
  `decode_page(index, preferred, stop) -> DecodeOutput`, per-page `preferred`).
- **The model gap:** `ImageInfo` is single-valued (one `source_color`, one
  `has_alpha` = "primary image only", `info.rs:434`). Per-page descriptors have
  nowhere to go and no enumeration API. `DecodeJob::with_start_frame_index` exists
  (no-op default); PDF overrides it (page-select), **TIFF does not** (locked to
  IFD0 through zencodec despite image-tiff's `seek_to_image` random access).
- **CMYK→CMYK transcode is impossible today.** `PixelDescriptor` models CMYK
  first-class (`ChannelLayout::Cmyk`, `CMYK8`), but neither decoder emits it — the
  channels are RGB-ified before any encoder sees them, and probe never reports
  CMYK. Preserving it needs (a) a CMYK-preserving decode mode via `preferred` (TIFF
  ignores `preferred` today), and (b) per-page descriptor reporting.

**Requirement:** implement the sketched `MultiPageDecoder` so the source-aware
pipeline (decode→convert→encode, §8) runs per page with per-page `preferred`. The
negotiator is already per-image; multi-page just iterates it with a per-page
source descriptor.

## 12. Color management — retag vs CMS, and a live corruption hazard

Color splits cleanly into the two-arrows model, but **both halves are landed-yet-
unwired for transcode**:

- **Signaling retag (arrow-3 metadata) — LOSSLESS.** `resolve_color_emit`
  (`color.rs:247`) decides ICC-vs-CICP emission; "orthogonal to which pixels are
  written." Handles CMYK/gray terminals (keep ICC, suppress RGB CICP). Every policy
  (Compatibility/Balanced/Compact/Verbatim/Custom) is a retag — **no policy
  recolors** P3/PQ→sRGB. **Zero production callers today.**
- **Pixel CMS (arrow-2) — Lossless or Lossy.** Gamut-widen = Lossless; gamut-clip,
  TRC remap w/ quantization, HDR→SDR tone-map = Lossy. Lives in zenpixels-convert
  (moxcms wired behind `cms-moxcms`). But `finalize_for_output*` (the lowering that
  would consume a `ColorEmitPlan` and run source-driven CMS) has **zero production
  callers**; only zenpipe's *explicit* ICC→ICC `IccTransform` node runs CMS. There
  is **no decode-side `ConvertToSrgb`/`ColorIntent` knob** in zencodec at all.

**Retag-vs-Lossy rule:** a transfer/primaries change is a **Native retag** iff the
pixels already carry the target's transfer AND primaries (`descriptors_match` →
byte copy) or you are only relabeling metadata. It is a **Lossy transform** when
primaries narrow (gamut clip), a TRC is remapped with quantization, or an HDR
transfer meets an SDR target (tone-map). Widening depth/gamut without clip is the
only Lossless-transform middle ground. HDR→HDR (PQ→PQ, same primaries) = lossless
retag.

### ⚠ Live corruption hazard — HDR→SDR fails OPEN

On a default (non-`hdr-experimental`) build, a PQ/HLG→sRGB `RowConverter` takes the
no-tone-map arms and **hard-clips every highlight to 1.0 silently**
(`convert.rs:549-566,1835-1844`); the refusal guard that would stop it is itself
feature-gated. Runner-up: the gamut step is skipped when either primaries is
`Unknown` (`convert.rs:664`), silently relabeling wide-gamut pixels as sRGB. Both
violate "the user's pixels are sacred" for any transcode that crosses HDR→SDR or
wide→narrow gamut.

**Design rule for the negotiator:** an HDR→SDR or gamut-narrowing descriptor
transition must route through an explicit (tone-map / gamut-map) convert step and
be reported as `Lossy` — or the transcode must **refuse** (error), never silently
clip. The negotiator's transfer/primaries gates (§5.2) are exactly this boundary;
they must be enforced, not bypassed.

## 13. Revised architecture & trait changes (supersedes §6)

1. **Relocate the fidelity classifier** `conversion_cost`/`LossBucket`/component
   fns from `zenpixels-convert` into **`zenpixels`** (no_std). One classifier for
   decode negotiation, encode negotiation, decoders, and convert.
2. **zencodec `negotiate_pixel_format(source, preferred, available)`** becomes a
   thin wrapper over the relocated classifier (widen-only, native fallback, never
   flatten — §5.2), returning `Negotiated { descriptor, fidelity: DescriptorSupport }`.
   Delete the precision-blind version.
3. **Encode `supported_descriptors() -> &[SupportedDescriptor]`** with the static
   `StorageFidelity` tag (§10). Decode list unchanged. `From<PixelDescriptor>`
   defaults to `Exact` for mechanical migration; codecs downgrade their
   accept-convert entries.
4. **`ImageInfo.source_descriptor: Option<PixelDescriptor>`** — best-effort
   transparency (Phase-1 §1 tiers); authoritative source is the decoder-native
   descriptor.
5. **`MultiPageDecoder`** (the sketched trait) — per-page `page_info` +
   `decode_page(index, preferred)`, so the pipeline runs per page. Wire TIFF's
   `seek_to_image` page selector; add CMYK-preserving decode via `preferred`.
6. **Color: wire the decision→mechanism path.** Call `resolve_color_emit` on the
   transcode encode side (signaling), and route pixel color changes through the
   convert stage's CMS with the Tier-2 `ColorContext` check for custom ICCs. Close
   the HDR→SDR fail-open (§12) — tone-map or refuse, never silent-clip.
7. **Two-tier fidelity** — descriptor classifier (no_std, Tier 1) + moxcms
   `ColorContext` resolution (std, Tier 2) for custom-ICC pairs.

**Rollout stays additive** where possible; the classifier relocation + encode
`SupportedDescriptor` type change are the two breaking pieces (pre-0.1, allowed).
The testkit conformance check (§6) extends to a per-page + HDR-refusal case.
