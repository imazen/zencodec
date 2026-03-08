# Implementing a zen* Codec

This guide walks through implementing the `zencodec-types` traits for a new image format. The PNM codec in `tests/pnm/mod.rs` is a complete, minimal reference implementation — read it alongside this guide.

## The Three-Layer Pattern

Every codec implements three layers:

```text
Layer 1: Config     (Clone + Send + Sync, 'static, reusable across threads)
Layer 2: Job        (borrows per-operation temporaries, short-lived)
Layer 3: Executor   (borrows pixel data or file bytes, consumes self)
```

**Config** is a settings struct. A web server keeps one at quality 85 and shares it across request threads. It must be `Clone + Send + Sync`.

**Job** borrows stack-local data that only lives for one encode/decode call: a cancellation token (`&dyn Stop`), `ResourceLimits`, `MetadataView`. The job is where you validate limits and parse headers.

**Executor** borrows the actual pixels or file bytes. It consumes itself to produce output — single-shot by design. This prevents use-after-encode/decode bugs at the type level.

```text
ENCODE:  EncoderConfig → EncodeJob<'a> → Encoder (or FullFrameEncoder)
DECODE:  DecoderConfig → DecodeJob<'a> → Decode  (or StreamingDecode, FullFrameDecoder)
```

## Define Your Error Type

Your error type must implement `From<UnsupportedOperation>`. This lets default trait method implementations return proper errors for paths your codec doesn't support.

```rust
use zc::UnsupportedOperation;

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

If your codec supports cancellation via `enough::Stop`, add a `From<StopReason>` impl too. If it checks `ResourceLimits`, add `From<zc::LimitExceeded>`.

## Implement Encoding

### Step 1: EncoderConfig

```rust
use zc::encode::{EncodeCapabilities, EncodeJob, EncoderConfig};
use zc::ImageFormat;
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
    type Job<'a> = MyEncodeJob<'a>;

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

    fn job(&self) -> MyEncodeJob<'_> {
        MyEncodeJob {
            config: self,
            limits: zc::ResourceLimits::none(),
            stop: None,
            metadata: None,
        }
    }
}
```

The `with_*` / getter pairs follow a pattern: if your codec doesn't support a knob (e.g., effort), don't override the defaults. The getter returns `None`, telling callers the codec ignored the setting.

### Step 2: EncodeJob

```rust
use zc::encode::EncodeJob;
use zc::{MetadataView, ResourceLimits};
use enough::Stop;

pub struct MyEncodeJob<'a> {
    config: &'a MyEncoderConfig,
    limits: ResourceLimits,
    stop: Option<&'a dyn Stop>,
    metadata: Option<&'a MetadataView<'a>>,
}

impl<'a> EncodeJob<'a> for MyEncodeJob<'a> {
    type Error = MyError;
    type Enc = MyEncoder<'a>;
    type FullFrameEnc = (); // Set to () if no animation support

    fn with_stop(mut self, stop: &'a dyn Stop) -> Self {
        self.stop = Some(stop);
        self
    }

    fn with_limits(mut self, limits: ResourceLimits) -> Self {
        self.limits = limits;
        self
    }

    fn with_metadata(mut self, meta: &'a MetadataView<'a>) -> Self {
        self.metadata = Some(meta);
        self
    }

    fn encoder(self) -> Result<MyEncoder<'a>, MyError> {
        Ok(MyEncoder { job: self })
    }

    fn full_frame_encoder(self) -> Result<(), MyError> {
        // Return UnsupportedOperation for features you don't support.
        Err(UnsupportedOperation::AnimationEncode.into())
    }
}
```

**Important:** `type FullFrameEnc = ()` is the standard rejection stub. `FullFrameEncoder` is implemented for `()` — all methods return `UnsupportedOperation`. This means the dyn dispatch blanket impls work correctly even for codecs without animation.

### Step 3: Encoder

The `Encoder` trait has three mutually exclusive encode paths. Implement the ones your codec supports; the rest have default implementations that return `UnsupportedOperation`.

```rust
use zc::encode::{EncodeOutput, Encoder};
use zenpixels::PixelSlice;

pub struct MyEncoder<'a> {
    job: MyEncodeJob<'a>,
}

impl Encoder for MyEncoder<'_> {
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

Set the corresponding capability flags (`with_row_level(true)`, `with_pull(true)`) if you implement paths 2 or 3.

## Implement Decoding

### Step 1: DecoderConfig

```rust
use zc::decode::{DecodeCapabilities, DecodeJob, DecoderConfig};

static MY_DECODE_CAPS: DecodeCapabilities = DecodeCapabilities::new()
    .with_cheap_probe(true)  // probe() only reads the header
    .with_icc(true)
    .with_exif(true);

impl DecoderConfig for MyDecoderConfig {
    type Error = MyError;
    type Job<'a> = MyDecodeJob<'a>;

    fn format() -> ImageFormat { ImageFormat::Jpeg }

    fn supported_descriptors() -> &'static [PixelDescriptor] {
        // Every pixel format your decoder can produce
        &[PixelDescriptor::RGB8_SRGB, PixelDescriptor::GRAY8_SRGB]
    }

    fn capabilities() -> &'static DecodeCapabilities { &MY_DECODE_CAPS }

    fn job(&self) -> MyDecodeJob<'_> {
        MyDecodeJob { config: self, limits: ResourceLimits::none(), stop: None }
    }
}
```

### Step 2: DecodeJob

The decode job is where probing, limit checking, and executor creation happen. Data is bound here, not on the executor.

