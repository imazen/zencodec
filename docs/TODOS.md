# zencodec-types + zen* Codec TODOs

Audit completed 2025-02-12. Updated 2025-02-12 after implementing zencodec-types changes.

## zencodec-types — DONE

All zencodec-types changes have been implemented:

- **Breaking trait changes:** `Send + Sync` bounds, `probe_header()`/`probe_full()` split,
  `capabilities()` method, owned `ResourceLimits` (now `Copy`)
- **CodecCapabilities struct:** encode/decode ICC/EXIF/XMP, cancel, native_gray, cheap_probe
- **GrayF32** variant in `PixelData` with full conversion support
- **DecodeFrame** parity with `DecodeOutput` (into_gray8, as_* methods)
- **Missing derives:** `Eq` on ImageInfo, `PartialEq`+`Eq` on ImageMetadata,
  `Copy`+`PartialEq`+`Eq` on ResourceLimits, `PartialEq`+`Eq` on EncodeOutput
- **16-bit conversion:** proper rounding via `(v * 255 + 32768) >> 16`, sRGB documented
- **Tests:** expanded from 33 to 77
- **MSRV:** bumped to 1.93, CI updated

### Still to consider (this crate)

#### Pixel conversion utilities
- `bytes_to_rgb`, `img_rgb_to_bytes` etc. are copy-pasted across zenjpeg/zenwebp
- Consider adding to zencodec-types as utility functions
- Weigh against: crate lightness, keeping this traits-only
- Could be a separate `zencodec-convert` micro-crate

#### Update design doc to match reality
- `api-design.md` still shows `with_quality`/`with_effort`/`with_lossless` on traits
- `PixelData` in doc has 7 variants, implementation has 10
- `Orientation` not in design doc
- `decode_into_*` methods not in design doc
- `ResourceLimits` struct replaces individual limit methods
- `probe_header`/`probe_full` replaces `probe`
- `CodecCapabilities` not in design doc
- **Files:** `/home/lilith/work/zencodecs/api-design.md`

---

## Codec-Specific Issues (other repos, not yet done)

All codecs need to update their `zencodec.rs` to match the new trait API:
- `probe()` → implement `probe_header()`, optionally override `probe_full()`
- `with_limits(&ResourceLimits)` → `with_limits(ResourceLimits)` (owned)
- Add `fn capabilities() -> &'static CodecCapabilities` to both Encoding and Decoding impls

### zengif

- **Return error on dimension overflow** — `img.width() as u16` silently truncates > 65535
- **Remove dead `quality: Option<f32>` field** — set but never read
- **Split probe** — currently decodes all frames just to count them;
  `probe_header()` should parse GIF header only, `probe_full()` counts frames

### zenavif

- **Fix silent ICC/XMP metadata loss** — `with_metadata` drops ICC and XMP silently
- **Split probe** — currently does a full image decode for probe;
  `probe_header()` should parse AVIF container headers only

### zenpng

- **Set capabilities** — `encode_cancel: false`, `decode_cancel: false` (stop is no-op)

### zenjxl

- **Set capabilities** — `decode_cancel: false` (decode stop is no-op)

### All codecs

- **MSRV 1.93, edition 2024** — align all repos
- **Gray8 native support** — use `capabilities().native_gray()` to indicate;
  codecs that support gray natively should avoid expanding to RGB
- **ResourceLimits enforcement** — document which limits are enforced via capabilities;
  log when limits are requested but not enforced

---

## Not Doing (Decided Against)

- **Quality/effort/lossless on trait** — leave on concrete types
- **`Deref<Target=[u8]>` on EncodeOutput** — `AsRef<[u8]>` is sufficient
