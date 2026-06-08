# Scope: gain-map / HDR delivery through the zencodec encode pipeline — 2026-06-08

How to make HDR output (gain-map *and* native) reachable through the codec-agnostic
zencodec encode path, so a `DynEncoder` pipeline can emit HDR without per-codec
native API calls. Scoping only — no implementation here.

---

## HDR delivery in 2026: native vs gain map (what's typical)

Two mechanisms, chosen by container capability and whether SDR backward-compat is needed:

| | **Native HDR** | **Gain-map HDR** |
|---|---|---|
| **What** | High-bit-depth pixels (10/12-bit or float) + PQ (CICP transfer 16) or HLG (18) + BT.2020 primaries (9), signaled in the container's CICP carrier | An SDR base image + a secondary *gain map* + ISO 21496-1 metadata; SDR viewers see the base, HDR viewers multiply by the gain map |
| **SDR fallback** | None (legacy viewers show wrong/clipped color) | Yes (the whole point) |
| **JPEG** | ✗ impossible (8-bit) | ✓ **the only HDR path** (Google UltraHDR) |
| **AVIF** | ✓ **the established path** (inherits AV1 10/12-bit PQ/HLG; what HDR video frames / cameras / screenshots use) | ✓ emerging (ISO 21496-1 `tmap` item; Apple Adaptive-HDR / Adobe — for SDR-compatible HDR photos) |
| **JXL** | ✓ **the primary path** (up to 32-bit float, PQ/HLG, `intensity_target`; JXL was designed for native HDR + lossless HDR-JPEG transcode) | ✓ supported, secondary |
| **PNG** | ✓ signaling only (`cICP` PQ/HLG on 16-bit); niche | ✗ no standard carrier |

**Convention:** ISO 21496-1 gain maps (the convergence of Google UltraHDR + Adobe,
adopted by Apple across HEIC/AVIF/JXL and by Android/Chrome) are becoming the
cross-format **SDR-compatible HDR-photo** mechanism — use them when SDR fallback
matters or the target is JPEG. **Native PQ/HLG** stays the path for AVIF/JXL when
no SDR fallback is needed (video-derived, pro workflows) — simpler and no
tone-map loss. So: **AVIF → traditionally native, gain maps rising; JXL →
overwhelmingly native; JPEG → gain map only.** A general pipeline must do both and
let policy/target decide.

---

## What already works vs the gap

**Native HDR is mostly already wired** — by the color-emit integration (this sweep).
`resolve_color_emit` carries a source CICP (incl. transfer 16=PQ / 18=HLG, primaries
9=BT.2020) to each format's native carrier: JXL enum color (zenjxl already maps
`16→Pq, 18→Hlg` + `intensity_target`), AVIF `nclx`, PNG `cICP`. The remaining
native-HDR need is just letting high-bit-depth/float pixels through the encoder
(the type-erased `PixelSlice` already allows it; the codec must accept the format)
and carrying `Metadata.content_light_level` / `mastering_display`.

**Gain-map HDR is the real gap.** `Encoder::encode(PixelSlice)` takes *one* image;
a gain-map file is a *composite* (base + gain map + ISO 21496-1 metadata). zencodec
has no encode-side way to express "emit this as gain-map HDR." Today it's per-codec
native API with *different input models*:
- `zenjpeg::ultrahdr::encode_ultrahdr(hdr_pixels, gainmap_cfg, tonemap_cfg, enc_cfg, …)` — **HDR-pixels in**, framework tonemaps + computes the gain map (via ultrahdr-core) + assembles UltraHDR.
- `zenavif::EncoderConfig::with_gain_map(av1_data, w, h, depth, metadata)` — **pre-encoded gain-map AV1 in**, serializer builds the `tmap` item.
- zenjxl: native HDR only; no gain-map wiring.

