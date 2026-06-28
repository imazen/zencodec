<!-- GENERATED FROM README.md by zenutils gen-readme-crates.sh — DO NOT EDIT. -->

# zencodec

zencodec is the shared trait crate that defines the common API for all zen\* image codecs.

zencodec contains no pixel encoding or decoding logic — that lives in the individual codec crates. It does include shared metadata parsing needed for nearly every image format: pixel-descriptor derivation from CICP/ICC metadata (with identification delegated to `zenpixels::icc`, covering 163 RGB + 18 grayscale web-corpus profiles), EXIF orientation extraction, ISO 21496-1 gain map parsing and serialization, and format detection via magic bytes. `no_std` compatible (requires `alloc`), `forbid(unsafe_code)`.

Import as `zencodec` — `zencodec::encode`, `zencodec::decode`, and the shared
types (metadata, limits, format detection, color emission) at the root.

## Quick start

```toml
[dependencies]
zencodec = "0.1.25"
```

zencodec defines the traits; a concrete codec crate (here `zenjpeg`) supplies the
pixels. The three-layer `Config → Job → Encoder/Decoder` shape is identical across
every zen\* codec, so swapping `zenjpeg` for `zenwebp`, `zengif`, `zenavif`, …
changes only the config type:

```rust,ignore
use std::borrow::Cow;
use zenjpeg::{JpegEncoderConfig, JpegDecoderConfig};
use zencodec::encode::{EncoderConfig, EncodeJob, Encoder};
use zencodec::decode::{DecoderConfig, DecodeJob, Decode};

// Encode. `with_generic_quality` is the codec-agnostic knob on a calibrated
// 0.0..=100.0 scale (NOT 0.0..=1.0); higher is better. Read it back with
// `generic_quality()` — `None` means the codec has no quality dial.
let config = JpegEncoderConfig::new().with_generic_quality(85.0);
// (assuming pixels: PixelSlice from your pipeline)
let output = config.job().encoder()?.encode(pixels.as_slice())?;
let jpeg_bytes = output.into_vec();

// Decode
let config = JpegDecoderConfig::new();
let decoded = config.job().decoder(Cow::Borrowed(&jpeg_bytes), &[])?.decode()?;
let pixels = decoded.into_buffer();
```

