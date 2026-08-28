# Implementing a zen* Codec

This guide walks through implementing the `zencodec` traits for a new image format. The PNM codec in `tests/pnm/mod.rs` is a complete, minimal reference implementation — read it alongside this guide.

## The Three-Layer Pattern

Every codec implements three layers:

```text
Layer 1: Config     (Clone + Send + Sync, 'static, reusable across threads)
Layer 2: Job        (borrows per-operation temporaries, short-lived)
Layer 3: Executor   (borrows pixel data or file bytes, consumes self)
```

**Config** is a settings struct. A web server keeps one at quality 85 and shares it across request threads. It must be `Clone + Send + Sync`.

**Job** borrows stack-local data that only lives for one encode/decode call: a cancellation token (`&dyn Stop`), `ResourceLimits`, `Metadata`. The job is where you validate limits and parse headers.

**Executor** borrows the actual pixels or file bytes. It consumes itself to produce output — single-shot by design. This prevents use-after-encode/decode bugs at the type level.

```text
ENCODE:  EncoderConfig → EncodeJob → Encoder (or AnimationFrameEncoder)
DECODE:  DecoderConfig → DecodeJob<'a> → Decode  (or StreamingDecode, AnimationFrameDecoder)
```

## Define Your Error Type

Your error type must implement `From<UnsupportedOperation>`. This lets default trait method implementations return proper errors for paths your codec doesn't support.

```rust
use zencodec::UnsupportedOperation;

#[derive(Debug)]
pub enum MyError {
    Unsupported(UnsupportedOperation),
    InvalidData(String),
    // ...codec-specific variants
}

impl From<UnsupportedOperation> for MyError {
    fn from(op: UnsupportedOperation) -> Self {
        Self::Unsupported(op)
    }
}

impl core::fmt::Display for MyError { /* ... */ }
impl core::error::Error for MyError {}
```

If your codec supports cancellation via `enough::Stop`, add a `From<StopReason>` impl too. If it checks `ResourceLimits`, add `From<zencodec::LimitExceeded>`.

## Implement Encoding

### Step 1: EncoderConfig

```rust
use zencodec::encode::{EncodeCapabilities, EncodeJob, EncoderConfig};
use zencodec::ImageFormat;
use zenpixels::PixelDescriptor;

#[derive(Clone, Debug)]
pub struct MyEncoderConfig {
    quality: Option<f32>,
}

// Declare capabilities as a static. These are compile-time constants.
static MY_ENCODE_CAPS: EncodeCapabilities = EncodeCapabilities::new()
    .with_lossy(true)
    .with_lossless(true)
    .with_icc(true)
    .with_exif(true)
    .with_quality_range(0.0, 100.0)
    .with_effort_range(1, 9);

impl EncoderConfig for MyEncoderConfig {
    type Error = MyError;
    type Job = MyEncodeJob;

    fn format() -> ImageFormat {
        ImageFormat::Jpeg // or whichever format
    }

    fn supported_descriptors() -> &'static [PixelDescriptor] {
        // List every pixel format your encoder can accept without
        // lossy conversion. Order doesn't matter.
        &[
            PixelDescriptor::RGB8_SRGB,
            PixelDescriptor::GRAY8_SRGB,
        ]
    }

    fn capabilities() -> &'static EncodeCapabilities {
        &MY_ENCODE_CAPS
    }

    // Override quality handling (default methods are no-ops):
    fn with_generic_quality(mut self, quality: f32) -> Self {
        self.quality = Some(quality);
        self
    }

    fn generic_quality(&self) -> Option<f32> {
        self.quality
    }

    fn job(self) -> MyEncodeJob {
        MyEncodeJob {
            config: self,
            limits: zencodec::ResourceLimits::none(),
            stop: None,
            metadata: None,
        }
    }
}
```

The `with_*` / getter pairs follow a pattern: if your codec doesn't support a knob (e.g., effort), don't override the defaults. The getter returns `None`, telling callers the codec ignored the setting.

#### Fidelity: honour what you can, report what you did

