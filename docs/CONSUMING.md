# Using zen* Codecs

This guide covers encoding and decoding images with any zen* codec through the `zencodec-types` API.

## Dependencies

Add the codec crate(s) you need and `zenpixels` for pixel types:

```toml
[dependencies]
zenjpeg = "0.1"
zenpng = "0.1"
zenpixels = { version = "0.1", features = ["buffer"] }
```

Each codec crate re-exports `zencodec-types` as `zc`, so you don't need to depend on it directly. Import the traits from `zc::encode` and `zc::decode`.

For codec-agnostic multi-format dispatch, use `zencodecs` instead of individual codec crates.

## Basic Encode

The flow is always **Config → Job → Encoder → output**.

```rust
use zenjpeg::JpegEncoderConfig;
use zc::encode::{EncoderConfig, EncodeJob, Encoder};
use zenpixels::{PixelBuffer, PixelDescriptor};

// 1. Create a reusable config (Clone + Send + Sync, share across threads)
let config = JpegEncoderConfig::new()
    .with_generic_quality(85.0);

// 2. Create a per-operation job
let job = config.job();

// 3. Create the encoder
let encoder = job.encoder()?;

// 4. Encode pixels
let output = encoder.encode(pixels.as_slice())?;

// 5. Get the bytes
let jpeg_bytes: Vec<u8> = output.into_vec();
```

The config is reusable — create it once, call `.job()` for each encode operation. The job and encoder are consumed on use; they can't be reused.

## Basic Decode

The flow is **Config → Job → (probe) → Decoder → output**.

```rust
use std::borrow::Cow;
use zenjpeg::JpegDecoderConfig;
use zc::decode::{DecoderConfig, DecodeJob, Decode};

let config = JpegDecoderConfig::new();
let job = config.job();

// Optional: probe to get dimensions without decoding
let info = job.probe(&jpeg_bytes)?;
println!("{}x{}", info.width, info.height);

// Decode
let decoder = job.decoder(Cow::Borrowed(&jpeg_bytes), &[])?;
let output = decoder.decode()?;

let pixels = output.pixels();       // borrow as PixelSlice
let buffer = output.into_buffer();  // or take ownership as PixelBuffer
```

The `&[]` passed to `decoder()` is the preferred pixel format list. Empty means "give me the native format" — the decoder picks whatever it produces most efficiently.

## Choosing Output Pixel Formats

Pass a ranked preference list to `decoder()`. The decoder picks the first format it can produce without lossy conversion:

```rust
use zenpixels::PixelDescriptor;

// "I'd prefer RGBA8, but RGB8 is fine too"
let preferred = &[PixelDescriptor::RGBA8_SRGB, PixelDescriptor::RGB8_SRGB];
let decoder = job.decoder(Cow::Borrowed(&data), preferred)?;
let output = decoder.decode()?;

// Check what you actually got
let actual_format = output.descriptor();
```

If none of your preferences match, the decoder returns its native format. It will never do lossy conversion to satisfy a preference.

You can also check format support before decoding:

```rust
use zc::decode::{is_format_available, negotiate_pixel_format};

// Does this decoder support RGBA8 output at all?
let supported = JpegDecoderConfig::supported_descriptors();
if is_format_available(PixelFormat::Rgba8, supported) {
    // ...
}
```

On the encode side, check whether the encoder accepts your pixel format:

```rust
use zc::encode::best_encode_format;

let supported = JpegEncoderConfig::supported_descriptors();
if let Some(fmt) = best_encode_format(my_pixels.descriptor(), supported) {
    // Encoder accepts this format (possibly after non-lossy conversion)
}
```

## Codec-Agnostic Code (Dyn Dispatch)

For code that works with any codec — plugin systems, multi-format pipelines, format-selection-at-runtime — use the `Dyn*` traits:

