# Downstream Users of zencodec-types

Audit date: 2026-03-08. Covers all users under `~/work/` and `~/work/zen/`.

## Downstream Map

### Codec Crates (implement traits)

| Crate | Path | Version | Dep Type | Encode | Decode | Stream | Animation | SourceEncoding |
|-------|------|---------|----------|--------|--------|--------|-----------|----------------|
| zenjpeg | `zen/zenjpeg/zenjpeg` | 0.6.1 | required | yes | yes | yes | N/A | yes |
| zenpng | `zen/zenpng` | 0.1.0 | required | yes | yes | yes | yes (APNG) | yes |
| zenavif | `zen/zenavif` | 0.1.0 | optional `zencodec` | yes | yes | yes | decode only | yes |
| zenwebp | `zen/zenwebp` | 0.3.2 | optional `zencodec` | yes | yes | no | yes | yes |
| zenjxl | `zen/zenjxl` | 0.1.0 | optional `zencodec` | yes | yes | stub | yes | yes |
| zengif | `zen/zengif` | 0.6.0 | optional `zencodec` | yes | yes | stub | yes | yes |
| zenbitmaps | `zen/zenbitmaps` | 0.1.0 | optional `zencodec` | yes (PNM/BMP/FF) | yes | stub | N/A | no |
| zentiff | `zen/zentiff` | 0.1.0 | optional `zencodec` | **NONE** | **NONE** | **NONE** | **NONE** | **NONE** |
| heic-decoder-rs | `zen/heic-decoder-rs` | 0.1.0 | optional `zencodec` | N/A | yes | yes | stub | no |
| ultrahdr | `zen/ultrahdr/ultrahdr` | 0.2.0 | optional `zencodec` | no | yes | stub | stub | no |

### Aggregator / Dispatch Crates

| Crate | Path | Version | Role |
|-------|------|---------|------|
| zencodecs | `zen/zencodecs` | 0.1.0 | Umbrella crate, dispatches to codec crates via traits |
| zcimg | `zen/zencodecs/zcimg` | 0.1.0 | CLI tool using zencodecs (**BROKEN: does not compile**) |

### Consumer Crates (use traits / types)

| Crate | Path | Version | Dep Type | Notes |
|-------|------|---------|----------|-------|
| zenimage | `zen/zenimage` | 0.1.0 | required | Uses only `ImageFormat` + `ResourceLimits` bridge; reimplements everything else |
| imageflow4 | `zen/imageflow4/imageflow_core` | 4.0.0-alpha.1 | required | 100% dyn dispatch, legacy + new pipeline |
| imageflow (old) | `work/imageflow/imageflow_core` | 0.1.0 | optional `zen-codecs` | Identical code to imageflow4 adapter layer |
| zensquoosh-codecs | `zen/zensquoosh/crates/zensquoosh-codecs` | 0.1.0 | required (WASM) | **BROKEN: does not compile** |
| coefficient | `work/coefficient` | 0.1.0 | dev-dep only | Uses only `PixelBufferConvertExt` re-export in one example |

---

## Broken Crates (will not compile)

### zcimg (`zen/zencodecs/zcimg`)
- `ImageMetadata` imported but doesn't exist (should be `MetadataView`)
- `encoded.bytes()` called but method is `.data()` or `.into_vec()`
- `decoded.into_rgba8()` / `into_rgb8()` / `into_gray8()` don't exist on `DecodeOutput`
- `ImageFormat::detect()` doesn't exist (should be `ImageFormatRegistry::common().detect()`)

### zensquoosh-codecs (`zen/zensquoosh/crates/zensquoosh-codecs`)
- `decoded.into_rgba8()` / `into_rgb8()` / `into_gray8()` don't exist on `DecodeOutput`
- `zencodec_types::ChannelLayout` doesn't exist (it's in `zenpixels`)
- `rgba.into_contiguous_buf()` doesn't exist on `PixelBuffer`
- `rgba.as_ref()` assumes `ImgVec`, not `PixelBuffer`

### zentiff (`zen/zentiff`) — feature declared, zero implementation
- `zencodec = ["dep:zencodec-types"]` feature exists but no source file references zencodec-types

---

## Cross-Cutting Issues

These patterns repeat across 3+ codecs. Fixing them in zencodec-types or documenting the expected pattern would help all downstream.

### 1. `DecodePolicy` / `EncodePolicy` universally ignored
**Affected:** zenjpeg (partial), zenpng (partial), zenavif, zenwebp, zenjxl, zengif, zenbitmaps, heic-decoder-rs, ultrahdr, imageflow4