`with_fidelity(Fidelity)` is the cross-codec entry point for "how lossy"
(`Lossless`, or `Lossy(LossyTarget)` — a codec-scale quality, or an
SSIMULACRA2 / butteraugli / zensim target). It is infallible and best-effort;
the contract lives entirely in `resolved_target_fidelity()`, which must tell
the truth. The defaults bridge to `with_lossless` / `with_generic_quality`
and derive the report from `is_lossless()` / `generic_quality()`, so a codec
that only implements the legacy knobs already behaves. Override both when you
honour a target natively (JPEG's `ApproxSsim2` picker, JXL distance) or when
the bridge would misreport. The rules `check_fidelity_honesty` enforces:

| you declare | `Lossless` request | any `Lossy(_)` request |
|---|---|---|
| `lossless` | resolves `Some(Lossless)`; decoded pixels byte-exact | — |
| no `lossless` | must not resolve `Some(Lossless)` | — |
| `lossy` | — | never resolves `Lossless`; with a `quality_range`, resolves `Some(Lossy(_))` (the target you mapped it to), not `None` |
| no `lossy` | — | **promote and say so**: resolve `Some(Lossless)` (or `None` if you have no fidelity control at all) — never `Some(Lossy(_))` |

Whatever you report as `Lossless` must decode exactly, and when both are
`Some`, `is_lossless()` must agree with `resolved_target_fidelity()`. A
lossless-only codec (the testkit `reference`) returns `Some(true)` from
`is_lossless()` unconditionally, so a `Lossy` request promotes through the
default bridge. A butteraugli *distance* has no honest codec-agnostic 0–100
mapping — if you cannot honour it, report the target you actually used.

### Step 2: EncodeJob

```rust
use zencodec::encode::EncodeJob;
use zencodec::{Metadata, ResourceLimits, StopToken};

pub struct MyEncodeJob {
    config: MyEncoderConfig,
    limits: ResourceLimits,
    stop: Option<StopToken>,
    metadata: Option<Metadata>,
}

impl EncodeJob for MyEncodeJob {
    type Error = MyError;
    type Enc = MyEncoder;
    type AnimationFrameEnc = (); // Set to () if no animation support

    fn with_stop(mut self, stop: StopToken) -> Self {
        self.stop = Some(stop);
        self
    }

    fn with_limits(mut self, limits: ResourceLimits) -> Self {
        self.limits = limits;
        self
    }

    // `with_metadata` is the storage primitive every codec implements. It is
    // marked `#[deprecated]` *for callers* (to push them toward an explicit
    // retention policy) — implementing it here does NOT warn. The framework
    // provides `with_metadata_policy(meta, policy)`, which filters via
    // `Metadata::filtered` and then calls this; you store whatever it hands you.
    fn with_metadata(mut self, meta: Metadata) -> Self {
        self.metadata = Some(meta);
        self
    }

    fn encoder(self) -> Result<MyEncoder, MyError> {
        Ok(MyEncoder { job: self })
    }

    fn animation_frame_encoder(self) -> Result<(), MyError> {
        // Return UnsupportedOperation for features you don't support.
        Err(UnsupportedOperation::AnimationEncode.into())
    }
}
```

**Important:** `type AnimationFrameEnc = ()` is the standard rejection stub. `AnimationFrameEncoder` is implemented for `()` — all methods return `UnsupportedOperation`. This means the dyn dispatch blanket impls work correctly even for codecs without animation.

### Step 3: Encoder

The `Encoder` trait has three mutually exclusive encode paths. Implement the ones your codec supports; the rest have default implementations that return `UnsupportedOperation`.

```rust
use zencodec::encode::{EncodeOutput, Encoder};
use zenpixels::PixelSlice;

pub struct MyEncoder {
    job: MyEncodeJob,
}

impl Encoder for MyEncoder {
    type Error = MyError;