```rust
use zc::encode::{DynEncoderConfig, BoxedError};
use zc::decode::DynDecoderConfig;
use zenpixels::PixelSlice;

fn encode_any(
    config: &dyn DynEncoderConfig,
    pixels: PixelSlice<'_>,
) -> Result<Vec<u8>, BoxedError> {
    let job = config.dyn_job();
    let encoder = job.into_encoder()?;
    Ok(encoder.encode(pixels)?.into_vec())
}

fn decode_any(
    config: &dyn DynDecoderConfig,
    data: &[u8],
) -> Result<zenpixels::PixelBuffer, BoxedError> {
    let job = config.dyn_job();
    let decoder = job.into_decoder(Cow::Borrowed(data), &[])?;
    Ok(decoder.decode()?.into_buffer())
}

// Works with any codec:
let jpeg_config = JpegEncoderConfig::new();
let png_config = PngEncoderConfig::new();
encode_any(&jpeg_config, pixels)?;
encode_any(&png_config, pixels)?;
```

Every `EncoderConfig` automatically implements `DynEncoderConfig` via blanket impls. Same for decoders. You don't need to do anything special — just cast with `&config as &dyn DynEncoderConfig` or pass by reference.

The dyn API uses `BoxedError` instead of codec-specific error types. Downcast if you need the concrete error:

```rust
match result {
    Err(e) => {
        if let Some(jpeg_err) = e.downcast_ref::<zenjpeg::JpegError>() {
            // handle codec-specific error
        }
    }
    Ok(output) => { /* ... */ }
}
```

## Checking Capabilities

Before calling optional methods, check whether the codec supports them:

```rust
use zc::encode::EncoderConfig;

let caps = JpegEncoderConfig::capabilities();

// Metadata support
if caps.icc()  { /* encoder can embed ICC profiles */ }
if caps.exif() { /* encoder can embed EXIF data */ }

// Compression modes
if caps.lossy()    { /* supports lossy encoding */ }
if caps.lossless() { /* supports lossless encoding */ }

// Quality/effort ranges
if let Some([min, max]) = caps.quality_range() {
    println!("quality range: {min}..{max}");
}

// Advanced encode paths
if caps.row_level() { /* push_rows() + finish() is supported */ }
if caps.pull()      { /* encode_from() callback is supported */ }
if caps.animation() { /* full_frame_encoder() works */ }
```

Decode capabilities follow the same pattern:

```rust
let caps = JpegDecoderConfig::capabilities();
if caps.cheap_probe() { /* probe() is fast, reads header only */ }
if caps.animation()   { /* full_frame_decoder() works */ }
if caps.row_level()   { /* streaming_decoder() works */ }
```

## Resource Limits

Protect against oversized or malicious inputs:

```rust
use zc::ResourceLimits;

let limits = ResourceLimits::none()
    .with_max_width(8192)
    .with_max_height(8192)
    .with_max_pixels(64_000_000)          // 64 megapixels
    .with_max_memory(512 * 1024 * 1024);  // 512 MB peak

let job = config.job().with_limits(limits);
let decoder = job.decoder(Cow::Borrowed(&data), &[])?; // fails early if limits exceeded
```

Limits are checked at job creation time (before allocation), not after decoding. The codec calls `limits.check_dimensions()`, `limits.check_memory()`, etc. during executor construction.

`ResourceLimits::none()` means no limits (all checks pass). Each `with_*` call adds one constraint.

For animation:

```rust
let limits = ResourceLimits::none()
    .with_max_frames(1000)
    .with_max_duration(30_000); // 30 seconds total
```

## Cancellation

Use `enough::Stop` for cooperative cancellation:

```rust
use enough::AtomicStop;

let stop = AtomicStop::new();

// In another thread or signal handler:
// stop.request_stop();

let job = config.job().with_stop(&stop);
let decoder = job.decoder(Cow::Borrowed(&data), &[])?;
let output = decoder.decode()?; // returns Err if stop was requested
```

The codec checks the stop token periodically during encode/decode. How often depends on the codec — typically once per MCU row or scanline batch.

## Policies

Policies control security-relevant behavior on a per-job basis:

```rust
use zc::encode::EncodePolicy;
use zc::decode::DecodePolicy;

// Strip all metadata, deterministic output
let enc_policy = EncodePolicy::strict();
let job = enc_config.job().with_policy(enc_policy);

// Or fine-grained control:
let enc_policy = EncodePolicy::none()
    .with_embed_icc(true)     // keep ICC
    .with_embed_exif(false)   // strip EXIF
    .with_deterministic(true);

// Decode: restrict what the decoder processes
let dec_policy = DecodePolicy::strict(); // minimal attack surface
let dec_policy = DecodePolicy::none()
    .with_allow_progressive(false)   // reject progressive JPEGs
    .with_allow_truncated(false)     // reject truncated files
    .with_strict(true);              // strict spec compliance
```

