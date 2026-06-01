# zencodec

Shared traits and types for zen* image codecs.

## API Specification

**[spec.md](docs/spec.md)** — canonical reference for the full public API surface.
Read this before modifying any traits.

## Purpose

Tiny, stable crate defining the common interface that all zen* codecs implement:

- **Encode**: `EncoderConfig` → `EncodeJob` → `Encoder` (type-erased, accepts any `PixelSlice`)
- **Encode animation**: `EncodeJob` → `AnimationFrameEncoder` (push frames one at a time)
- **Decode**: `DecoderConfig` → `DecodeJob` → `Decode` (one-shot), `StreamingDecode` (scanline batches), or `AnimationFrameDecoder` (animation)
- **Dyn dispatch**: `DynEncoderConfig` / `DynDecoderConfig` for codec-agnostic pipelines
- **Metadata**: `ImageInfo`, `Metadata`, `OutputInfo`, `Orientation`
- **Format detection**: `ImageFormat::from_magic()`, `ImageFormatRegistry`
- **Capabilities**: `EncodeCapabilities` / `DecodeCapabilities` (const-constructible flag structs)
- **Errors**: `UnsupportedOperation`, `CodecErrorExt` (error chain inspection)
- **Re-exports**: `enough` (cooperative cancellation), `Cicp`/`ContentLightLevel`/`MasteringDisplay` (from zenpixels)

## Design Rules

- `#![no_std]` + `alloc` — must build on wasm32
- `#![forbid(unsafe_code)]`
- Codec feature gates the trait hierarchy; pixel/metadata types always available
- No codec-specific types here (those live in codec crates)
- No `CodecError` here — each codec has its own error type (associated type on trait)
- Traits use GATs for lifetime-parameterized Job types
- `EncodeJob::Enc`/`AnimationFrameEnc` have NO trait bounds — codecs implement whichever
  encode approach they support (type-erased `Encoder`, animation, or both)
- **zenpixels pixel types: use but NEVER re-export.** `PixelDescriptor`, `PixelSlice`,
  `PixelSliceMut`, `PixelBuffer`, `PixelFormat`, `ChannelLayout`, `ChannelType`,
  etc. are defined in `zenpixels` and used as the cross-crate interchange format.
  All zen crates depend on `zenpixels` directly. zencodec uses these types
  in trait signatures but must not re-export them — callers import from `zenpixels`.
- **zenpixels color metadata types: re-export is OK.** `Cicp`,
  `ContentLightLevel`, and `MasteringDisplay` appear in zencodec's public
  API types. Re-exporting avoids forcing callers to add zenpixels as a
  direct dependency just for these types.

## Key Design Decisions

- **Type-erased encode**: `Encoder` accepts `PixelSlice<'_>` (type-erased, any format). Codecs do runtime dispatch internally. No per-format encode traits.
- **`StreamingDecode`**: Pull-based scanline iterator. `impl StreamingDecode for ()` is the rejection stub for codecs that don't support streaming.
- **Decode format negotiation**: Caller provides ranked `&[PixelDescriptor]` preference list. Decoder picks best match without lossy conversion.

## Release Requirements

**CI MUST pass before any crates.io release.** This includes:
- All tests pass on Linux, Windows, macOS
- WASM build succeeds (wasm32-wasip1)
- Clippy clean (no warnings)
- Format check passes
- MSRV 1.93 check passes
- `cargo-semver-checks` passes (no unintended breaking changes)

**Before publishing:**
1. Verify README.md reflects current API
2. Run `cargo semver-checks check-release` locally
3. Bump version in Cargo.toml
4. Get explicit user approval
5. `cargo publish`

## Known Issues

Three bugs verified during the cross-codec color/metadata scenario-matrix
research (2026-06-01). The first is in this crate; the other two are recorded
here as cross-repo findings (do NOT edit those repos from here — flag to the
owner). Full design context: [`docs/color-emit-model.md`](docs/color-emit-model.md).

1. **Double-rotation hazard (this crate, `src/metadata.rs`).** When a decoder
   bakes orientation upright it sets `Metadata::orientation = Identity`, but the
   EXIF blob still carries the `Orientation` tag (e.g. `6`). `Metadata::filtered`
   keeps that tag, so the field says `Identity` while the blob says `Rotate90` —
   they disagree, and a consumer that re-applies the EXIF tag rotates twice. The
   test at `src/metadata.rs:816` currently locks in keeping the stale tag. The
   byte-level fix now exists — `helpers::set_exif_orientation(blob, 1)` rewrites
   the inline tag offset-preservingly. **Still TODO:** the pipeline (the layer
   that bakes orientation) must actually call it on the emitted blob, and the
   `metadata.rs:816` test should be updated to expect a rewritten tag, not a
   stale one. This is a pipeline-applied fix, not a `Metadata::filtered` change.

2. **AVIF descriptor-CICP override (zenavif, `src/codec.rs:824-831`).**
   `apply_descriptor_color` overrides a metadata-set CICP unconditionally,
   ignoring a CICP explicitly provided via `Metadata`. It should check for a
   caller-supplied CICP before overriding from the pixel descriptor.

3. **Missing signal-range conversion kernels (zenpixels-convert).** No
   `Narrow <-> Full` range conversion kernels exist, so a range mismatch refuses
   zero-copy but can relabel without rescaling — a black-crush risk. Needs
   `ConvertStep::{Expand,Contract}NarrowToFull`. Until then, range must be
   preserved verbatim, never relabeled.