```rust
use std::borrow::Cow;
use zc::decode::{DecodeJob, OutputInfo};

impl<'a> DecodeJob<'a> for MyDecodeJob<'a> {
    type Error = MyError;
    type Dec = MyDecoder<'a>;
    type StreamDec = ();       // () stub if no streaming support
    type FullFrameDec = ();    // () stub if no animation support

    fn with_stop(mut self, stop: &'a dyn Stop) -> Self {
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

    fn full_frame_decoder(self, _: Cow<'a, [u8]>, _: &[PixelDescriptor])
        -> Result<(), MyError>
    {
        Err(UnsupportedOperation::AnimationDecode.into())
    }
}
```

### Step 3: Decode

```rust
use zc::decode::{Decode, DecodeOutput};

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

Codecs that can detect how an image was encoded should attach a
`SourceEncodingDetails` implementation to the `DecodeOutput`. This lets
callers query source quality (for re-encode-at-matching-quality workflows)
and access codec-specific probe data.

### Define your probe struct

```rust
use zc::decode::SourceEncodingDetails;

/// JPEG-specific source encoding properties.
#[derive(Debug, Clone)]
pub struct JpegProbe {
    pub encoder: Option<JpegEncoder>,   // detected encoder software
    pub subsampling: ChromaSubsampling,  // 4:2:0, 4:4:4, etc.
    pub quality_estimate: Option<f32>,   // 0-100, from quantization tables
    pub is_progressive: bool,
    // ... other codec-specific fields
}

impl SourceEncodingDetails for JpegProbe {
    fn source_generic_quality(&self) -> Option<f32> {
        self.quality_estimate
    }
}
```

### Attach during decode, not probe

`probe()` returns `ImageInfo` only — it reads headers for dimensions, metadata,
and format. Source encoding analysis (quality estimation, encoder detection) is
heavier work and belongs in the decode path.

```rust
impl<'a> Decode for MyDecoder<'a> {
    type Error = MyError;

    fn decode(self) -> Result<DecodeOutput, MyError> {
        let pixels = do_decode(self.data, &self.header)?;
        let info = ImageInfo::new(
            self.header.width,
            self.header.height,
            ImageFormat::Jpeg,
        );

        // Build probe details from what we learned during decode
        let probe = JpegProbe {
            encoder: detect_encoder(&self.header),
            subsampling: self.header.subsampling,
            quality_estimate: estimate_quality(&self.header.quant_tables),
            is_progressive: self.header.is_progressive,
        };

        Ok(DecodeOutput::new(pixels, info)
            .with_source_encoding_details(probe))
    }
}
```

Don't attach during `probe()` — there's no `DecodeOutput` to attach to. The
`probe()` method returns `ImageInfo`, which describes the image. Source encoding
details describe the *encoder*, and are only available after parsing enough of
the bitstream to estimate quality tables, detect encoder fingerprints, etc.

If your codec can cheaply detect encoding details from headers alone, do the
analysis in `decode()` and attach it there. There's no cost to the probe path.

## Format Negotiation (Decode Side)

The `preferred` parameter in `decoder()` is a ranked list of pixel formats the caller wants. Your decoder should pick the first format it can produce without lossy conversion:

```rust
use zc::decode::negotiate_pixel_format;

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

Encoders receive metadata via `MetadataView` on the job. Check policy flags before embedding:

```rust
fn encode(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, MyError> {
    if let Some(meta) = self.job.metadata {
        if self.job.policy.resolve_icc(true) {
            if let Some(icc) = meta.icc_profile {
                embed_icc(icc);
            }
        }
    }
    // ...
}
```

## Dyn Dispatch: Free

You don't implement `DynEncoderConfig`, `DynEncodeJob`, etc. Blanket implementations generate the object-safe wrappers automatically from your generic trait impls. Once you implement `EncoderConfig`, your codec works with `&dyn DynEncoderConfig` — no extra code.

The only requirement: your `EncodeJob::Enc` type must implement `Encoder` and your `EncodeJob::FullFrameEnc` must implement `FullFrameEncoder` (which `()` satisfies). Same pattern on the decode side.

## Animation Support

If your codec supports animation, implement `FullFrameEncoder` (encode) and `FullFrameDecoder` (decode) on real types instead of `()`.

**Encode side:** Set `type FullFrameEnc = MyFrameEncoder` on your `EncodeJob`. The caller uses `EncodeJob::full_frame_encoder()` to get it. Your `FullFrameEncoder` accepts frames via `push_frame(pixels, duration_ms, stop)`, then `finish(stop)` produces the final `EncodeOutput`.

**Decode side:** Set `type FullFrameDec = MyFrameDecoder` on your `DecodeJob`. It composites internally and yields full-canvas frames — the caller calls `render_next_frame(stop)` repeatedly until it returns `Ok(None)`. Each `FullFrame` carries composited pixel data, duration, and frame index.

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

Set `with_row_level(true)` on your decode capabilities.

## Checklist

Before calling your implementation complete:

- [ ] `EncoderConfig` and `DecoderConfig` are `Clone + Send + Sync`
- [ ] Error type implements `From<UnsupportedOperation>`
- [ ] Capabilities accurately reflect what you support
- [ ] `supported_descriptors()` lists every format you handle without lossy conversion
- [ ] Unsupported paths return the correct `UnsupportedOperation` variant
- [ ] `probe()` only reads headers (mark `with_cheap_probe(true)` if so)
- [ ] `ResourceLimits` are checked before allocation
- [ ] Dyn dispatch works (test with `&dyn DynEncoderConfig` / `&dyn DynDecoderConfig`)
- [ ] `no_std` compatible (no `std` imports, `alloc` only)

## Reference Implementation

The PNM codec in `tests/pnm/mod.rs` implements the full pipeline in ~450 lines. It's the simplest possible codec — no compression, no metadata, no animation — but exercises every layer of the trait hierarchy including dyn dispatch.