**Type gap:** `zencodec::gainmap` has good *decode-side* types — `GainMapSource`
(encoded gain-map **bytes** + format + `GainMapInfo`, what a decoder extracts) and
`DecodedGainMap` (decoded **pixels** + `GainMapInfo`) — plus `GainMapInfo` /
`GainMapParams` / `parse_iso21496`. There is **no encode-side gain-map-pixels input
type**. The scope must add one (or reuse a `DecodedGainMap`-shaped input).

---

## Proposed design

### Layering — two input models, both needed
- **Low-level (codec contract): pre-composed gain map.** Caller (or the shared
  math) supplies the gain-map **pixels** + `GainMapInfo`; the codec encodes the
  gain map in its own format and assembles the container. Normalize on **pixels**
  (not pre-encoded bytes) so one contract works for all codecs — AVIF encodes the
  pixels to AV1 internally, JXL to a codestream, JPEG to the MPF secondary.
- **High-level (framework convenience): HDR pixels.** Caller hands HDR pixels
  (linear f32 / 10-bit + HDR CICP/CLLI); the framework decides native-vs-gain-map
  per target caps + policy, and for the gain-map route computes (tonemap → SDR base
  + gain map + metadata) via the shared math, then calls the low-level contract.

### Trait surface (recommendation)
1. **New encode-side input type** in `zencodec::gainmap`, e.g.
   `GainMapEncodeSource { base_alternate: …, pixels: PixelBuffer, metadata: GainMapInfo }`
   (gain-map pixels + ISO 21496-1 params). Mirrors `DecodedGainMap` but is an
   *input*. (Do **not** overload the decode-oriented `GainMapSource`.)
