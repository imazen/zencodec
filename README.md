# zencodec

Shared traits and types for the zen\* image codec family.

This crate defines the common interface that all zen\* codecs implement. It contains no codec logic — just traits, types, and format negotiation helpers. `no_std` compatible (requires `alloc`), `forbid(unsafe_code)`.

Import as `zencodec` — use `zencodec::encode`, `zencodec::decode`, etc.

**Guides:**
- [**Using zen\* codecs**](docs/CONSUMING.md) — encoding, decoding, format negotiation, dyn dispatch, animation, streaming
- [**Implementing a codec**](docs/IMPLEMENTING.md) — how to implement the traits for a new image format

## Crates in the zen\* family

| Crate | Format | Repo |
|-------|--------|------|
| `zenjpeg` | JPEG | [imazen/zenjpeg](https://github.com/imazen/zenjpeg) |
| `zenwebp` | WebP | [imazen/zenwebp](https://github.com/imazen/zenwebp) |
| `zenpng` | PNG | [imazen/zenpng](https://github.com/imazen/zenpng) |
| `zengif` | GIF | [imazen/zengif](https://github.com/imazen/zengif) |
| `zenavif` | AVIF | [imazen/zenavif](https://github.com/imazen/zenavif) |
| `zenjxl` | JPEG XL | [imazen/zenjxl](https://github.com/imazen/zenjxl) |
| `zenbitmaps` | PNM/BMP/Farbfeld | [imazen/zenbitmaps](https://github.com/imazen/zenbitmaps) |
| `heic-decoder-rs` | HEIC/HEIF | [imazen/heic-decoder-rs](https://github.com/imazen/heic-decoder-rs) |
| `zentiff` | TIFF (experimental) | [imazen/zentiff](https://github.com/imazen/zentiff) |
| `zenpdf` | PDF (experimental) | [imazen/zenpdf](https://github.com/imazen/zenpdf) |
| `zencodecs` | Multi-format dispatch | [imazen/zencodecs](https://github.com/imazen/zencodecs) |

## Architecture

Every codec follows a three-layer pattern:

```text
Config     →  reusable, Clone + Send + Sync, 'static
Job        →  per-operation, borrows temporaries (stop token, limits, metadata)
Executor   →  borrows pixel data or file bytes, consumes self to produce output
```

```text
ENCODE:  EncoderConfig → EncodeJob<'a> → Encoder / AnimationFrameEncoder
DECODE:  DecoderConfig → DecodeJob<'a> → Decode / StreamingDecode / AnimationFrameDecoder
```

Config lives in a struct and gets shared across threads. A web server keeps one `JpegEncoderConfig` at quality 85 for all requests. Job borrows stack-local data (cancellation token, resource limits, metadata). Executor borrows pixels or bytes and consumes itself to produce output.

Each layer also has object-safe `Dyn*` variants for codec-agnostic dispatch:

```text
DynEncoderConfig → DynEncodeJob → DynEncoder / DynAnimationFrameEncoder
DynDecoderConfig → DynDecodeJob → DynDecoder / DynStreamingDecoder / DynAnimationFrameDecoder
```

Blanket impls generate the dyn API automatically — codec authors implement the generic traits and get dyn dispatch for free.

## Quick Example

```rust,ignore
use std::borrow::Cow;
use zenjpeg::{JpegEncoderConfig, JpegDecoderConfig};
use zencodec::encode::{EncoderConfig, EncodeJob, Encoder};
use zencodec::decode::{DecoderConfig, DecodeJob, Decode};

// Encode
let config = JpegEncoderConfig::new().with_generic_quality(85.0);
let output = config.job().encoder()?.encode(pixels.as_slice())?;
let jpeg_bytes = output.into_vec();

// Decode
let config = JpegDecoderConfig::new();
let decoded = config.job().decoder(Cow::Borrowed(&jpeg_bytes), &[])?.decode()?;
let pixels = decoded.into_buffer();
```

## Key Design Decisions

**Color management is not the codec's job.** Decoders return native pixels with ICC/CICP metadata. Encoders accept pixels as-is and embed the provided metadata. The caller handles CMS transforms.

**Format negotiation over conversion.** Decoders take a ranked `&[PixelDescriptor]` preference list and pick the first they can produce without lossy conversion. Pass `&[]` for native format.

**Capabilities over try/catch.** Codecs declare their capabilities as const `EncodeCapabilities` / `DecodeCapabilities` structs. Check before calling instead of catching `UnsupportedOperation` errors.

**Pixel types from `zenpixels`.** All pixel interchange types (`PixelSlice`, `PixelBuffer`, `PixelDescriptor`, etc.) are defined in the `zenpixels` crate. All zen\* crates depend on `zenpixels` directly.

## What's in this crate

| Module | Contents |
|--------|----------|
| `zencodec::encode` | `EncoderConfig`, `EncodeJob`, `Encoder`, `AnimationFrameEncoder`, `EncodeOutput`, `EncodeCapabilities`, `EncodePolicy`, dyn dispatch traits |
| `zencodec::decode` | `DecoderConfig`, `DecodeJob`, `Decode`, `StreamingDecode`, `AnimationFrameDecoder`, `DecodeOutput`, `DecodeCapabilities`, `DecodePolicy`, `DecodeRowSink`, dyn dispatch traits, format negotiation |
| root | `ImageFormat`, `ImageInfo`, `Metadata`, `Orientation`, `OrientationHint`, `ResourceLimits`, `UnsupportedOperation` |

## License

Apache-2.0 OR MIT
