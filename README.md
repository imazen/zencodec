# zencodec [![CI](https://img.shields.io/github/actions/workflow/status/imazen/zencodec/ci.yml?branch=main&style=flat-square)](https://github.com/imazen/zencodec/actions/workflows/ci.yml) [![crates.io](https://img.shields.io/crates/v/zencodec?style=flat-square)](https://crates.io/crates/zencodec) [![lib.rs](https://img.shields.io/crates/v/zencodec?style=flat-square&label=lib.rs&color=blue)](https://lib.rs/crates/zencodec) [![docs.rs](https://img.shields.io/docsrs/zencodec?style=flat-square)](https://docs.rs/zencodec) [![license](https://img.shields.io/crates/l/zencodec?style=flat-square)](https://github.com/imazen/zencodec#license)

zencodec is the shared trait crate that defines the common API for all zen\* image codecs.

It contains no codec logic — just traits, types, and format negotiation helpers. `no_std` compatible (requires `alloc`), `forbid(unsafe_code)`.

Import as `zencodec` — use `zencodec::encode`, `zencodec::decode`, etc.

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
| `heic` | HEIC/HEIF | [imazen/heic](https://github.com/imazen/heic) |
| `zentiff` | TIFF (experimental) | [imazen/zentiff](https://github.com/imazen/zentiff) |
| `zenpdf` | PDF (experimental) | [imazen/zenpdf](https://github.com/imazen/zenpdf) |

## Architecture

Every codec follows a three-layer pattern:

```text
Config     →  reusable, Clone + Send + Sync, 'static — consumed by job()
Job        →  per-operation, owns config + stop token + limits + metadata
Executor   →  borrows pixel data or file bytes, consumes self to produce output
```

```text
ENCODE:  EncoderConfig → EncodeJob → Encoder / AnimationFrameEncoder
DECODE:  DecoderConfig → DecodeJob<'a> → Decode / StreamingDecode / AnimationFrameDecoder
```

Config lives in a struct and gets shared across threads. A web server keeps one `JpegEncoderConfig` at quality 85 for all requests and clones it per-request. Calling `job()` consumes the config — clone first if you need it again. Job owns its config, cancellation token, resource limits, and metadata. Executor borrows pixels or bytes and consumes itself to produce output.

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
// (assuming pixels: PixelSlice from your pipeline)
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
| `zencodec::encode` | `EncoderConfig`, `EncodeJob`, `Encoder`, `AnimationFrameEncoder`, `EncodeOutput`, `EncodeCapabilities`, `EncodePolicy`, `best_encode_format`, dyn dispatch traits (`DynEncoderConfig`, `DynEncodeJob`, `DynEncoder`, `DynAnimationFrameEncoder`) |
| `zencodec::decode` | `DecoderConfig`, `DecodeJob`, `Decode`, `StreamingDecode`, `AnimationFrameDecoder`, `DecodeOutput`, `DecodeCapabilities`, `DecodePolicy`, `DecodeRowSink`, `SinkError`, `OutputInfo`, `SourceEncodingDetails`, `negotiate_pixel_format`, `is_format_available`, dyn dispatch traits (`DynDecoderConfig`, `DynDecodeJob`, `DynDecoder`, `DynStreamingDecoder`, `DynAnimationFrameDecoder`) |
| `zencodec::gainmap` | `GainMapInfo`, `GainMapParams`, `GainMapChannel`, `GainMapDirection`, `GainMapPresence` — cross-codec gain map types (ISO 21496-1) |
| `zencodec::helpers` | Codec implementation helpers (not consumer API) — shared boilerplate for trait implementors |
| root | `ImageFormat`, `ImageFormatDefinition`, `ImageFormatRegistry` (format detection via `ImageFormatRegistry::detect()`), `ImageInfo`, `Metadata`, `Orientation`, `OrientationHint`, `ResourceLimits`, `LimitExceeded`, `ThreadingPolicy`, `UnsupportedOperation`, `CodecErrorExt`, `find_cause`, `Unsupported`, `Extensions`, `AnimationFrame`, `OwnedAnimationFrame`, `Cicp`, `ContentLightLevel`, `MasteringDisplay`, `StopToken`, `Unstoppable` |

zencodec has no feature flags. The full API is always available.

## Limitations

- Contains no codec logic — traits, types, and format detection only.
- `ImageFormat` enum is not extensible at runtime (the `Custom` variant requires a `&'static` definition).
- Always `no_std` + `alloc` (no `std` feature gate).

## MSRV

Rust 1.93+, 2024 edition.

## Image tech I maintain

| | |
|:--|:--|
| State of the art codecs* | [zenjpeg] · [zenpng] · [zenwebp] · [zengif] · [zenavif] ([rav1d-safe] · [zenrav1e] · [zenavif-parse] · [zenavif-serialize]) · [zenjxl] ([jxl-encoder] · [zenjxl-decoder]) · [zentiff] · [zenbitmaps] · [heic] · [zenraw] · [zenpdf] · [ultrahdr] · [mozjpeg-rs] · [webpx] |
| Compression | [zenflate] · [zenzop] |
| Processing | [zenresize] · [zenfilters] · [zenquant] · [zenblend] |
| Metrics | [zensim] · [fast-ssim2] · [butteraugli] · [resamplescope-rs] · [codec-eval] · [codec-corpus] |
| Pixel types & color | [zenpixels] · [zenpixels-convert] · [linear-srgb] · [garb] |
| Pipeline | [zenpipe] · **zencodec** · [zencodecs] · [zenlayout] · [zennode] |
| ImageResizer | [ImageResizer] (C#) — 24M+ NuGet downloads across all packages |
| [Imageflow][] | Image optimization engine (Rust) — [.NET][imageflow-dotnet] · [node][imageflow-node] · [go][imageflow-go] — 9M+ NuGet downloads across all packages |
| [Imageflow Server][] | [The fast, safe image server](https://www.imazen.io/) (Rust+C#) — 552K+ NuGet downloads, deployed by Fortune 500s and major brands |

<sub>* as of 2026</sub>

### General Rust awesomeness

[archmage] · [magetypes] · [enough] · [whereat] · [zenbench] · [cargo-copter]

[And other projects](https://www.imazen.io/open-source) · [GitHub @imazen](https://github.com/imazen) · [GitHub @lilith](https://github.com/lilith) · [lib.rs/~lilith](https://lib.rs/~lilith) · [NuGet](https://www.nuget.org/profiles/imazen) (over 30 million downloads / 87 packages)

## License

Apache-2.0 OR MIT

[zenjpeg]: https://github.com/imazen/zenjpeg
[zenpng]: https://github.com/imazen/zenpng
[zenwebp]: https://github.com/imazen/zenwebp
[zengif]: https://github.com/imazen/zengif
[zenavif]: https://github.com/imazen/zenavif
[zenjxl]: https://github.com/imazen/zenjxl
[zentiff]: https://github.com/imazen/zentiff
[zenbitmaps]: https://github.com/imazen/zenbitmaps
[heic]: https://github.com/imazen/heic-decoder-rs
[zenraw]: https://github.com/imazen/zenraw
[zenpdf]: https://github.com/imazen/zenpdf
[ultrahdr]: https://github.com/imazen/ultrahdr
[jxl-encoder]: https://github.com/imazen/jxl-encoder
[zenjxl-decoder]: https://github.com/imazen/zenjxl-decoder
[rav1d-safe]: https://github.com/imazen/rav1d-safe
[zenrav1e]: https://github.com/imazen/zenrav1e
[mozjpeg-rs]: https://github.com/imazen/mozjpeg-rs
[zenavif-parse]: https://github.com/imazen/zenavif-parse
[zenavif-serialize]: https://github.com/imazen/zenavif-serialize
[webpx]: https://github.com/imazen/webpx
[zenflate]: https://github.com/imazen/zenflate
[zenzop]: https://github.com/imazen/zenzop
[zenresize]: https://github.com/imazen/zenresize
[zenfilters]: https://github.com/imazen/zenfilters
[zenquant]: https://github.com/imazen/zenquant
[zenblend]: https://github.com/imazen/zenblend
[zensim]: https://github.com/imazen/zensim
[fast-ssim2]: https://github.com/imazen/fast-ssim2
[butteraugli]: https://github.com/imazen/butteraugli
[zenpixels]: https://github.com/imazen/zenpixels
[zenpixels-convert]: https://github.com/imazen/zenpixels
[linear-srgb]: https://github.com/imazen/linear-srgb
[garb]: https://github.com/imazen/garb
[zenpipe]: https://github.com/imazen/zenpipe
[zencodecs]: https://github.com/imazen/zencodecs
[zenlayout]: https://github.com/imazen/zenlayout
[zennode]: https://github.com/imazen/zennode
[Imageflow]: https://github.com/imazen/imageflow
[Imageflow Server]: https://github.com/imazen/imageflow-server
[imageflow-dotnet]: https://github.com/imazen/imageflow-dotnet
[imageflow-node]: https://github.com/imazen/imageflow-node
[imageflow-go]: https://github.com/imazen/imageflow-go
[ImageResizer]: https://github.com/imazen/resizer
[archmage]: https://github.com/imazen/archmage
[magetypes]: https://github.com/imazen/archmage
[enough]: https://github.com/imazen/enough
[whereat]: https://github.com/lilith/whereat
[zenbench]: https://github.com/imazen/zenbench
[cargo-copter]: https://github.com/imazen/cargo-copter
[resamplescope-rs]: https://github.com/imazen/resamplescope-rs
[codec-eval]: https://github.com/imazen/codec-eval
[codec-corpus]: https://github.com/imazen/codec-corpus