Policies use three-valued logic (`None` / `Some(true)` / `Some(false)`). `None` means "use codec default." The `resolve_*()` methods collapse to `bool` with codec-appropriate defaults.

## Metadata Roundtrip

Decode captures metadata. Pass it through to encode for lossless metadata roundtrip:

```rust
// Decode
let dec_output = decoder.decode()?;
let metadata = dec_output.metadata(); // borrows MetadataView

// Re-encode with same metadata
let job = enc_config.job()
    .with_metadata(&metadata);
let encoder = job.encoder()?;
let output = encoder.encode(dec_output.pixels())?;
```

`MetadataView` carries ICC profile, EXIF, XMP, CICP, orientation, HDR metadata (content light level, mastering display), and gain map metadata. The encoder embeds what it supports and what the policy allows.

Color management is not the codec's job. Decoders return native pixels with ICC/CICP metadata attached. Encoders accept pixels as-is and embed the provided metadata. If you need color space conversion, handle it between decode and encode using a CMS.

## Animation

### Decoding Animation

The `FullFrameDecoder` composites internally — it handles disposal, blending, sub-canvas positioning, and reference slots, then yields full-canvas frames ready for display.

```rust
use std::borrow::Cow;
use zc::decode::{DecodeJob, FullFrameDecoder};

let job = config.job();
let mut frame_dec = job.full_frame_decoder(Cow::Borrowed(&gif_bytes), &[])?;

println!("frames: {:?}", frame_dec.frame_count());
println!("loop count: {:?}", frame_dec.loop_count()); // Some(0) = infinite

while let Some(frame) = frame_dec.render_next_frame(None)? {
    let pixels = frame.pixels();
    let delay = frame.duration_ms();
    let index = frame.frame_index();
    // frame borrows the decoder's canvas — next call invalidates it.
    // Use render_next_frame_owned() if you need to keep frames.
}
```

### Encoding Animation

```rust
use zc::encode::{EncodeJob, FullFrameEncoder};

let job = config.job()
    .with_canvas_size(640, 480)
    .with_loop_count(Some(0)); // infinite loop

let mut frame_enc = job.full_frame_encoder()?;

// Push full-canvas frames (pixels, duration_ms, stop token):
frame_enc.push_frame(frame1_pixels, 100, None)?;
frame_enc.push_frame(frame2_pixels, 100, None)?;

let output = frame_enc.finish(None)?;
```

## Streaming Decode

For codecs that support it (check `capabilities().row_level()`), streaming decode yields scanline batches without buffering the entire image:

```rust
use zc::decode::StreamingDecode;

let mut stream = job.streaming_decoder(Cow::Borrowed(&data), &[])?;
let info = stream.info(); // dimensions, format, metadata

while let Some((y_offset, strip)) = stream.next_batch()? {
    // strip is a PixelSlice covering rows [y_offset .. y_offset + strip.rows())
    process_strip(y_offset, strip);
}
```

Strip height is codec-determined — it might be one row, eight rows (JPEG MCU height), or the full image. Don't assume a specific strip size.

## Push Decode (DecodeRowSink)

For zero-copy decoding into caller-provided buffers:

```rust
use zc::decode::{DecodeRowSink, SinkError};
use zenpixels::{PixelDescriptor, PixelSliceMut};

struct MyBuffer {
    buf: Vec<u8>,
}

impl DecodeRowSink for MyBuffer {
    fn begin(&mut self, width: u32, height: u32, descriptor: PixelDescriptor)
        -> Result<(), SinkError>
    {
        // Pre-allocate for the full image
        let stride = width as usize * descriptor.bytes_per_pixel();
        self.buf.resize(height as usize * stride, 0);
        Ok(())
    }

    fn provide_next_buffer(&mut self, y: u32, height: u32, width: u32,
        descriptor: PixelDescriptor) -> Result<PixelSliceMut<'_>, SinkError>
    {
        let bpp = descriptor.bytes_per_pixel();
        let stride = width as usize * bpp;
        let offset = y as usize * stride;
        let len = height as usize * stride;
        Ok(PixelSliceMut::new(
            &mut self.buf[offset..offset + len],
            width, height, stride, descriptor,
        ).expect("buffer sized correctly"))
    }

    fn finish(&mut self) -> Result<(), SinkError> {
        Ok(()) // flush or finalize if needed
    }
}

let mut sink = MyBuffer { buf: Vec::new() };
let output_info = job.push_decoder(Cow::Borrowed(&data), &mut sink, &[])?;
// sink.buf now contains the decoded pixels
```

