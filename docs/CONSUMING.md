# Using zen* Codecs

This guide covers encoding and decoding images with any zen* codec through the `zencodec` API.

## Dependencies

Add the codec crate(s) you need and `zenpixels` for pixel types:

```toml
[dependencies]
zenjpeg = "0.1"
zenpng = "0.1"
zenpixels = { version = "0.1", features = ["buffer"] }
```

Each codec crate re-exports `zencodec`, so you don't need to depend on it directly. Import the traits from `zencodec::encode` and `zencodec::decode`.

For codec-agnostic multi-format dispatch, use `zencodecs` instead of individual codec crates.

## Basic Encode

The flow is always **Config → Job → Encoder → output**.

```rust
use zenjpeg::JpegEncoderConfig;
use zencodec::encode::{EncoderConfig, EncodeJob, Encoder};
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
use zencodec::decode::{DecoderConfig, DecodeJob, Decode};

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
use zencodec::decode::{is_format_available, negotiate_pixel_format};

// Does this decoder support RGBA8 output at all?
let supported = JpegDecoderConfig::supported_descriptors();
if is_format_available(PixelFormat::Rgba8, supported) {
    // ...
}
```

On the encode side, check whether the encoder accepts your pixel format:

```rust
use zencodec::encode::best_encode_format;

let supported = JpegEncoderConfig::supported_descriptors();
if let Some(fmt) = best_encode_format(my_pixels.descriptor(), supported) {
    // Encoder accepts this format (possibly after non-lossy conversion)
}
```

## Codec-Agnostic Code (Dyn Dispatch)

For code that works with any codec — plugin systems, multi-format pipelines, format-selection-at-runtime — use the `Dyn*` traits:

```rust
use zencodec::encode::{DynEncoderConfig, BoxedError};
use zencodec::decode::DynDecoderConfig;
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
use zencodec::encode::EncoderConfig;

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
if caps.push_rows()   { /* push_rows() + finish() is supported */ }
if caps.streaming_output() { /* some configurations stream; begin() says which */ }
if caps.encode_from() { /* encode_from() callback is supported */ }
if caps.animation() { /* animation_frame_encoder() works */ }
```

Decode capabilities follow the same pattern:

```rust
let caps = JpegDecoderConfig::capabilities();
if caps.cheap_probe() { /* probe() is fast, reads header only */ }
if caps.animation()   { /* animation_frame_decoder() works */ }
if caps.streaming()   { /* streaming_decoder() works */ }
if caps.incremental() { /* dyn_incremental_decoder(): sans-I/O, decodes as bytes arrive */ }
if caps.seekable()    { /* push_decoder_from_source(): reads ranges from a ReadAt */ }
```

## Resource Limits

Protect against oversized or malicious inputs:

```rust
use zencodec::ResourceLimits;

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
    .with_max_animation_ms(30_000); // 30 seconds total
```

## Cancellation

Use `enough::Stop` for cooperative cancellation:

```rust
use almost_enough::{Stopper, StopExt};

let stopper = Stopper::new();

// In another thread or signal handler:
// stopper.cancel();

let job = config.job().with_stop(stopper.clone().into_token());
let decoder = job.decoder(Cow::Borrowed(&data), &[])?;
let output = decoder.decode()?; // returns Err if stop was requested
```

The codec checks the stop token periodically during encode/decode. How often depends on the codec — typically once per MCU row or scanline batch.

`StopToken` is `Clone + Send + Sync + 'static` — it's an owned, type-erased stop handle. Convert any `Stop + 'static` with `stop.into_token()` or `StopToken::new(stop)`.

## Policies

Policies control security-relevant behavior on a per-job basis:

```rust
use zencodec::encode::EncodePolicy;
use zencodec::decode::DecodePolicy;

// Strip all metadata
let enc_policy = EncodePolicy::strip_all();
let job = enc_config.job().with_policy(enc_policy);

// Or fine-grained control:
let enc_policy = EncodePolicy::none()
    .with_embed_icc(true)     // keep ICC
    .with_embed_exif(false)   // strip EXIF
    .with_embed_xmp(false);   // strip XMP

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
let metadata = dec_output.metadata(); // returns Metadata (Arc-backed, cheap clone)

// Re-encode, choosing a retention policy explicitly.
// PreserveExact = verbatim roundtrip; MetadataPolicy::Web = strip camera/GPS,
// keep orientation + rights (privacy-safe).
let job = enc_config.job()
    .with_metadata_policy(metadata, MetadataPolicy::PreserveExact);
