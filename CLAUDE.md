# zencodec-types

Shared traits and types for zen* image codecs.

## API Specification

**[spec.md](spec.md)** — canonical reference for the full public API surface.
Read this before modifying any traits.

## Purpose

Tiny, stable crate defining the common interface that all zen* codecs implement:

- **Encode**: `EncoderConfig` → `EncodeJob` → `Encoder` (type-erased) or per-format traits (`EncodeRgb8`, `EncodeRgba8`, etc.)
- **Encode animation**: `EncodeJob` → `FrameEncoder` (type-erased) or `FrameEncodeRgba8`, etc.
- **Decode**: `DecoderConfig` → `DecodeJob` → `Decode` (one-shot), `StreamingDecode` (scanline batches), or `FrameDecode` (animation)
- **Pixel types**: `PixelSlice<'a, P>`, `PixelSliceMut`, `PixelBuffer`, `PixelDescriptor`, `PixelFormat`
- **Metadata**: `ImageInfo`, `Metadata`, `MetadataView`, `OutputInfo`, `Orientation`
- **Format detection**: `ImageFormat::from_magic()`
- **Capabilities**: `CodecCapabilities` (const-constructible flag struct)
- **Errors**: `UnsupportedOperation`, `HasUnsupportedOperation`, `At<E>` (via `whereat`)
- **Re-exports**: `rgb`, `imgref`, `enough`, `whereat`, `zenpixels_convert`

## Design Rules

- `#![no_std]` + `alloc` — must build on wasm32
- `#![forbid(unsafe_code)]`
- Codec feature gates the trait hierarchy; pixel/metadata types always available
- No codec-specific types here (those live in codec crates)
- No `CodecError` here — each codec has its own error type (associated type on trait)
- Traits use GATs for lifetime-parameterized Job types
- `EncodeJob::Enc`/`FrameEnc` have NO trait bounds — codecs implement whichever
  encode approach they support (type-erased `Encoder`, per-format, or both)
- **zenpixels types: use but NEVER re-export.** `PixelDescriptor`, `PixelSlice`,
  `PixelSliceMut`, `PixelBuffer`, `PixelFormat`, `ChannelLayout`, `ChannelType`,
  etc. are defined in `zenpixels` and used as the cross-crate interchange format.
  All zen crates depend on `zenpixels` directly. zencodec-types uses these types
  in trait signatures but must not re-export them — callers import from `zenpixels`
  directly. Current re-exports are a migration artifact to be removed.

## Key Design Decisions

- **Type-erased vs per-format encode**: `Encoder` accepts `PixelSlice<'_>` (any format, runtime dispatch). Per-format traits like `EncodeRgb8` accept `PixelSlice<'_, Rgb<u8>>` (compile-time guarantee). A codec can implement either or both.
- **`PixelSlice<'a, P = ()>`**: Default `P = ()` is type-erased. Concrete `P` is typed. `From<PixelSlice<'a, P>> for PixelSlice<'a>` converts typed → erased.
- **`StreamingDecode`**: Pull-based scanline iterator. `impl StreamingDecode for ()` is the rejection stub for codecs that don't support streaming.
- **Decode format negotiation**: Caller provides ranked `&[PixelDescriptor]` preference list. Decoder picks best match without lossy conversion.

## Release Requirements

**CI MUST pass before any crates.io release.** This includes:
- All tests pass on Linux, Windows, macOS
- WASM build succeeds (wasm32-wasip1)
- Clippy clean (no warnings)
- Format check passes
- MSRV 1.85 check passes
- `cargo-semver-checks` passes (no unintended breaking changes)

**Before publishing:**
1. Verify README.md reflects current API
2. Run `cargo semver-checks check-release` locally
3. Bump version in Cargo.toml
4. Get explicit user approval
5. `cargo publish`

## Known Issues

(none)