2. **Codec contract — set gain map on the job (mirrors `with_metadata_policy`):**
   `EncodeJob::with_gain_map(GainMapEncodeSource) -> Self` (provided default = no-op
   for codecs that can't carry one, like `with_policy`). Then the normal
   `Encoder::encode(base_pixels)` composes base + the job's gain map + metadata.
   Codecs that support it (JPEG/AVIF/JXL) override it; everyone else ignores it.
3. **Framework HDR-pixels convenience** (not a trait method on the codec):
   `encode_hdr(job, hdr_pixels, HdrEmitPolicy)` — picks native vs gain-map per the
   target's `EncodeCapabilities`, computes the gain map via the shared math when
   needed, and drives `with_gain_map` + `encode`. Keeps the gain-map math *out* of
   every codec.
4. **Capabilities:** `EncodeCapabilities::{hdr, gain_map}` already exist. Clarify
   their contract (`gain_map` = "composes an ISO 21496-1 gain-map file via
   `with_gain_map`"; `hdr` = "accepts high-bit-depth/float native HDR pixels +
   PQ/HLG CICP"). Add a testkit `check_gain_map_*` conformance check.
5. **Policy:** an `HdrEmitPolicy` (sibling to `ColorEmitPolicy`), or a field on
   `EncodePolicy`: `PreferGainMap` (SDR-compat; required for JPEG), `PreferNative`
   (AVIF/JXL/PNG; better quality, no fallback), `Auto` (gain-map if the target has
   no native HDR carrier or the caller wants SDR-compat, else native).

### Shared gain-map math (compute once, embed per-codec)
The tonemap + gain-map fit lives in **ultrahdr-core** (already used by
`zenjpeg::ultrahdr`). Lift the "HDR pixels → (SDR base pixels, gain-map pixels,
`GainMapInfo`)" step to a shared helper the framework calls, so every codec only
implements *composition* (`with_gain_map`), never the math. This is the key
decoupling and avoids three divergent gain-map implementations.

### Per-codec mapping
| codec | native HDR (via color-emit) | gain-map (`with_gain_map`) | work to wire |
|---|---|---|---|
| **JPEG** | ✗ (8-bit) | ✓ route to `ultrahdr` assemble | implement `with_gain_map`; encode gain map as MPF secondary; `gain_map` cap = true |
| **AVIF** | ✓ nclx PQ/HLG (already, 10/12-bit) | ✓ route to `zenavif-serialize::set_gain_map` (`tmap`) | implement `with_gain_map`: encode gain-map pixels → AV1, build tmap; `gain_map` cap = true; native already via color-emit |
| **JXL** | ✓ enum PQ/HLG + intensity_target (largely done) | ✓ ISO 21496-1 box (optional) | implement `with_gain_map` if/when jxl-encoder exposes the gain-map box; native already done |
| **PNG** | ✓ cICP PQ/HLG signaling (16-bit) | ✗ | none (native signaling done) |
| WebP / GIF / TIFF / bitmaps | ✗ | ✗ | none (declare both caps false) |

### Decode symmetry (already partly there)
Decode already surfaces gain maps: `DecodeCapabilities::gain_map`, `GainMapSource`
(encoded) / `DecodedGainMap` (pixels) in `DecodeOutput` extras, `GainMapPresence`
on `ImageInfo`. The encode contract should mirror this so a decode→encode transcode
can round-trip a gain map (decode → `DecodedGainMap` → `GainMapEncodeSource` →
`with_gain_map`).

---

## Phasing
1. **Phase 0 — lock native HDR.** Add tests that PQ/HLG (CICP transfer 16/18 +
   BT.2020) + high-bit-depth pixels round-trip through the existing color-emit path
   for AVIF/JXL/PNG. (Likely already works — prove it.)
2. **Phase 1 — the contract.** Add `GainMapEncodeSource` + `EncodeJob::with_gain_map`
   (provided no-op default) + cap semantics + a testkit conformance check. Pure
   zencodec, additive, non-breaking.
3. **Phase 2 — per-codec composition.** Implement `with_gain_map` in zenjpeg
   (→ ultrahdr assemble), zenavif (→ serializer tmap), zenjxl (if box exposed).
   Each takes *supplied* gain-map pixels; no math in the codec.
4. **Phase 3 — framework HDR-pixels convenience + policy.** `encode_hdr` +
   `HdrEmitPolicy`, using the lifted ultrahdr-core math + native-vs-gain-map choice.
5. **Phase 4 — transcode round-trip.** decode `DecodedGainMap` → `with_gain_map`
   so gain maps survive a pipeline transcode; testkit cross-path check.

## Open questions
- **Pixels vs pre-encoded gain map in the contract.** Recommendation: pixels
  (one contract, codecs encode internally). zenavif's current `with_gain_map`
  wants pre-encoded AV1 — the AVIF impl encodes the supplied pixels first. Confirm
  no double-encode quality loss vs accepting pre-encoded for AVIF specifically.
- **Where does `HdrEmitPolicy` live** — its own enum (like `ColorEmitPolicy`) or a
  field on `EncodePolicy`? Leaning standalone, parallel to color.
- **`intensity_target` / CLLI/MDCV plumbing** for native HDR — confirm
  `Metadata.content_light_level` reaches each native encoder (JXL does; verify
  AVIF/PNG).
- **Gain-map math ownership** — lift from `zenjpeg::ultrahdr` into a shared crate
  (ultrahdr-core is the natural home) so JPEG/AVIF/JXL share it; avoid forking.
- **Effort:** Phase 1 ~S (additive types + one provided method + a test), Phase 2
  ~M per codec, Phase 3 ~M (math lift + policy). Native HDR (Phase 0) is mostly a
  test-writing exercise on top of the color-emit work already done.

---

## Caller intent: decode rendition + encode tonemap

**Does intent bloat each codec, or is it a per-codec feature flag? Neither — three
separate layers.** The mistake would be to bake "apply the gain map?" / "tonemap?"
decisions into each codec. Instead:

| layer | nature | who owns it |
|---|---|---|
| **Feature flag** (`ultrahdr`, gain-map, hdr) | *compile-time* — does this build link the gain-map/HDR machinery + its deps (ultrahdr-core, tonemap)? | each codec's Cargo features |
| **Capability** (`EncodeCapabilities`/`DecodeCapabilities::{hdr, gain_map}`) | *runtime reflection* of the flag — "this build can do it" | codec (reports caps) |
| **Intent / policy** (decode render, encode emit) | *runtime caller choice* — what to do given the codec can | **zencodec framework** (one policy type, all codecs honor it) |
| **Math** (apply gain map; tonemap+fit gain map) | shared implementation | ultrahdr-core, not per-codec |

So the feature flag gates the *weight* (gain-map deps aren't free), the capability
reflects it, the **intent is a uniform framework policy** every codec reads, and the
heavy math is shared. A codec's only added surface is "read the policy, pick the
rendition/emission, call the shared math" — it does **not** grow its own intent
vocabulary, and consumers get one consistent knob across formats.

### Decode intent — which rendition? (`DecodePolicy.gain_map`)
A gain-map file (UltraHDR JPEG, AVIF `tmap`, …) can be decoded three ways. Add a
`GainMapRender` to `DecodePolicy` (it stays `Copy` — all small):
```rust
pub enum GainMapRender {
    BaseOnly,                                   // SDR base only; ignore the gain map (DEFAULT)
    ReconstructHdr { target_headroom: Option<f32> }, // base × gain map → HDR (None = full metadata headroom)
    Components,                                  // base pixels + DecodedGainMap + metadata, no compositing
}
```
- **Default `BaseOnly`** is the pit-of-success: an SDR consumer that doesn't ask for
  HDR gets a normal SDR image (and an SDR-sized buffer) — applying the gain map by
  default would surprise callers and force HDR output buffers.
- **`ReconstructHdr`** is opt-in; output is an HDR pixel format (f32 / 10-bit) chosen
  through the existing ranked-`PixelDescriptor` negotiation. `target_headroom` lets a
  caller render for a specific display's HDR headroom (gain maps are display-referred).
- **`Components`** feeds transcode/re-processing (it's what a decode→encode gain-map
  round-trip needs — pairs with `DecodedGainMap` → `GainMapEncodeSource`).
- **Fallback semantics (document):** `ReconstructHdr` on a file with *no* gain map →
  return the image as-is + a warning (it's already the best rendition); on a codec
  without the `gain_map` capability → `UnsupportedOperation` (don't silently hand back
  SDR labeled as HDR — sacred pixels).

### Encode intent — tonemap or not? (`EncodePolicy` HDR emit, = the prior `HdrEmitPolicy`)
"Tonemap or not" is the same decision as native-vs-gain-map:
- `Native` — encode the HDR pixels directly (PQ/HLG via the color-emit CICP path); **no tonemap**. Needs a native-HDR-capable target (AVIF/JXL/PNG).
- `GainMap` — **tonemap** HDR → SDR base + compute the gain map (shared math) + compose. SDR-compatible; the only option for JPEG.
- `Auto` (default) — `Native` if the target has a native HDR carrier and the caller didn't ask for SDR-compat; else `GainMap`; else (SDR-only target like WebP) tonemap-to-SDR with a warning that HDR was lost.
- A caller who supplies a *pre-composed* gain map via `with_gain_map` has already
  tonemapped — the framework just composes (no tonemap step).

### Why this is the right shape
- **One vocabulary, every codec.** Same `DecodePolicy.gain_map` / `EncodePolicy` HDR
  knob whether the target is JPEG, AVIF, or JXL — like `ColorEmitPolicy` and
  `MetadataPolicy` already are. No per-codec intent enums to learn.
- **Honest capabilities gate it.** A build without the gain-map feature reports
  `gain_map=false`; the framework then errors or falls back per the documented rules,
  never silently mis-renders.
- **Testkit conformance.** Add `check_gain_map_render_intent` (decode honors
  BaseOnly/ReconstructHdr/Components or errors honestly) and `check_hdr_emit_intent`
  (encode honors Native/GainMap or errors) — same pattern as the existing capability-
  honesty checks.

### Phasing addition
- Folds into Phase 1 (add `GainMapRender` to `DecodePolicy` + the HDR emit policy to
  `EncodePolicy`, both additive/non-breaking) and Phase 3 (the `encode_hdr`/decode
  paths honor them via the shared math). No new phase.