let encoder = job.encoder()?;
let output = encoder.encode(dec_output.pixels())?;
```

`Metadata` carries ICC profile, EXIF, XMP, CICP, orientation, and HDR metadata (content light level, mastering display). Retention is an explicit choice at embed time: `with_metadata_policy(meta, policy)` filters the record before the codec sees it. The plain `with_metadata(meta)` still works but is `#[deprecated]` — the compiler warns when no policy is chosen, so metadata can't propagate silently. The encoder then embeds what its format supports.

Color management is not the codec's job. Decoders return native pixels with ICC/CICP metadata attached. Encoders accept pixels as-is and embed the provided metadata. If you need color space conversion, handle it between decode and encode using a CMS.

## Animation

### Decoding Animation

The `AnimationFrameDecoder` composites internally — it handles disposal, blending, sub-canvas positioning, and reference slots, then yields full-canvas frames ready for display.

```rust
use std::borrow::Cow;
use zencodec::decode::{DecodeJob, AnimationFrameDecoder};

let job = config.job();
let mut frame_dec = job.animation_frame_decoder(Cow::Borrowed(&gif_bytes), &[])?;

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
use zencodec::encode::{EncodeJob, AnimationFrameEncoder};

let job = config.job()
    .with_canvas_size(640, 480)
    .with_loop_count(Some(0)); // infinite loop

let mut frame_enc = job.animation_frame_encoder()?;

// Push full-canvas frames (pixels, duration_ms, stop token):
frame_enc.push_frame(frame1_pixels, 100, None)?;
frame_enc.push_frame(frame2_pixels, 100, None)?;

let output = frame_enc.finish(None)?;
```

## Streaming Decode

For codecs that support it (check `capabilities().streaming()`), streaming decode yields scanline batches without buffering the entire image:

```rust
use zencodec::decode::StreamingDecode;

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
use zencodec::decode::{DecodeRowSink, SinkError};
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

## Decoding as Bytes Arrive (`zencodec-io`)

Codecs that declare `capabilities().incremental()` return a resumable decoder
from `job.dyn_incremental_decoder(&[])`. It performs no I/O; the drivers in the
`zencodec-io` workspace crate feed it from wherever the bytes are, and rows go
into any `DecodeRowSink` as soon as the decoder has enough input.

```rust
use zencodec_io::{CollectSink, DriveOptions, FileReadAt, drive_from_read_at, pump_read_to_sink};

// A file (or any ReadAt: object store, FFI callback). Range requests are
// batched into one read_many_at per round.
let mut dec = config.clone().job().dyn_incremental_decoder(&[])?;
let mut src = FileReadAt::open("photo.qoi")?;
let mut sink = CollectSink::new();
let (info, _stats) = drive_from_read_at(&mut *dec, &mut src, &mut sink, None, DriveOptions::default())?;

// A sequential stream (an HTTP body, stdin): rows come out while it downloads.
let mut dec = config.job().dyn_incremental_decoder(&[])?;
let info = pump_read_to_sink(body_reader, &mut *dec, &mut sink, None, 64 << 10, 16 << 20)?;
```

Bytes already in memory go through `drive_from_slice`, which copies nothing.
A random-access decoder over a stream needs `BufferedReadAt` (choose the policy
with `BufferPolicy::for_pattern(dec.access_pattern(), max_bytes)`); the pump
reports `NeedsRandomAccess` instead of guessing.

**Async (feature `tokio`).** `tokio_io::decode_async(decoder, sink, &mut source,
compute, opts, pulse)` keeps the decoder in your task, awaits an `AsyncSource`
(`AsyncReadSource` wraps any `AsyncRead`; implement `read_ranges` to overlap
range requests) and runs decode batches on `compute`: a closure such as
`|job| { tokio::task::spawn_blocking(job); }`, or `Inline` for single-threaded
hosts. No thread waits on the network and no codec code is `async`. The futures
are `Send`, so they go to `tokio::spawn`. To cancel while waiting for bytes,
drop the future or race it in `tokio::select!`; the pulse stops a running batch
within one step.

**Transcode with bounded memory.** `tokio_io::transcode_async(decoder, encoder,
&mut source, &mut writer, compute, opts, pulse)` routes rows into the encoder
through `EncoderSink` and writes encoded bytes as they become available
(encoders whose `begin` reports a streaming `OutputTiming`), so memory stays
at the input window, one strip and the encoder's unsent output.
`EncoderSink::output_timing()` reports the encoder's answer once the decoder
has declared the image. Synchronously, drive a decoder into
`EncoderSink::new(encoder)` and call `write_pending` between steps.

**Stream an encode you feed yourself.** For an image already in memory,
`zencodec_io::encode_to_writer` does the whole loop and falls back to a
one-shot encode when the configuration can't encode from rows:

```rust
let config = config.with_prefer_streaming(true);  // codec-agnostic ask
let enc = config.job().with_metadata_policy(meta, policy).encoder()?;
let report = zencodec_io::encode_to_writer(enc, pixels, &mut out)?;
```

Feeding rows as they are produced, `begin` declares the image and says when
bytes arrive; send what is pending after `begin` (the header) and after each
strip. `write_pending` writes and consumes together, so the same bytes are
never sent twice:

```rust
use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};
use zencodec_io::write_pending;

