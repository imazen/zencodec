# Zencodec Ecosystem Cleanup Plan

**Goal:** Efficient, low-RAM data passing across the entire codec ecosystem before any API users exist.

**Status: COMPLETE** (Feb 2026). All phases merged to main across 7 repos + zenjxl.

**Scope:** zencodec-types, zenjpeg, zengif, zenwebp, zenavif, zenbitmaps, zenresize, zencodecs.
**Remaining:** zenpng (has uncommitted user work, needs collect_contiguous_bytes removal).

## Phase 1: Buffer Alignment & Stride Contract (zencodec-types) ✅

### 1a. Enforce stride % bpp == 0 ✅
### 1b. DecodeRowSink: stride-aware demand ✅
### 1c. Default decode_rows implementation ✅

## Phase 2: Encoder Stride Support (zencodec-types + codecs) ✅

### 2a. Encoders accept strided input ✅
Via PixelSlice::contiguous_bytes() — Cow-based, zero-copy when tightly packed.

### 2b-c. Remove dead trait methods ✅
next_frame_rows removed from all codecs. decode_rows stubs removed (default impl handles them).

## Phase 3: Dead Type Cleanup (zencodec-types)

### 3a. PixelBuffer — KEEP ✅
Only used in zencodec-types tests, but valid API surface for downstream users.

### 3b-c. PixelData::as_bytes / GrayAlpha — DEFERRED
Requires unsafe Pod impl for GrayAlpha, blocked by #![forbid(unsafe_code)]. Low priority.

## Phase 4: zenresize Stride Support ✅ (already had it)

zenresize already has ResizeConfig::in_stride, push_rows(buf, stride, count), and out_stride.

## Phase 5: zencodecs Integration ✅

Eliminated 2 full-image copies per resize path using ComponentBytes::as_bytes() (input)
and bytemuck::allocation::cast_vec() (output).

## Phase 6: Format Descriptor Bridging — SKIPPED

Only 3 hardcoded PixelFormat match arms in zencodecs pipeline. Not worth a cross-crate bridge.

---

## Invariants (enforced everywhere after cleanup)

**Hard (validated at construction):**
- `stride % bpp == 0` — every row starts pixel-aligned
- `stride >= width * bpp` — no truncated rows
- `row_0_ptr % bpp == 0` — first row is type-aligned
- (Follows) every `row_N_ptr % bpp == 0`

**Soft (opt-in):**
- `stride % 64 == 0` — SIMD-friendly (use simd_aligned_stride/new_simd_aligned)
- `row_0_ptr % 64 == 0` — SIMD-friendly (use AVec or aligned alloc)

**Codec contracts:**
- Decoders write at stride offsets from DecodeRowSink.demand(), never assume tight packing
- Encoders read via PixelSlice::row(y), never assume contiguous memory
- zenresize accepts stride parameter, reads rows at stride offsets