    // Most codecs only need this one:
    fn encode(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, MyError> {
        let desc = pixels.descriptor();
        let w = pixels.width();
        let h = pixels.rows();

        // Check resource limits
        self.job.limits.check_dimensions(w, h)?;

        // Check cancellation
        if let Some(stop) = self.job.stop {
            stop.check()?;
        }

        // Do the actual encoding...
        let bytes: Vec<u8> = do_encode(pixels, self.job.config.quality);

        Ok(EncodeOutput::new(bytes, ImageFormat::Jpeg))
    }

    // Optional: row-level push encoding
    // fn push_rows(&mut self, rows: PixelSlice<'_>) -> Result<(), MyError> { ... }
    // fn finish(self) -> Result<EncodeOutput, MyError> { ... }

    // Optional: pull-based encoding
    // fn encode_from(self, source: &mut dyn FnMut(u32, PixelSliceMut) -> usize)
    //     -> Result<EncodeOutput, MyError> { ... }
}
```

The three paths:

1. **`encode()`** — all pixels at once. The common case.
2. **`push_rows()` + `finish()`** — caller pushes strips of rows. For codecs that can flush compressed data incrementally.
3. **`encode_from()`** — encoder pulls rows from a callback. For codecs that need specific strip heights or ordering.

Set the corresponding capability flags (`with_push_rows(true)`, `with_encode_from(true)`) if you implement paths 2 or 3.

## Implement Decoding

### Step 1: DecoderConfig

```rust
use zencodec::decode::{DecodeCapabilities, DecodeJob, DecoderConfig};

static MY_DECODE_CAPS: DecodeCapabilities = DecodeCapabilities::new()
    .with_cheap_probe(true)  // probe() only reads the header
    .with_icc(true)
    .with_exif(true);

impl DecoderConfig for MyDecoderConfig {
    type Error = MyError;
    type Job<'a> = MyDecodeJob;

    fn format() -> ImageFormat { ImageFormat::Jpeg }

    fn supported_descriptors() -> &'static [PixelDescriptor] {
        // Every pixel format your decoder can produce
        &[PixelDescriptor::RGB8_SRGB, PixelDescriptor::GRAY8_SRGB]
    }

    fn capabilities() -> &'static DecodeCapabilities { &MY_DECODE_CAPS }

    fn job<'a>(self) -> Self::Job<'a> {
        MyDecodeJob { config: self, limits: ResourceLimits::none(), stop: None }
    }
}
```

### Step 2: DecodeJob

The decode job is where probing, limit checking, and executor creation happen. Data is bound here, not on the executor.

```rust
use std::borrow::Cow;
use zencodec::decode::{DecodeJob, OutputInfo};

impl<'a> DecodeJob<'a> for MyDecodeJob {
    type Error = MyError;
    type Dec = MyDecoder<'a>;
    type StreamDec = ();       // () stub if no streaming support
    type AnimationFrameDec = ();    // () stub if no animation support

    fn with_stop(mut self, stop: zencodec::StopToken) -> Self {
        self.stop = Some(stop);
        self
    }

    fn with_limits(mut self, limits: ResourceLimits) -> Self {
        self.limits = limits;
        self
    }

    // Probe: parse headers only, return dimensions and metadata.
    fn probe(&self, data: &[u8]) -> Result<ImageInfo, MyError> {
        let header = parse_header(data)?;
        Ok(ImageInfo::new(header.width, header.height, ImageFormat::Jpeg)
            .with_frame_count(1))
    }

    // Output prediction: what format and size will the decode produce?
    fn output_info(&self, data: &[u8]) -> Result<OutputInfo, MyError> {
        let header = parse_header(data)?;
        Ok(OutputInfo::full_decode(header.width, header.height, header.native_format()))
    }

    // Bind data and preferred formats, check limits, create executor.
    fn decoder(
        self,
        data: Cow<'a, [u8]>,
        preferred: &[PixelDescriptor],
    ) -> Result<MyDecoder<'a>, MyError> {
        let header = parse_header(data)?;

        // Enforce resource limits before allocating
        self.limits.check_dimensions(header.width, header.height)?;

