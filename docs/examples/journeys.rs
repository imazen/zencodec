//! Executable guide fragments. Native QOI is a small lossless verification
//! fixture for these shared interfaces, not a suggested browser image format.
#![forbid(unsafe_code)]
use std::borrow::Cow;
use std::sync::Arc;
use zenbitmaps::{QoiDecoderConfig, QoiEncoderConfig};
use zencodec::decode::{Decode, DecodeJob, DecodeOutput, DecoderConfig, erase_incremental};
use zencodec::encode::{EncodeJob, EncodeOutput, EncoderConfig};
use zencodec::helpers::WholeFileIncremental;
use zencodec::source::{ReadAt, SliceReadAt};
use zencodec::{MetadataPolicy, ResourceLimits};
use zencodec_io::tokio_io::{BytesSource, Inline, decode_async};
use zencodec_io::{CollectSink, DriveOptions, EncoderSink, drive_from_read_at, drive_from_slice};
use zenpixels::{PixelDescriptor, PixelSlice};
type Error = Box<dyn std::error::Error + Send + Sync>;

pub fn preview(bytes: &[u8]) -> Result<DecodeOutput, Error> {
    let config = QoiDecoderConfig::new();
    // ANCHOR: preview
    let image = config.decode(bytes)?;
    // END: preview
    Ok(image)
}

pub fn encode(pixels: PixelSlice<'_>) -> Result<EncodeOutput, Error> {
    let config = QoiEncoderConfig::new();
    // ANCHOR: encode
    let output = config.encode(pixels)?;
    // END: encode
    Ok(output)
}

pub fn limited(bytes: &[u8], max_pixels: u64) -> Result<DecodeOutput, Error> {
    let config = QoiDecoderConfig::new();
    // ANCHOR: limits
    let limits = ResourceLimits::none().with_max_pixels(max_pixels);
    let image = config
        .job()
        .with_limits(limits)
        .decoder(Cow::Borrowed(bytes), &[])?
        .decode()?;
    // END: limits
    Ok(image)
}

pub fn preference(bytes: &[u8]) -> Result<DecodeOutput, Error> {
    let config = QoiDecoderConfig::new();
    // ANCHOR: preference
    let preferred = &[PixelDescriptor::RGBA8_SRGB, PixelDescriptor::RGB8_SRGB];
    let image = config
        .job()
        .decoder(Cow::Borrowed(bytes), preferred)?
        .decode()?;
    let actual = image.descriptor();
    // END: preference
    assert!(actual.bytes_per_pixel() >= 3);
    Ok(image)
}

pub fn retained(image: &DecodeOutput) -> Result<EncodeOutput, Error> {
    let config = QoiEncoderConfig::new();
    // ANCHOR: metadata
    let output = config
        .job()
        .with_metadata_policy(image.metadata(), MetadataPolicy::Web)
        .encode(image.pixels())?;
    // END: metadata
    Ok(output)
}

pub fn from_slice(bytes: &[u8]) -> Result<CollectSink, Error> {
    let mut decoder = QoiDecoderConfig::new().job().dyn_incremental_decoder(&[])?;
    let mut sink = CollectSink::new();
    // ANCHOR: slice
    drive_from_slice(decoder.as_mut(), bytes, &mut sink, None, 4096).map_err(|e| e.to_string())?;
    // END: slice
    Ok(sink)
}

pub fn positioned<S: ReadAt>(bytes: &mut S) -> Result<CollectSink, Error> {
    let mut decoder = QoiDecoderConfig::new().job().dyn_incremental_decoder(&[])?;
    let mut sink = CollectSink::new();
    // ANCHOR: positioned
    drive_from_read_at(
        decoder.as_mut(),
        bytes,
        &mut sink,
        None,
        DriveOptions::default(),
    )
    .map_err(|e| e.to_string())?;
    // END: positioned
    Ok(sink)
}

pub fn batch<S: ReadAt>(source: &mut S, first: &mut [u8], second: &mut [u8]) -> Result<(), Error> {
    // ANCHOR: batch
    source.read_many_at(&mut [(0, first), (8192, second)])?;
    // END: batch
    Ok(())
}

pub async fn asynchronous(bytes: Arc<[u8]>) -> Result<CollectSink, Error> {
    let decoder = QoiDecoderConfig::new()
        .job()
        .into_incremental_decoder(&[])?;
    let sink = CollectSink::new();
    // BytesSource is the deterministic stand-in for an application's AsyncSource.
    let mut source = BytesSource(bytes);
    let compute = Inline;
    // ANCHOR: async
    let (_, sink) = decode_async(
        decoder,
        sink,
        &mut source,
        compute,
        DriveOptions::default(),
        None,
    )
    .await
    .map_err(|e| e.to_string())?;
    // END: async
    Ok(sink)
}

pub async fn finish_before_write(bytes: Arc<[u8]>) -> Result<EncodeOutput, Error> {
    let decoder = QoiDecoderConfig::new()
        .job()
        .into_incremental_decoder(&[])?;
    let encoder = QoiEncoderConfig::new().job().encoder()?;
    let mut source = BytesSource(bytes);
    let compute = Inline;
    // ANCHOR: finalize
    let sink = EncoderSink::new(encoder);
    let (_, sink) = decode_async(
        decoder,
        sink,
        &mut source,
        compute,
        DriveOptions::default(),
        None,
    )
    .await
    .map_err(|e| e.to_string())?;
    let output = sink.finish()?;
    // The encoder is consumed before the application awaits its output writer.
    // END: finalize
    Ok(output)
}

pub fn bridge(bytes: &[u8]) -> Result<CollectSink, Error> {
    let job = QoiDecoderConfig::new().job();
    // ANCHOR: bridge
    let mut decoder = erase_incremental(WholeFileIncremental::new(job, &[]));
    // END: bridge
    let mut sink = CollectSink::new();
    drive_from_slice(decoder.as_mut(), bytes, &mut sink, None, 3).map_err(|e| e.to_string())?;
    Ok(sink)
}

pub fn bounded_batch_example() -> Result<(), Error> {
    let bytes: Vec<u8> = (0..16384).map(|i| (i % 251) as u8).collect();
    let mut source = SliceReadAt::new(&bytes);
    let (mut first, mut second) = ([0; 32], [0; 32]);
    batch(&mut source, &mut first, &mut second)?;
    assert_eq!(first, bytes[..32]);
    assert_eq!(second, bytes[8192..8224]);
    Ok(())
}