For untrusted input, attach a resource limit and a cancellation token to the
**job** — see [Untrusted input](#untrusted-input-limits-cancellation-errors) below.

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

## Untrusted input: limits, cancellation, errors

Two server-critical knobs — `ResourceLimits` and a cancellation `StopToken` — are
attached to the **job**, not the config: `DecodeJob`/`EncodeJob` expose
`.with_limits(limits)` and `.with_stop(token)` (both consume-and-return-`self`
builders), so the per-request job carries them while the shared config stays
immutable. Everything below is on the root: `use zencodec::{ResourceLimits, StopToken};`.

### Constructing `ResourceLimits`

`ResourceLimits` is a plain struct with `pub` `Option<_>` fields — build it three ways:

```rust,ignore
use zencodec::ResourceLimits;

// 1. Server preset. `for_untrusted_input()` fills generous-but-bounded caps:
//    max_pixels 120 MP/frame, max_total_pixels 200 MP (all frames),
//    max_width/max_height 16384 each, max_memory_bytes 1 GiB,
//    max_input_bytes 256 MiB, max_frames 65 536, max_animation_ms 1 hour.
//    (`Default`/`none()` is the OPPOSITE — every field `None`, i.e. UNLIMITED;
//    use that only for trusted input.) Tighten any field with a `with_*` builder:
let limits = ResourceLimits::for_untrusted_input()
    .with_max_pixels(4_000_000)          // 4 MP cap for a thumbnail service
    .with_max_memory(256 * 1024 * 1024); // 256 MiB

// 2. From scratch off the unlimited default, set only what you need:
let limits = ResourceLimits::none()
    .with_max_pixels(16_000_000)         // pixels = width × height (per frame)
    .with_max_input_bytes(8 * 1024 * 1024); // bytes of encoded input (decode)

// 3. Direct field set (every field is public):
let mut limits = ResourceLimits::default();
limits.max_width = Some(8192);           // pixels
limits.max_height = Some(8192);          // pixels
```

Units: `max_pixels` / `max_total_pixels` are pixel counts (`width × height`,
the latter ×`frame_count`); `max_width` / `max_height` are pixels; `max_memory_bytes`,
`max_input_bytes`, `max_output_bytes` are bytes; `max_frames` a count;
`max_animation_ms` milliseconds. A `None` field means that dimension is unchecked.

`ResourceLimits` also carries an allocation-fallibility *preference*,
`prefer_fallible_allocations` (an `AllocPreference`: `CodecDefault` / `Fallible` /
`Infallible`). `CodecDefault` (the default) lets each codec choose — decoders favour the
fallible `try_reserve` path on untrusted input, encoders the faster infallible `vec!`;
`for_untrusted_input()` presets `Fallible`. Override with
`.with_prefer_fallible_allocations(AllocPreference::Fallible)`.

### Constructing a cancellation `StopToken`

`StopToken` is re-exported from the [`almost-enough`](https://crates.io/crates/almost-enough)
crate (`cargo add almost-enough`). Make a `Stopper` (cheap, `Clone`, 8 bytes), erase it
into a `StopToken`, hold a clone of the `Stopper` to fire later from any thread:

```rust,ignore
use zencodec::StopToken;

let stopper = almost_enough::Stopper::new();
let token = StopToken::new(stopper.clone()); // or: stopper.clone().into()

// Fire from a deadline / client-disconnect watcher — `cancel()` signals every clone:
std::thread::spawn({
    let stopper = stopper.clone();
    move || { /* on timeout or disconnect: */ stopper.cancel(); }
});
// (For a no-op token when you don't need cancellation, use `zencodec::Unstoppable`.)
```

### End-to-end: decode untrusted bytes with a limit + a stop token

`.with_limits()` and `.with_stop()` chain onto the job before you ask for the
decoder. `probe()` (also on the job) is an O(header) parse — validate against the
limits **before** the codec allocates pixels, then attach both to the real decode:

```rust,ignore
use std::borrow::Cow;
use zencodec::{ResourceLimits, StopToken, CodecErrorExt};
use zencodec::decode::{DecoderConfig, DecodeJob, Decode};

let limits = ResourceLimits::for_untrusted_input().with_max_memory(256 * 1024 * 1024);
let stopper = almost_enough::Stopper::new();
let token = StopToken::new(stopper.clone());

// 1. Probe the header and reject oversized inputs before any pixel allocation.
let info = config.clone().job().probe(bytes)?;
limits.check_image_info(&info)?; // -> Err(LimitExceeded) if too big

// 2. Attach BOTH to the job, then decode. Order is free; `decoder()` consumes the job.
let decoded = config.job()
    .with_limits(limits)
    .with_stop(token)
    .decoder(Cow::Borrowed(bytes), &[])? // &[] = native pixel format
    .decode()?;
let pixels = decoded.into_buffer();
```

`.with_stop()` is honored only by codecs whose `DecodeCapabilities::stop()` is
true; on others it is a silent no-op (the decode still completes correctly, just
not interruptibly). `check_image_info` only sees what the header reports — keep
the `max_memory_bytes` cap so a codec that under-reports still can't over-allocate.

### Classifying a codec's error for an HTTP response

Each codec keeps its OWN opaque error type (there is no shared `CodecError`).
Classify one without naming the concrete enum, via `CodecErrorExt`:

```rust,ignore
match config.job().decoder(Cow::Borrowed(bytes), &[]) {
    Ok(_decoder) => { /* _decoder.decode()? */ }
    Err(e) => {
        if let Some(limit) = e.limit_exceeded() {
            eprintln!("resource limit: {limit}"); // -> HTTP 413
        } else if e.unsupported_operation().is_some() {
            eprintln!("unsupported"); // -> HTTP 415
        } else {
            eprintln!("malformed input: {e}"); // -> HTTP 400
        }
    }
}
```

## Controlling decode parallelism

Zen codecs parallelize with [rayon](https://docs.rs/rayon)'s **ambient** pool. The
responsibility splits cleanly: the *codec* chooses sequential vs parallel
(`ResourceLimits::threading` → `ThreadingPolicy::{Sequential, Parallel}`); the
*caller* chooses the thread **count** by sizing a pool and running the decode
inside `rayon::ThreadPool::install` — you don't ask the codec to cap threads from
the inside.

`DynDecoder` (one-shot) is intentionally **not `Send`** — it may borrow your input
zero-copy. That is *not* an obstacle to capping threads: `decode()` returns an
owned, `Send` `DecodeOutput`, so construct **and** consume the decoder *inside*
the closure and only the result crosses back out. The codec's internal rayon work
then runs on your sized pool:

```rust
use std::borrow::Cow;
use rayon::ThreadPoolBuilder;
use zencodec::decode::{DecodeOutput, DynDecoderConfig};

let pool = ThreadPoolBuilder::new().num_threads(2).build()?;
let out: DecodeOutput = pool.install(|| {
    config.dyn_job()
        .into_decoder(Cow::Borrowed(&bytes), &[])?   // Box<dyn DynDecoder> — not Send, but local
        .decode()                                     // -> DecodeOutput (Send), consumes the decoder
})?;
// the non-Send decoder lived and died on a pool worker; only owned pixels came back
```

Run **many** decodes under one capped pool the same way:
`pool.install(|| inputs.par_iter().map(decode_one).collect())`, each decoder built
inside its own task. Need a **live decoder on another thread** (not just
thread-capping)? Use the **streaming** path: `DynStreamingDecoder` *is* `Send` by
contract (it owns/copies its data), so it can move across a thread boundary —
one-shot trades that for zero-copy borrowing.

**Exception — native-threaded codecs.** AV1 (`rav1d` / `zenrav1e` / AVIF) spawns
OS threads, not rayon, so `install()` has no effect on them; cap those with
codec-specific config (e.g. `AvifEncoderConfig::with_threads(4)`). See
[`ThreadingPolicy`](https://docs.rs/zencodec/latest/zencodec/enum.ThreadingPolicy.html)
for the full model.

> These patterns are exercised end-to-end in `zencodec-testkit/tests/decode_parallelism.rs`.

## Encode fidelity

`EncoderConfig::with_fidelity` is the codec-agnostic way to ask for a quality
level — infallible and best-effort: the codec does what it can and substitutes
the rest.

```rust
use zencodec::encode::{EncoderConfig, Fidelity};

let cfg = my_encoder_config
    .with_fidelity(Fidelity::Lossless);              // mathematically exact
//  .with_fidelity(Fidelity::ssim2(90.0))            // aim at SSIMULACRA2 ~= 90
//  .with_fidelity(Fidelity::butteraugli(1.0))       // aim at butteraugli max-norm ~= 1.0
//  .with_fidelity(Fidelity::codec_quality(85.0));   // the codec's own native dial
```

`Fidelity` is either `Lossless` or `Lossy(LossyTarget)`, where a `LossyTarget` is:

- **`ApproxSsim2(score)`** — a one-shot SSIMULACRA2 target (a real cross-codec metric).
- **`ApproxButteraugli(distance)`** — a one-shot butteraugli max-norm distance.
- **`CodecSpecificQuality(q)`** — the codec's own native quality scale (meaning differs per codec).

These are **blind, single-pass**: the target maps to a native dial in one encode,
no re-encode loop. A codec that hasn't implemented native fidelity still behaves
sensibly — the default bridges to the legacy `with_lossless` / `with_generic_quality`
setters. Read back what the codec resolved to with
`resolved_target_fidelity() -> Option<Fidelity>`.

> A `LosslessMode` container variant (lossless coding within a loss budget — the
> screen-content path) and a fail-fast `try_with_fidelity` verdict are designed but
> deferred while their semantics settle; see the reserved blocks in
> `src/fidelity.rs` and [imazen/zencodec#104](https://github.com/imazen/zencodec/issues/104).

## Key Design Decisions

**Color management is not the codec's job.** Decoders return native pixels with ICC/CICP metadata. Encoders accept pixels as-is and embed the provided metadata. The caller handles CMS transforms.

**Format negotiation over conversion.** Decoders take a ranked `&[PixelDescriptor]` preference list and pick the first they can produce without lossy conversion. Pass `&[]` for native format.

**Capabilities over try/catch.** Codecs declare their capabilities as const `EncodeCapabilities` / `DecodeCapabilities` structs. Check before calling instead of catching `UnsupportedOperation` errors.

**Pixel types from `zenpixels`.** All pixel interchange types (`PixelSlice`, `PixelBuffer`, `PixelDescriptor`, etc.) are defined in the `zenpixels` crate. All zen\* crates depend on `zenpixels` directly.

## Metadata Retention

Re-encode and recompress pipelines need to decide what metadata survives. `Metadata::filtered` applies a `MetadataPolicy`, so callers never hand-parse EXIF:

```rust,ignore
use zencodec::{MetadataPolicy, MetadataFields, IccRetention, exif::{ExifPolicy, Retention}};

// Decode → filter → re-encode. `Web` (recommended for publishing) keeps the ICC profile
// (unless a redundant sRGB), EXIF orientation + rights, and CICP/HDR color
// signaling — and strips GPS, timestamps, camera info, thumbnail, and XMP.
let kept = decoded_meta.filtered(&MetadataPolicy::Web);

// Presets: PreserveExact (keep all, incl. duplicate sRGB), Preserve (drop dup
// sRGB), Web, ColorAndRotation (only what places pixels), Custom.
let minimal = decoded_meta.filtered(&MetadataPolicy::ColorAndRotation);

// Per-field control — drop only the thumbnail, keep everything else:
let policy = MetadataPolicy::Custom(
    MetadataFields::KEEP_ALL.with_exif(ExifPolicy::KEEP_ALL.with_thumbnail(Retention::Discard)),
);
let no_thumb = decoded_meta.filtered(&policy);
```

`MetadataFields` encapsulates EXIF in an `ExifPolicy` with seven keep/discard categories — `orientation`, `rights`, `thumbnail`, `gps`, `datetimes`, `camera`, `other` — and three-way ICC handling (`IccRetention::{Drop, KeepNonSrgb, Keep}`). EXIF passes through byte-unchanged (zero-copy) when no category is dropped, and is rewritten — offsets recomputed — only when pruning. CICP/HDR are color *signaling* (dropping them changes displayed pixels), so the presets keep them; a `Custom` policy can drop them. The structured parser/editor is public as [`zencodec::exif::Exif`](https://docs.rs/zencodec) (`parse` → `filtered`/edit → `to_bytes`) for direct EXIF work — including setting Copyright/Artist (`set_copyright` / `set_artist`, with a `TextEncoding` choice of Exif 2.x ASCII or Exif 3.0 UTF-8) and Orientation (`set_orientation`, insert-or-replace).

**Privacy is an explicit choice — enforced at compile time.** Retention is a *transient* decision made when you hand metadata to the encoder, not a field stored on `Metadata`. The blessed path is `job.with_metadata_policy(meta, MetadataPolicy::Web)` (privacy-safe: strips camera/GPS, keeps orientation + rights) or `PreserveExact` (verbatim). The old unguarded `with_metadata(meta)` still works but is `#[deprecated]` — the compiler **warns** at every call site that picks no policy, so you can't propagate metadata without choosing retention by accident. It's a compile-time nudge, not a semver break: existing code keeps compiling, but the warning points you at the safe call. The filter runs *before* the codec sees the record, so a codec only ever receives exactly what the policy kept. The carried bytes stay untouched until then, so you can still pull `metadata.exif` out, edit it with any EXIF library, and put it back via `with_exif`.

To **stamp** rights in one line — `Metadata::none().with_copyright("© 2026 You")` builds (or merges into) the EXIF blob (ASCII); or build it directly with `Exif::new(TextEncoding::Ascii).set_copyright(…)` → `to_bytes()` — `Exif::new` requires the Exif 2.x-vs-3.0 field-type choice (type 129 is read by almost nothing, so it's never a silent default).

Metadata retention, color emission, and orientation are the three *correctness* signals an encode has to get right; [docs/correctness-model.md](https://github.com/imazen/zencodec/blob/main/docs/correctness-model.md) describes how the framework resolves each one before the codec runs so a codec can't quietly clobber it. The [`zencodec-testkit`](https://github.com/imazen/zencodec/tree/main/zencodec-testkit) crate verifies a codec honors that contract — `check_metadata_no_leak` re-parses the embedded EXIF to prove a policy's drops actually happened, and `check_cross_path_pixel_equivalence` diffs every feeding mode.

## Color Emission

The encode-side dual of color resolution: which color carriers (ICC vs CICP) should an encode *write*? `resolve_color_emit` decides — a pure, `no_std`, CMS-free function of the source color, the target's carrier capabilities, and a policy:

```rust,ignore
use zencodec::{resolve_color_emit, ColorEmitPolicy, IccDisposition};

let plan = resolve_color_emit(&source_color, &target_caps, ColorEmitPolicy::Balanced);
// plan.cicp: Option<Cicp>   — write this CICP (JXL/AVIF/HEIC nclx, PNG cICP) if the format carries it
// plan.icc:  IccDisposition — KeepSource | SynthesizeFrom(Cicp) | Drop
```

`ColorEmitPolicy` picks the tradeoff: `Compatibility` (widest reader support), `Balanced` (default — CICP where it's a spec-mandated *safe sole carrier*, an ICC companion otherwise), `Compact` (smallest — prefer CICP, drop the ICC), `Verbatim` (carry the source's signals unchanged), or `Custom(ColorEmitFields)`. A target advertises its carriers via `EncodeCapabilities::{cicp_is_valid_carrier, cicp_safe_sole_carrier}`. The plan never emits a redundant `SynthesizeFrom(sRGB)`; a codec lowers a `SynthesizeFrom` through `zenpixels-convert`'s transfer-aware `synthesize_icc_for_cicp` (a bundled `const` profile, or — with its `cms-moxcms` feature — a generated one) so an HDR transfer is never mis-tagged with an SDR profile and color is never silently dropped. The names carry the emit direction so they can't be confused with the decode-side `SourceColor`. Design + rejected alternatives: [docs/color-emit-model.md](https://github.com/imazen/zencodec/blob/main/docs/color-emit-model.md).

## What's in this crate

| Module | Contents |
|--------|----------|
| `zencodec::encode` | `EncoderConfig`, `EncodeJob`, `Encoder`, `AnimationFrameEncoder`, `EncodeOutput`, `EncodeCapabilities`, `EncodePolicy`, `best_encode_format`, dyn dispatch traits (`DynEncoderConfig`, `DynEncodeJob`, `DynEncoder`, `DynAnimationFrameEncoder`) |
| `zencodec::decode` | `DecoderConfig`, `DecodeJob`, `Decode`, `StreamingDecode`, `AnimationFrameDecoder`, `DecodeOutput`, `DecodeCapabilities`, `DecodePolicy`, `DecodeRowSink`, `SinkError`, `OutputInfo`, `SourceEncodingDetails`, `negotiate_pixel_format`, `is_format_available`, dyn dispatch traits (`DynDecoderConfig`, `DynDecodeJob`, `DynDecoder`, `DynStreamingDecoder`, `DynAnimationFrameDecoder`) |
| `zencodec::estimate` | `ResourceEstimate` (predicted peak memory / wall + CPU time / core-scaling, all `Option`), `ComputeEnvironment` (cores, RAM, `SimdTier`), `ImageCharacteristics`, `SimdTier`, `ThreadingInformation` — codec-agnostic resource estimation, surfaced via `EncoderConfig::estimate_encode_resources` / `DecoderConfig::estimate_decode_resources` |
| `zencodec::gainmap` | `GainMapInfo`, `GainMapParams`, `GainMapChannel`, `GainMapDirection`, `GainMapPresence`, `Iso21496Format` (wire-format variant: `AvifTmap`, `JxlJhgm`, `JpegApp2BodyWithUrn`; the original `JpegApp2` is deprecated since 0.1.20), `ISO_21496_1_URN`, `ISO_21496_1_PRIMARY_APP2_BODY`, `serialize_iso21496_fmt` / `serialize_iso21496_fmt_into` / `parse_iso21496_fmt`, `GainMapParseError` — cross-codec gain map types and wire-format helpers (ISO 21496-1) |
| `zencodec::exif` | Structured EXIF/TIFF: `Exif` (borrowing parse → prune → serialize), `ExifPolicy` (7 keep/discard categories), `Retention`, `ByteOrder`, `retain` |
| `zencodec::helpers` | Codec implementation helpers (not consumer API) — shared boilerplate for trait implementors, plus the lightweight `parse_exif_orientation` accessor |
| root | `ImageFormat`, `ImageFormatDefinition`, `ImageFormatRegistry` (format detection via `ImageFormatRegistry::detect()`), `ImageInfo`, `Metadata`, `MetadataPolicy`, `MetadataFields`, `IccRetention`, `Exif`, `ExifPolicy`, `Retention`, `ByteOrder`, `Orientation`, `OrientationHint`, `ResourceLimits`, `AllocPreference`, `LimitExceeded`, `ThreadingPolicy`, `UnsupportedOperation`, `CodecErrorExt`, `find_cause`, `Unsupported`, `Extensions`, `AnimationFrame`, `OwnedAnimationFrame`, `resolve_color_emit`, `ColorEmitPolicy`, `ColorEmitPlan`, `ColorEmitFields`, `IccDisposition`, `CicpEmission`, `ColorAuthority`, `Cicp`, `ContentLightLevel`, `MasteringDisplay`, `StopToken`, `Unstoppable` |

zencodec has no feature flags. The full API is always available.

## Limitations

- Contains no codec logic — traits, types, and format detection only.
- `ImageFormat` enum is not extensible at runtime (the `Custom` variant requires a `&'static` definition).
- Always `no_std` + `alloc` (no `std` feature gate).

## MSRV

Rust 1.88+, 2024 edition.

## License

Licensed under either of [Apache-2.0](https://github.com/imazen/zencodec/blob/main/LICENSE-APACHE) or [MIT](https://github.com/imazen/zencodec/blob/main/LICENSE-MIT) at your option.

## Image tech I maintain

| | |
|:--|:--|
| **Codecs** ¹ | [zenjpeg] · [zenpng] · [zenwebp] · [zengif] · [zenavif] · [zenjxl] · [zenbitmaps] · [heic] · [zentiff] · [zenpdf] · [zensvg] · [zenjp2] · [zenraw] · [ultrahdr] |
| Codec internals | [zenjxl-decoder] · [jxl-encoder] · [zenrav1e] · [rav1d-safe] · [zenavif-parse] · [zenavif-serialize] |
| Compression | [zenflate] · [zenzop] · [zenzstd] |
| Processing | [zenresize] · [zenquant] · [zenblend] · [zenfilters] · [zensally] · [zentone] |
| Pixels & color | [zenpixels] · [zenpixels-convert] · [linear-srgb] · [garb] |
| Pipeline & framework | [zenpipe] · **zencodec** · [zencodecs] · [zenlayout] · [zennode] · [zenwasm] · [zentract] |
| Metrics | [zensim] · [fast-ssim2] · [butteraugli] · [zenmetrics] · [resamplescope-rs] |
| Pickers & ML | [zenanalyze] · [zenpredict] · [zenpicker] |
| Products | [Imageflow] image engine ([.NET][imageflow-dotnet] · [Node][imageflow-node] · [Go][imageflow-go]) · [Imageflow Server] · [ImageResizer] (C#) |

<sub>¹ pure-Rust, `#![forbid(unsafe_code)]` codecs, as of 2026</sub>

### General Rust awesomeness

[zenbench] · [archmage] · [magetypes] · [enough] · [whereat] · [cargo-copter]

[Open source](https://www.imazen.io/open-source) · [@imazen](https://github.com/imazen) · [@lilith](https://github.com/lilith) · [lib.rs/~lilith](https://lib.rs/~lilith)

[zenjpeg]: https://github.com/imazen/zenjpeg
[zenpng]: https://github.com/imazen/zenpng
[zenwebp]: https://github.com/imazen/zenwebp
[zengif]: https://github.com/imazen/zengif
[zenavif]: https://github.com/imazen/zenavif
[zenjxl]: https://github.com/imazen/zenjxl
[zenbitmaps]: https://github.com/imazen/zenbitmaps
[heic]: https://github.com/imazen/heic
[zentiff]: https://github.com/imazen/zentiff
[zenpdf]: https://github.com/imazen/zenpdf
[zensvg]: https://github.com/imazen/zenextras
[zenjp2]: https://github.com/imazen/zenextras
[zenraw]: https://github.com/imazen/zenraw
[ultrahdr]: https://github.com/imazen/ultrahdr
[zenjxl-decoder]: https://github.com/imazen/zenjxl-decoder
[jxl-encoder]: https://github.com/imazen/jxl-encoder
[zenrav1e]: https://github.com/imazen/zenrav1e
[rav1d-safe]: https://github.com/imazen/rav1d-safe
[zenavif-parse]: https://github.com/imazen/zenavif-parse
[zenavif-serialize]: https://github.com/imazen/zenavif-serialize
[zenflate]: https://github.com/imazen/zenflate
[zenzop]: https://github.com/imazen/zenzop
[zenzstd]: https://github.com/imazen/zenzstd
[zenresize]: https://github.com/imazen/zenresize
[zenquant]: https://github.com/imazen/zenquant
[zenblend]: https://github.com/imazen/zenblend
[zenfilters]: https://github.com/imazen/zenfilters
[zensally]: https://github.com/imazen/zensally
[zentone]: https://github.com/imazen/zentone
[zenpixels]: https://github.com/imazen/zenpixels
[zenpixels-convert]: https://github.com/imazen/zenpixels
[linear-srgb]: https://github.com/imazen/linear-srgb
[garb]: https://github.com/imazen/garb
[zenpipe]: https://github.com/imazen/zenpipe
[zencodecs]: https://github.com/imazen/zencodecs
[zenlayout]: https://github.com/imazen/zenlayout
[zennode]: https://github.com/imazen/zennode
[zenwasm]: https://github.com/imazen/zenwasm
[zentract]: https://github.com/imazen/zentract
[zensim]: https://github.com/imazen/zensim
[fast-ssim2]: https://github.com/imazen/fast-ssim2
[butteraugli]: https://github.com/imazen/butteraugli
[zenmetrics]: https://github.com/imazen/zenmetrics
[resamplescope-rs]: https://github.com/imazen/resamplescope-rs
[zenanalyze]: https://github.com/imazen/zenanalyze
[zenpredict]: https://github.com/imazen/zenanalyze
[zenpicker]: https://github.com/imazen/zenanalyze
[zenbench]: https://github.com/imazen/zenbench
[archmage]: https://github.com/imazen/archmage
[magetypes]: https://github.com/imazen/archmage
[enough]: https://github.com/imazen/enough
[whereat]: https://github.com/lilith/whereat
[cargo-copter]: https://github.com/imazen/cargo-copter
[Imageflow]: https://github.com/imazen/imageflow
[Imageflow Server]: https://github.com/imazen/imageflow-dotnet-server
[ImageResizer]: https://github.com/imazen/resizer
[imageflow-dotnet]: https://github.com/imazen/imageflow-dotnet
[imageflow-node]: https://github.com/imazen/imageflow-node
[imageflow-go]: https://github.com/imazen/imageflow-go