        Ok(MyDecoder { data, header })
    }

    // Return errors for unsupported decode modes:
    fn streaming_decoder(self, _: Cow<'a, [u8]>, _: &[PixelDescriptor])
        -> Result<(), MyError>
    {
        Err(UnsupportedOperation::RowLevelDecode.into())
    }

    fn animation_frame_decoder(self, _: Cow<'a, [u8]>, _: &[PixelDescriptor])
        -> Result<(), MyError>
    {
        Err(UnsupportedOperation::AnimationDecode.into())
    }
}
```

### Step 3: Decode

```rust
use zencodec::decode::{Decode, DecodeOutput};

impl<'a> Decode for MyDecoder<'a> {
    type Error = MyError;

    fn decode(self) -> Result<DecodeOutput, MyError> {
        let pixels = do_decode(self.data, &self.header)?;
        let info = ImageInfo::new(
            self.header.width,
            self.header.height,
            ImageFormat::Jpeg,
        );
        Ok(DecodeOutput::new(pixels, info))
    }
}
```

## Source Encoding Details

Codecs that can detect how an image was encoded should implement
`SourceEncodingDetails` on a probe struct and attach it to both `ImageInfo`
(from probe) and `DecodeOutput` (from decode). This lets callers query
source quality and access codec-specific probe data.

### The trait is minimal — put details on your struct

The `SourceEncodingDetails` trait has only two methods: `source_generic_quality()`
and `is_lossless()`. These are the only properties meaningful across **all** image
formats.

Everything codec-specific — color type, bit depth, palette size, chroma
subsampling, encoder family, quantizer tables, compression ratio — belongs as
**fields or methods on your concrete probe struct**, not on the trait. Callers
access codec-specific data via `codec_details::<T>()` downcast:

```rust
// Caller side:
if let Some(png) = details.codec_details::<PngProbe>() {
    println!("PNG{} ({:?})", png.bits_per_pixel(), png.color_type);
    println!("Palette: {} entries", png.palette_size);
}
```

This is a general zen* design principle: **traits define the cross-codec contract;
concrete types carry codec-specific richness.** The same pattern applies to
`EncoderConfig` (concrete configs expose codec-specific knobs), `EncodeOutput`
(concrete extras via `extras::<T>()`), and `DecodeOutput` (concrete extras and
source encoding details via downcast). Never add a method to a trait just because
two or three codecs happen to share a concept — if it's not universal, it belongs
on the concrete type.

### Define your probe struct

```rust
use zencodec::SourceEncodingDetails;

/// PNG-specific source encoding properties.
#[derive(Debug, Clone)]
pub struct PngProbe {
    pub color_type: ColorType,    // Grayscale, Rgb, Indexed, Rgba, ...
    pub bit_depth: u8,            // 1, 2, 4, 8, or 16
    pub palette_size: u16,        // 0 if not indexed
    pub interlaced: bool,
    pub compression_ratio: f32,
    // ... other codec-specific fields
}

impl PngProbe {
    /// Bits per pixel (PNG24=24, PNG32=32, PNG48=48, PNG64=64).
    pub fn bits_per_pixel(&self) -> u16 {
        self.color_type.channels() as u16 * self.bit_depth as u16
    }
}

impl SourceEncodingDetails for PngProbe {
    fn source_generic_quality(&self) -> Option<f32> {
        None // PNG is lossless
    }
    fn is_lossless(&self) -> bool {
        true
    }
}
```

For lossy codecs, map to the generic 0–100 scale:

```rust
impl SourceEncodingDetails for JpegProbe {
    fn source_generic_quality(&self) -> Option<f32> {
        self.quality_estimate // already 0-100 for IJG/mozjpeg
    }
    // is_lossless() defaults to false
}
```

### Attach to ImageInfo and DecodeOutput

**During probe** — if detection is cheap (header-only), attach to `ImageInfo`:

```rust
fn probe(&self, data: &[u8]) -> Result<ImageInfo, MyError> {
    let header = parse_header(data)?;
    let probe = PngProbe { /* ... */ };
    Ok(ImageInfo::new(header.width, header.height, ImageFormat::Png)
        .with_source_encoding_details(probe))
}
```

**During decode** — attach to `DecodeOutput` too (may have richer data):

```rust
fn decode(self) -> Result<DecodeOutput, MyError> {
    let pixels = do_decode(&self.data)?;
    let probe = PngProbe { /* ... */ };
    Ok(DecodeOutput::new(pixels, info)
        .with_source_encoding_details(probe))
}
```

If detection is cheap (header-only), populate it in both `probe()` and
`decode()`. If it requires deeper parsing, only populate it in `decode()`.
Callers who only probe will get whatever the codec can provide from headers.

## Source Colour Fields (`ImageInfo.source_color`)

The 2026 cross-codec audit (zencodec #11) found most decoders either left
`SourceColor` fields at their defaults or named the wrong `ColorAuthority`.
Two rules, both checked by the testkit:

### Name the authority the format spec assigns

`SourceColor::color_authority` decides which field
`SourceColor::to_color_context()` keeps and which it drops, so a wrong value
silently changes the colour of every downstream pixel (a HEIC that read only an
`nclx` box but left the `Icc` default lost its CICP and fell back to sRGB).
The rule per format is the testkit's `expected_color_authority` table:

| format | authority |
|---|---|
| PNG, JXL, HEIC, Radiance HDR | `Cicp` when you read a CICP, else `Icc` |
| AVIF | MIAF order — `Icc` when you read an ICC, else `Cicp` when you read a CICP, else `Icc` |
| JPEG, WebP, GIF, TIFF, BMP, ICO, PNM, Farbfeld, QOI, TGA, DNG, RAW, PDF | always `Icc` (embedded ICC or the sRGB assumption) |

Set it explicitly (`.with_color_authority(...)`) whenever you set `cicp`, and
never name `Cicp` without populating `cicp`. Run
`zencodec_testkit::check_source_color_authority(format, &info.source_color)`
on your own fixtures (an `nclx`-only file, an ICC-only file, both); the
`check_all` suite covers the mixes your encoder can produce.

### Fill the descriptive fields from the bitstream, not from constants

`bit_depth` and `channel_count` come from the header (PNG IHDR, JPEG SOF,
AV1 sequence header, TIFF `BitsPerSample`), even where the format only has one
answer today — a hardcoded `8` is right for WebP by accident and wrong the day
the container grows. `content_light_level` / `mastering_display` are populated
wherever the container carries them (PNG `cLLi`/`mDCv`, AVIF/HEIC `clli`/`mdcv`,
UltraHDR JPEG). Leave a field `None` when the format genuinely has no source
for it.

### `is_progressive` is a refinement order, not a decoder ability

Set `ImageInfo::is_progressive` only when the codestream is ordered so a
decoder can show a coarse version of the **whole** image before the last byte:
progressive JPEG (SOF2/6/10/14), Adam7 PNG, interlaced GIF, a JPEG XL frame
with more than one pass, a layered (`a1lx`) AVIF/HEIC item. Strips, tiles,
groups and row batches are spatial partitions — WebP, TIFF and the raster
formats are never progressive, however well the decoder streams them. The
full per-format table is on the field's rustdoc.

## Format Negotiation (Decode Side)

The `preferred` parameter in `decoder()` is a ranked list of pixel formats the caller wants. Your decoder should pick the first format it can produce without lossy conversion:

```rust
use zencodec::decode::negotiate_pixel_format;

fn decoder(self, data: Cow<'a, [u8]>, preferred: &[PixelDescriptor])
    -> Result<MyDecoder<'a>, MyError>
{
    let header = parse_header(data)?;
    let available = available_formats_for(&header);

    // negotiate_pixel_format picks the best match.
    // If preferred is empty, returns the first available (native format).
    let output_format = negotiate_pixel_format(preferred, &available);

    Ok(MyDecoder { data, header, output_format })
}
```

Pass `&[]` for native format (no preference). The decoder must never do lossy conversion to satisfy a preference — if none match, return the native format.

## Metadata Passthrough

Decoders embed metadata in `ImageInfo`:

```rust
let info = ImageInfo::new(w, h, ImageFormat::Jpeg)
    .with_icc_profile(icc_bytes.to_vec())
    .with_cicp(Cicp::SRGB)
    .with_orientation(Orientation::Rotate90);
```

Encoders receive metadata via `Metadata` on the job. Check policy flags before embedding:

```rust
fn encode(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, MyError> {
    if let Some(meta) = self.job.metadata {
        if self.job.policy.resolve_icc(true) {
            if let Some(icc) = &meta.icc_profile {
                embed_icc(icc);
            }
        }
    }
    // ...
}
```

## Colour on the Decoded Buffer (`ColorContext`)

Decoders SHOULD attach a `zenpixels::ColorContext` to every buffer they emit, so
the pixels are self-describing for any stage that sees only the buffer (a CMS, the
zenpixels-convert load-bearing reduction, a re-encoder). Adopted first by
zenavif; this section is the cross-codec convention (zencodec#25).

**Why the descriptor is not enough.** The pixel descriptor carries *enum-folded*
transfer/primaries — raw H.273 code points the enums don't model (BT.601 = 6,
SMPTE 240M = 7, …) are lost. `ColorContext.cicp` keeps the raw codes, and
`ColorContext.icc` carries the profile bytes. `ImageInfo.source_color` has both,
but it travels *beside* the buffer, so anything handed just the pixels has to
guess.

**Who carries which axis:**

| Carrier | Holds | Describes |
|---|---|---|
| `PixelDescriptor` (on the buffer, `Copy`) | folded transfer/primaries enums, range, depth | the current pixels, cheaply |
| `ColorContext` (on the buffer, `Arc`) | raw CICP + a class-valid ICC — the **authoritative** field only | the current pixels, for CMS |
| `ImageInfo.source_color` | raw CICP **and** ICC, authority flag, HDR envelope | the *source*, pre-negotiation, for provenance |

Consumers doing colour management read the buffer's context; consumers describing
provenance (or a re-encoder that needs the non-authoritative field the drop-dupe
context dropped) read `source_color`, which keeps both.

**Attach point.** After the output descriptor is final, before format
negotiation, so the reductions see it:

```rust
use zenpixels::ColorModel;

let mut ctx = info.source_color.to_color_context(); // authoritative field only
// Class gate: an ICC rides only a layout its device class describes.
if let Some(icc) = ctx
    .icc
    .take_if(|icc| zenpixels::icc::profile_color_space(icc) != Some(desc.color_model()))
{
    // Derive the profile's CICP (embedded cICP tag, then well-known identification)
    // and carry that alone; fall back to the signaled CICP.
    ctx.cicp = zenpixels::icc::extract_cicp(&icc)
        .or_else(|| zenpixels::icc::identify_common(&icc).and_then(|id| id.to_cicp()))
        .or(ctx.cicp)
        .or(info.source_color.cicp);
}
if ctx.icc.is_some() || ctx.cicp.is_some() {
    buf = buf.with_color_context(Arc::new(ctx));
}
```

(`zencodec-testkit::reference::class_gated_context` is this snippet, runnable.)

**The rules, ranked:**

1. **A class-matching profile rides as-is.** The ICC device class — header bytes
   16..20, `zenpixels::icc::profile_color_space` — must match the buffer's colour
   model: `RGB ` ↔ Rgb/Rgba/Bgr/Bgra, `GRAY` ↔ Gray/GrayAlpha, `CMYK` ↔ Cmyk.
   Crosswise pairing is invalid signaling (libpng rejects it).
2. **A derivable, class-mismatched profile → CICP-only context on the preferred
   layout.** Gray files carrying RGB-class profiles are common, and CICP is valid
   signaling for grayscale. Derive the profile's CICP (embedded `cICP` tag → 
   normalized-hash identification) and emit the gray layout with that alone —
   accurate colour, no RGB expansion, no class violation.
3. **An underivable, class-mismatched profile → keep a layout the profile
   describes.** Prefer choosing an output layout the profile describes over
   stripping the profile; a gray *preference* then resolves through the
   load-bearing reduction's ICC rules (gray-class swap when derivable, honest
   suppression otherwise). The class gate stays as defense-in-depth for codecs
   that cannot change layout.
4. **Derived/synthetic outputs carry a synthesized description, not the source's.**
   HDR reconstruction (linear f32) must not inherit an SDR profile; it *is*
   describable — source primaries, H.273 transfer 8 (linear), identity matrix,
   full range — so attach that CICP-only context. Inherit nothing that no longer
   describes the pixels; synthesize what does.
5. **Never attach an empty context.** `None` means "nothing known"; a context with
   neither field means "described as nothing" and confuses every consumer.
6. **Re-attach at every emission.** Streaming strips, per-batch scratch buffers,
   and owned frame copies are fresh slices — they are where a context silently
   dies. Compute it once, apply it on every `next_batch` / `render_next_frame`.
   (`AnimationFrame::to_owned_frame` carries it for you.)

`ImageInfo.source_color` stays the *source* description. After a gray collapse the
two legitimately differ (profile swapped or dropped on the buffer) — that is the
point.

**Testing.** `zencodec-testkit::check_color_context_consistency` (in `check_all`)
verifies class validity and that the one-shot, streaming, and animation
(borrowed + owned) paths carry identical contexts — the cheap way to catch both
the strip-drop bug and a probe-era vs frame-era CICP divergence.
`check_color_context_attached` (opt-in) is the strict positive direction: a
decoder that read colour back must attach it. Run it once your codec adopts the
convention.

## Dyn Dispatch: Free

You don't implement `DynEncoderConfig`, `DynEncodeJob`, etc. Blanket implementations generate the object-safe wrappers automatically from your generic trait impls. Once you implement `EncoderConfig`, your codec works with `&dyn DynEncoderConfig` — no extra code.

The only requirement: your `EncodeJob::Enc` type must implement `Encoder` and your `EncodeJob::AnimationFrameEnc` must implement `AnimationFrameEncoder` (which `()` satisfies). Same pattern on the decode side.

## Animation Support

If your codec supports animation, implement `AnimationFrameEncoder` (encode) and `AnimationFrameDecoder` (decode) on real types instead of `()`.

**Encode side:** Set `type AnimationFrameEnc = MyFrameEncoder` on your `EncodeJob`. The caller uses `EncodeJob::animation_frame_encoder()` to get it. Your `AnimationFrameEncoder` accepts frames via `push_frame(pixels, duration_ms, stop)`, then `finish(stop)` produces the final `EncodeOutput`.

**Decode side:** Set `type AnimationFrameDec = MyFrameDecoder` on your `DecodeJob`. It composites internally and yields full-canvas frames — the caller calls `render_next_frame(stop)` repeatedly until it returns `Ok(None)`. Each `AnimationFrame` carries composited pixel data, duration, and frame index.

Set `with_animation(true)` on your capabilities struct.

## Streaming Decode

If your codec can decode row-by-row (useful for progressive formats or memory-constrained environments), implement `StreamingDecode`:

```rust
impl StreamingDecode for MyStreamDecoder {
    type Error = MyError;

    fn next_batch(&mut self) -> Result<Option<(u32, PixelSlice<'_>)>, MyError> {
        // Return (y_offset, strip_pixels) or None when done.
        // Strip height is codec-determined.
    }

    fn info(&self) -> &ImageInfo { &self.info }
}
```

Set `with_streaming(true)` on your decode capabilities.

## Checklist

Before calling your implementation complete:

- [ ] `EncoderConfig` and `DecoderConfig` are `Clone + Send + Sync`
- [ ] Error type implements `From<UnsupportedOperation>`
- [ ] Capabilities accurately reflect what you support
- [ ] `resolved_target_fidelity()` reports honestly (promote → say `Lossless`; never claim a mode you don't declare)
- [ ] `supported_descriptors()` lists every format you handle without lossy conversion
- [ ] Unsupported paths return the correct `UnsupportedOperation` variant
- [ ] `probe()` only reads headers (mark `with_cheap_probe(true)` if so)
- [ ] `source_color.color_authority` follows the format table above; `bit_depth` /
      `channel_count` read from the header; `is_progressive` only for a refinement order
- [ ] `ResourceLimits` are checked before allocation
- [ ] Dyn dispatch works (test with `&dyn DynEncoderConfig` / `&dyn DynDecoderConfig`)
- [ ] `no_std` compatible (no `std` imports, `alloc` only)

## Reference Implementation

The PNM codec in `tests/pnm/mod.rs` implements the full pipeline in ~450 lines. It's the simplest possible codec — no compression, no metadata, no animation — but exercises every layer of the trait hierarchy including dyn dispatch.