Every codec either uses the default no-op `with_policy()` or only partially implements it. Key policy flags silently ignored across the ecosystem:
- `allow_animation` — never checked by any codec
- `allow_progressive` — only zenjpeg could benefit, doesn't check it
- `allow_icc/exif/xmp = false` — only zenpng (partially) and zenjxl (encode only) respect this
- `deterministic` — never checked

**Action:** Either document that policies are advisory, or add a compile-time lint / runtime warning when a codec ignores a non-None policy field.

### 2. `preferred` descriptors ignored or incorrectly negotiated
**Affected:** zenavif (push_decoder, streaming_decoder), zengif, zenbitmaps, heic-decoder-rs, zenjpeg (partially)

Most codecs either `_` the preferred parameter entirely or reimplement format negotiation with different semantics than `zc::decode::negotiate_pixel_format()`. Common issues:
- Preference ordering not respected (zengif checks RGB8 before BGRA8 regardless of caller ranking)
- Push/streaming paths ignore preferences entirely (zenavif)
- All 3 zenbitmaps decoders ignore preferences

**Action:** Consider making `negotiate_pixel_format()` the canonical implementation and documenting it as the expected approach. Possibly add a test helper that validates negotiation correctness.

### 3. `DecodeCapabilities` / `EncodeCapabilities` underreported
**Affected:** All codecs