The codec calls `begin()` once, then `provide_next_buffer()` for each strip of rows, then `finish()`. You provide the buffer; the codec fills it. This avoids an extra copy compared to `Decode::decode()` → copy into your buffer.

## Format Detection

```rust
use zc::ImageFormat;

let format = ImageFormat::from_magic(&file_bytes);
match format {
    ImageFormat::Jpeg => { /* ... */ }
    ImageFormat::Png  => { /* ... */ }
    _ => { /* unknown or unsupported */ }
}

// Format metadata
let mime = format.mime_type();          // "image/jpeg"
let ext = format.extension();          // "jpg"
let has_alpha = format.supports_alpha();
let animated = format.supports_animation();
```

## EncodeOutput Details

```rust
let output = encoder.encode(pixels)?;

output.data();       // &[u8] — borrow the encoded bytes
output.len();        // byte count
output.format();     // ImageFormat
output.mime_type();  // "image/jpeg" (may differ from format default, e.g. "image/apng")
output.extension();  // "jpg" (may differ from format default)
output.into_vec();   // Vec<u8> — take ownership
```

## Source Encoding Details

You can query how an image was originally encoded — useful for re-encoding at matching quality or for diagnostics. Available from both `ImageInfo` (after probe) and `DecodeOutput` (after decode):

```rust
use zc::SourceEncodingDetails;

// From probe (no decode needed):
let info = job.probe(&data)?;
if let Some(details) = info.source_encoding_details() {
    if let Some(q) = details.source_generic_quality() {
        println!("Source quality: {q:.0}");
    }
}

// Or from decode output:
let output = decoder.decode()?;
if let Some(details) = output.source_encoding_details() {
    // Generic quality on the 0-100 scale (same as with_generic_quality)
    if let Some(q) = details.source_generic_quality() {
        // Re-encode at matching quality:
        let enc_config = JpegEncoderConfig::new().with_generic_quality(q);
    }

    if details.is_lossless() {
        println!("Source was lossless");
    }

    // Color depth (PNG24=24, PNG32=32, PNG48=48, PNG64=64, GIF=8)
    if let Some(bpp) = details.source_bits_per_pixel() {
        println!("Source: {bpp} bits per pixel");
    }

    // Palette size for indexed formats (PNG indexed, GIF)
    if let Some(n) = details.source_palette_size() {
        println!("Palette: {n} colors");
    }

    // Downcast for codec-specific fields:
    if let Some(jpeg) = details.codec_details::<zenjpeg::detect::JpegProbe>() {
        println!("JPEG encoder: {:?}", jpeg.encoder);
        println!("Chroma subsampling: {:?}", jpeg.subsampling);
    }
}
```

Not all codecs populate this. `source_encoding_details()` returns `None` if the codec doesn't support quality detection.

## Error Handling

With the concrete API, errors are codec-specific:

```rust
match encoder.encode(pixels) {
    Ok(output) => { /* success */ }
    Err(e) => {
        // e is the codec's error type (e.g., JpegError)
        // Check if it's an unsupported operation:
        use zc::CodecErrorExt;
        if let Some(op) = e.unsupported_operation() {
            println!("not supported: {op}");
        }
    }
}
```

With the dyn API, errors are `BoxedError`. Downcast to the concrete type if needed.

## Threading

Control threading behavior per-job via `ResourceLimits`:

```rust
use zc::ThreadingPolicy;

let limits = ResourceLimits::none()
    .with_threading(ThreadingPolicy::SingleThread);     // force single-threaded
    // or: ThreadingPolicy::LimitOrSingle { max_threads: 4 }
    // or: ThreadingPolicy::Balanced
    // or: ThreadingPolicy::Unlimited (default)

let job = config.job().with_limits(limits);
```

How threading is applied depends on the codec. `ThreadingPolicy` is a hint; the codec may use fewer threads than requested but will respect `SingleThread`.