let mut enc = config.job().with_metadata_policy(meta, policy).encoder()?;
let timing = enc.begin(width, height, descriptor)?;
if !timing.streams() {
    // Everything arrives from finish(): buffer, or send a Content-Length.
}
write_pending(&mut enc, &mut out)?;         // the header, right after begin
for strip in strips {                       // preferred_strip_height() rows each
    enc.push_rows(strip)?;
    write_pending(&mut enc, &mut out)?;
}
out.write_all(enc.finish()?.data())?;
```

Under async, `zencodec_io::tokio_io::write_pending_async` is the same step;
awaiting it before the next `push_rows` is the backpressure. To hand bytes
onward as owned buffers (a `Bytes` body chunk, an upload part), call
`enc.take_output(&mut Vec::new())` instead: encoders built on
`OutputBuffer` give you their allocation without a copy.

Animation encoders stream the same way, counted in frames:
`AnimationFrameEncoder::begin(width, height, descriptor, frame_count)`
returns an `AnimationOutputTiming`, and `write_pending_animation` drains
after each `push_frame`. Give the frame count when you know it; formats
that write it first (APNG) can stream only then.

`timing.lag_rows()` is the most rows the encoder holds before their bytes
are pending. A configuration that cannot encode from rows fails at `begin`
with `UnsupportedOperation::RowLevelEncode`. If `push_rows` or `finish` fails
after bytes went out, the file can't be completed: abort the transport
(close the connection, abort the upload) instead of ending it normally.

Measured end to end over HTTP (zqi `benchmarks/stream_http_2026-10-14`), a
streaming encoder's first byte leaves within 0.25 ms instead of after the
whole encode, and server memory under concurrent 12 MP requests drops from
hundreds of MB to tens.

**Pull-native codecs** (`capabilities().seekable()`) read from a source
directly: `job.push_decoder_from_source(&mut source, &mut sink, &[])`, or
`tokio_io::pull_decode_on_thread` under async.

Every driver takes `Option<&dyn how_far::Pulse>` for cancellation and
progress; a plain stop passes as `how_far::StopOnly::borrowed(&stop)`.
Contract and measurements: [range-decoding-contract.md](range-decoding-contract.md).

## Format Detection

```rust
use zencodec::ImageFormat;

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
use zencodec::SourceEncodingDetails;

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

    // Downcast for codec-specific fields.
    // The trait only carries universal properties (quality, lossless).
    // Everything else lives on the concrete probe struct.
    if let Some(jpeg) = details.codec_details::<zenjpeg::detect::JpegProbe>() {
        println!("JPEG encoder: {:?}", jpeg.encoder);
        println!("Chroma subsampling: {:?}", jpeg.subsampling);
    }

    if let Some(png) = details.codec_details::<zenpng::detect::PngProbe>() {
        println!("PNG{}  ({:?}, {}bpp)",
            png.bits_per_pixel(), png.color_type, png.bit_depth);
        if png.color_type == zenpng::detect::ColorType::Indexed {
            println!("Palette: {} colors", png.palette_size);
        }
    }

    if let Some(gif) = details.codec_details::<zengif::detect::GifProbe>() {
        println!("GIF palette: {} colors", gif.global_palette_size);
    }
}
```

The trait intentionally has very few methods — only properties meaningful across all image formats. Codec-specific details (color type, bit depth, palette size, chroma subsampling, encoder family, quantizer tables) live on the concrete probe struct and are accessed via `codec_details::<T>()` downcast.

Not all codecs populate this. `source_encoding_details()` returns `None` if the codec doesn't support source detection.

## Error Handling

With the concrete API, errors are codec-specific:

```rust
match encoder.encode(pixels) {
    Ok(output) => { /* success */ }
    Err(e) => {
        // e is the codec's error type (e.g., JpegError)
        // Check if it's an unsupported operation:
        use zencodec::CodecErrorExt;
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
use zencodec::ThreadingPolicy;

let limits = ResourceLimits::none()
    .with_threading(ThreadingPolicy::SingleThread);     // force single-threaded
    // or: ThreadingPolicy::LimitOrSingle { max_threads: 4 }
    // or: ThreadingPolicy::Balanced
    // or: ThreadingPolicy::Unlimited (default)

let job = config.job().with_limits(limits);
```

How threading is applied depends on the codec. `ThreadingPolicy` is a hint; the codec may use fewer threads than requested but will respect `SingleThread`.