| Missing capability | Codecs |
|---|---|
| `enforces_max_input_bytes: true` | zenjpeg, zenwebp, zengif |
| `enforces_max_memory: true` | ultrahdr (decode), zenjxl (decode) |
| `row_level: true` (encode) | zenjxl (implements push_rows but doesn't advertise) |
| `native_16bit: true` (encode) | zenjxl |
| `threads_supported_range` | zenjpeg, zenjxl, heic-decoder-rs |
| `native_gray: true` claimed but not truly native | zenavif (expands Gray to RGB before encoding) |
| `native_f32: true` claimed but quantizes to u8/u16 | zenavif |
| `lossy: true` unconditional but feature-gated | zenpng (requires quantize/imagequant/quantette feature) |

**Action:** Add a CI test that validates capabilities match actual behavior.

### 4. `ImageInfo` incompletely populated
**Affected:** All codecs

Common missing fields:
- `bit_depth` / `channel_count` — missing from zenpng, zenjxl, ultrahdr, heic-decoder-rs, zenbitmaps
- `has_animation` / `frame_count` in cheap `probe()` — missing from zengif (has it in probe_full but not probe)
- `has_gain_map` / `gain_map_metadata` — missing from ultrahdr (despite having the data)
- CICP / ICC in probe — missing from ultrahdr, partially from heic-decoder-rs

**Action:** Consider a `ImageInfo` completeness lint or test helper.

### 5. `preferred.to_vec()` unnecessary allocation
**Affected:** zenjpeg, zenwebp, zenavif, heic-decoder-rs

All these codecs clone the preferred descriptor slice into a `Vec` to store on the decoder struct. The list is typically 1-4 descriptors and consumed once. Could use `SmallVec<[PixelDescriptor; 4]>` or borrow `&'a [PixelDescriptor]`.

### 6. Error chain broken for `UnsupportedOperation` / `LimitExceeded`
**Affected:** zenjpeg, ultrahdr, zenavif

Error variants wrap inner types without `#[source]`, so `CodecErrorExt::unsupported_operation()` and `find_cause::<LimitExceeded>()` traversal cannot find them. This breaks dyn dispatch error inspection.

### 7. `SourceEncodingDetails` not implemented
**Affected:** zenbitmaps, heic-decoder-rs, ultrahdr

These codecs don't attach source encoding details despite having relevant info (BMP bit depth, HEIC QP, etc.).

### 8. Stop token not checked in streaming/animation paths
**Affected:** zenjpeg (streaming/push), zenpng (APNG finish), zenjxl (decode), heic-decoder-rs

Cancellation tokens accepted but not propagated to long-running inner loops.

### 9. `output_info()` returns incorrect or placeholder data
**Affected:** ultrahdr (returns 0x0), zenpng (ignores grayscale)

---

## Per-Crate Findings

### zenjpeg (v0.6.1) — `zen/zenjpeg/zenjpeg`

**Traits:** Full encode + decode + streaming decode. No animation (correct).

| # | Finding | Severity |
|---|---------|----------|
| 1 | `UnsupportedOperation` variant missing `#[source]` — breaks `CodecErrorExt` chain | Medium |
| 2 | `enforces_max_input_bytes` not reported despite being enforced | Low |
| 3 | `allow_progressive` from `DecodePolicy` ignored | Medium |
| 4 | Stop token not checked in streaming/push decode paths | Medium |
| 5 | f32 decode copies entire buffer (bytemuck alignment prevents zero-copy cast_vec) | Low |
| 6 | Gray f32 decode: per-pixel map instead of bytemuck cast | Low |
| 7 | `push_rows` buffers entire image (needs `with_canvas_size` to fix) | Low |
| 8 | `JpegDecoderConfig::limits` field is dead code | Low |
| 9 | `threads_supported_range` not set despite rayon support | Low |

### zenpng (v0.1.0) — `zen/zenpng`

**Traits:** Full encode + decode + streaming decode + APNG animation. Most complete integration.

| # | Finding | Severity |
|---|---------|----------|
| 1 | `DecodePolicy` mostly ignored (only `strict` maps to CRC checking) | Medium |
| 2 | `EncodePolicy::allow_animation` and `deterministic` ignored | Low |
| 3 | `output_info()` ignores grayscale — always reports RGB/RGBA | Medium (bug) |
| 4 | `convert_info()` doesn't populate `bit_depth` or `channel_count` | Low |
| 5 | `FullFrameEncoder` only handles 3 of 10 advertised pixel formats | Medium |
| 6 | `FullFrameDecoder` copies file data even when `Cow::Owned` | Low (perf) |
| 7 | APNG finish discards caller's stop token, uses `Unstoppable` | Medium (bug) |
| 8 | Hardcoded 120s timeout on all encode ops, not configurable | Low |
| 9 | BGRA8 swizzle via per-pixel `flat_map` + collect (allocation) | Low (perf) |
| 10 | `with_lossy(true)` unconditional but depends on quantizer features | Low |
| 11 | Streaming decoder rejects `Cow::Owned` data | Low |

### zenavif (v0.1.0) — `zen/zenavif`

**Traits:** Encode + decode + streaming decode. No animation encode.

| # | Finding | Severity |
|---|---------|----------|
| 1 | Decode errors wrapped as `Error::Encode` (wrong variant) | Medium (bug) |
| 2 | `push_decoder` and `streaming_decoder` ignore `preferred` parameter | Medium |
| 3 | `apply_descriptor_color` clones config up to 4x (deep clone with Vec fields) | Medium (perf) |
| 4 | Double data copy in `full_frame_decoder` (probe + animation each copy) | Medium (perf) |
| 5 | `build_config` copies EXIF/ICC/XMP metadata unnecessarily | Low (perf) |
| 6 | Gray8 encode expands to 3x RGB buffer despite claiming `native_gray` | Medium |
| 7 | `native_f32: true` claimed but always quantizes to u8/u16 | Low |
| 8 | Format negotiation reimplemented instead of using `negotiate_pixel_format` | Low |
| 9 | `loop_count()` not implemented on `FullFrameDecoder` despite being available | Low |
| 10 | BGRA->RGBA swizzle allocates new Vec (could be in-place) | Low (perf) |

### zenwebp (v0.3.2) — `zen/zenwebp`

**Traits:** Full encode + decode + animation. Row-level streaming encode.

| # | Finding | Severity |
|---|---------|----------|
| 1 | Double pixel conversion on first `push_rows` call (Raw path) | Low (perf) |
| 2 | Gray8 encode: per-byte `flat_map` expansion to RGB | Low (perf) |
| 3 | GrayF32 encode: two allocations (gray u8 → RGB) | Low (perf) |
| 4 | Decode format negotiation incomplete — doesn't use prefs to choose RGB vs RGBA | Medium |
| 5 | `enforces_max_input_bytes` not reported despite being enforced | Low |
| 6 | `with_policy` not implemented — metadata policy ignored | Low |
| 7 | `FullFrameEncoder::finish()` hardcodes 100ms last frame duration | Medium (bug) |
| 8 | Quality calibration table — **well done**, good cross-codec consistency | (positive) |
| 9 | Resource limits thoroughly forwarded | (positive) |

### zenjxl (v0.1.0) — `zen/zenjxl`

**Traits:** Full encode + decode + animation. Streaming decode stubbed.

| # | Finding | Severity |
|---|---------|----------|
| 1 | `build_pixel_data` double-copies for U16/F32 (chunk + reconstruct + from_pixels) | Medium (perf) |
| 2 | `generic_quality()` returns calibrated JXL value, not original generic value | Medium (bug) |
| 3 | `row_level` encode capability not advertised despite being implemented | Medium |
| 4 | `FullFrameDecoder::info()` panics before first `render_next_frame` | Medium (bug) |
| 5 | `DecodePolicy` silently ignored | Low |
| 6 | `enforces_max_memory` not reported despite being enforced | Low |
| 7 | `native_16bit` encode capability not reported | Low |
| 8 | `threads_supported_range` not set despite threading support | Low |
| 9 | Decode stop token stored but never checked (field prefixed `_stop`) | Low |

### zengif (v0.6.0) — `zen/zengif`

**Traits:** Full encode + decode + animation. Streaming decode stubbed.

| # | Finding | Severity |
|---|---------|----------|
| 1 | `Box::leak` x2 per animation encode (memory leak) | High |
| 2 | `probe_full()` LZW-decompresses every frame just to count them | Medium (perf) |
| 3 | `probe()` omits `frame_count`/`has_animation` on ImageInfo | Medium (bug) |
| 4 | `probe()` ignores job-level limits (only uses config limits) | Medium (bug) |
| 5 | `ResourceLimits::max_frames` not mapped to zengif `Limits::max_frame_count` | Medium |
| 6 | Custom format negotiation with wrong preference ordering | Low |
| 7 | `has_alpha` hardcoded to `true` (correct for output but loses source info) | Low |
| 8 | RGBA8 encode path: `cast_slice` + `.to_vec()` = unnecessary copy | Low (perf) |
| 9 | `DecodePolicy`/`EncodePolicy` not used | Low |
| 10 | Error variant `InvalidEncoderState` used for decoder errors | Low (naming) |

### zenbitmaps (v0.1.0) — `zen/zenbitmaps`

**Traits:** Encode + decode for PNM, BMP, Farbfeld. No animation/streaming.

| # | Finding | Severity |
|---|---------|----------|
| 1 | `has_alpha` misses `Rgba16` in 3 places — farbfeld always reports `has_alpha: false` | Medium (bug) |
| 2 | `PNM_DECODE_DESCRIPTORS` claims RGBA16 support but downscales 16-bit to 8-bit | Medium (bug) |
| 3 | `layout_to_pixel_buffer` always copies — no zero-copy path | Low (perf) |
| 4 | RgbF32 promoted to RgbaF32 (33% memory inflation, no RGBF32 descriptor) | Low (perf) |
| 5 | All decoders ignore `preferred` parameter | Low |
| 6 | No `SourceEncodingDetails` | Low |
| 7 | No `source_color` / `bit_depth` / `channel_count` on ImageInfo | Low |
| 8 | BMP capabilities missing `native_gray` | Low |
| 9 | `linear-srgb` dependency is unused | Low |

### heic-decoder-rs (v0.1.0) — `zen/heic-decoder-rs`

**Traits:** Decode + streaming decode. No encode. No animation (stubbed).

| # | Finding | Severity |
|---|---------|----------|
| 1 | HEIF container parsed up to 5 times per decode (ICC, EXIF, XMP, gain map, header) | High (perf) |
| 2 | 16-bit path: two intermediate allocations (Vec\<u16> → Vec\<Rgba\<u16>> → PixelBuffer) | Medium (perf) |
| 3 | Grid streaming copies all tile data upfront via `into_owned()` | Medium (perf) |
| 4 | `probe()` eagerly extracts all metadata (should be cheap header-only) | Medium |
| 5 | `DecodePolicy` completely ignored | Low |
| 6 | No `SourceEncodingDetails` | Low |
| 7 | CICP truncation from u16 to u8 (non-standard values could be lost) | Low |
| 8 | `enforces_max_input_bytes` not reported/enforced | Low |
| 9 | `StreamingDecode` non-grid fallback does full decode upfront (no memory savings) | Low |
| 10 | HEIC image sequences (msf1) not exposed via ImageInfo | Low |

### ultrahdr (v0.2.0) — `zen/ultrahdr/ultrahdr`

**Traits:** Decode only. Streaming/animation stubbed.

| # | Finding | Severity |
|---|---------|----------|
| 1 | `output_info()` returns width=0, height=0 placeholder | Medium (bug) |
| 2 | `DecodeCapabilities` all false (nothing reported despite enforcement) | Medium |
| 3 | Pixel data `.to_vec()` copies entire buffer (could take ownership) | Medium (perf) |
| 4 | `probe()` missing gain map metadata on ImageInfo | Medium |
| 5 | `decode()` ImageInfo missing ICC/CICP/bit_depth/has_alpha | Medium |
| 6 | Error chain broken — `Jpeg(String)` loses original error | Low |
| 7 | No encoder implementation despite ultrahdr supporting encoding | Low (future) |

### zencodecs (v0.1.0) — `zen/zencodecs`

**Role:** Umbrella dispatch crate. Uses traits, does not implement them.

| # | Finding | Severity |
|---|---------|----------|
| 1 | **zcimg subcrate does not compile** (4+ API mismatches) | High |
| 2 | AVIF decode doesn't forward limits to job (inconsistent with other codecs) | Medium |
| 3 | UltraHDR RGB→RGBA encode: per-pixel `extend_from_slice`, 16MB alloc for 1MP | Medium (perf) |
| 4 | `encode_rgba8` scans every pixel for alpha (no opaque fast path) | Low (perf) |
| 5 | Doesn't query `CodecCapabilities` | Low |
| 6 | Doesn't use `SourceEncodingDetails` from decode output | Low |
| 7 | No `StreamingDecode` / `FullFrameDecoder` usage (documented limitation) | Low |
| 8 | No `DecodePolicy` / `EncodePolicy` forwarding | Low |

### zenimage (v0.1.0) — `zen/zenimage`

**Role:** High-level image processing. Parallel type ecosystem.

| # | Finding | Severity |
|---|---------|----------|
| 1 | Massive type duplication — reimplements nearly every zencodec-types concept | Architectural |
| 2 | `ResourceLimits` conversion loses information in both directions | Medium |
| 3 | No format negotiation | Low |
| 4 | `pub mod zencodec { pub use zc::*; }` re-export appears to be dead code | Low |
| 5 | Uses only `ImageFormat` + `ResourceLimits` from zencodec-types | Note |

### imageflow4 (v4.0.0-alpha.1) — `zen/imageflow4/imageflow_core`

**Role:** Image pipeline. 100% dyn dispatch.

| # | Finding | Severity |
|---|---------|----------|
| 1 | Metadata not passed to encoders — EXIF/ICC/XMP silently dropped | High |
| 2 | Stop token not passed to dyn decode/encode jobs (no cancellation) | High |
| 3 | `multiple_frames` always true for animation-capable formats | Medium (bug) |
| 4 | Magic byte detection duplicated instead of using `ImageFormat::from_magic()` | Medium |
| 5 | `row_to_bgra()` scalar loop not vectorized | Low (perf) |
| 6 | ICC profile cloned (to_vec) instead of Arc ref-count bump | Low (perf) |
| 7 | `encode_srgba8()` hot path not used | Low |
| 8 | No `DecodePolicy`/`EncodePolicy` usage | Low |
| 9 | `has_more_frames()` eagerly decodes next frame (doubles peak frame memory) | Low (perf) |

### imageflow (old, v0.1.0) — `work/imageflow/imageflow_core`
Identical adapter code to imageflow4. All findings from imageflow4 apply.

### coefficient (v0.1.0) — `work/coefficient`
Dev-dep only. Uses `PixelBufferConvertExt` re-export in one example. Should import from `zenpixels-convert` directly.

### zensquoosh-codecs (v0.1.0) — `zen/zensquoosh/crates/zensquoosh-codecs`
**BROKEN.** Does not compile. See "Broken Crates" section above.

---

## Rename Migration Notes (zencodec-types → zencodec)

### Import paths to update

All crates use one of these patterns:
```toml
# Cargo.toml — most common
zencodec-types = { path = "../zencodec-types" }

# Cargo.toml — with lib rename
zc = { path = "../../zen/zencodec-types", package = "zencodec-types" }

# Cargo.toml — with version
zencodec-types = { version = "0.1", path = "../zencodec-types" }
```

In source, the import alias is always `zc` (via `use zencodec_types as zc` or Cargo rename).

### Breaking change surface

If the crate is renamed to `zencodec`:
- Crates with `zencodec = ["dep:zencodec-types"]` just change to `zencodec = { path = "..", optional = true }` — Cargo's implicit feature from the dep name replaces the explicit one. No feature-name conflict.
- `zenimage` has `pub mod zencodec { pub use zc::*; }` which would conflict with a crate named `zencodec`.
- `zensquoosh-codecs` imports `zencodec_types::ImageFormat` directly — would need `zencodec::ImageFormat`.
- `coefficient` imports `zencodec_types::PixelBufferConvertExt` — should switch to `zenpixels_convert` anyway.

### Crates requiring changes (by effort)

**Trivial** (Cargo.toml path only, feature works as-is):
- zentiff, zenbitmaps, ultrahdr, zensquoosh-codecs, coefficient
- zenavif, zenwebp, zenjxl, zengif, heic-decoder-rs (just delete explicit feature def, change dep name)

**Small** (Cargo.toml + source import renames):
- zenjpeg, zenpng, zencodecs (update `use zencodec_types` / `package = "zencodec-types"` references)

**Medium** (architectural touches):
- zenimage (`pub mod zencodec` conflicts with crate name — rename module)
- imageflow4 / imageflow (multiple import sites + adapter layer)
