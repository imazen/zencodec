//! Executable guide examples, each in its own scope so expanded imports and
//! complete function context are typechecked together. Native QOI and BMP are
//! lossless verification fixtures, not browser-format recommendations.
#![forbid(unsafe_code)]

mod preview_example {
    // EXAMPLE: preview
    use zenbitmaps::QoiDecoderConfig;
    use zencodec::decode::{DecodeOutput, DecoderConfig};
    type Error = Box<dyn std::error::Error + Send + Sync>;

    pub fn preview(bytes: &[u8]) -> Result<DecodeOutput, Error> {
        let config = QoiDecoderConfig::new();
        // ANCHOR: preview
        let image = config.decode(bytes)?;
        // END: preview
        Ok(image)
    }
    // END EXAMPLE: preview
}
pub use preview_example::preview;

mod encode_example {
    // EXAMPLE: encode
    use zenbitmaps::QoiEncoderConfig;
    use zencodec::encode::{EncodeOutput, EncoderConfig};
    use zenpixels::PixelSlice;
    type Error = Box<dyn std::error::Error + Send + Sync>;

    pub fn encode(pixels: PixelSlice<'_>) -> Result<EncodeOutput, Error> {
        let config = QoiEncoderConfig::new();
        // ANCHOR: encode
        let output = config.encode(pixels)?;
        // END: encode
        Ok(output)
    }
    // END EXAMPLE: encode
}
pub use encode_example::encode;

mod limits_example {
    // EXAMPLE: limits
    use std::borrow::Cow;
    use zenbitmaps::QoiDecoderConfig;
    use zencodec::ResourceLimits;
    use zencodec::decode::{Decode, DecodeJob, DecodeOutput, DecoderConfig};
    type Error = Box<dyn std::error::Error + Send + Sync>;

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
    // END EXAMPLE: limits
}
pub use limits_example::limited;

mod preference_example {
    // EXAMPLE: preference
    use std::borrow::Cow;
    use zenbitmaps::QoiDecoderConfig;
    use zencodec::decode::{Decode, DecodeJob, DecodeOutput, DecoderConfig};
    use zenpixels::PixelDescriptor;
    type Error = Box<dyn std::error::Error + Send + Sync>;

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
    // END EXAMPLE: preference
}
pub use preference_example::preference;

mod metadata_example {
    // EXAMPLE: metadata
    use zenbitmaps::QoiEncoderConfig;
    use zencodec::MetadataPolicy;
    use zencodec::decode::DecodeOutput;
    use zencodec::encode::{EncodeJob, EncodeOutput, EncoderConfig};
    type Error = Box<dyn std::error::Error + Send + Sync>;

    #[allow(deprecated)] // This QOI example demonstrates best-effort metadata filtering.
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
    // END EXAMPLE: metadata
}
pub use metadata_example::retained;

mod slice_example {
    // EXAMPLE: slice
    use zenbitmaps::QoiDecoderConfig;
    use zencodec::decode::{DecodeJob, DecoderConfig};
    use zencodec_io::{CollectSink, drive_from_slice};
    type Error = Box<dyn std::error::Error + Send + Sync>;

    pub fn from_slice(bytes: &[u8]) -> Result<CollectSink, Error> {
        let mut decoder = QoiDecoderConfig::new().job().dyn_incremental_decoder(&[])?;
        let mut sink = CollectSink::new();
        // ANCHOR: slice
        // None adds no per-call control; the decoder retains its job control.
        drive_from_slice(decoder.as_mut(), bytes, &mut sink, None, 4096)?;
        // END: slice
        Ok(sink)
    }
    // END EXAMPLE: slice
}
pub use slice_example::from_slice;

mod positioned_example {
    // EXAMPLE: positioned
    use zenbitmaps::QoiDecoderConfig;
    use zencodec::decode::{DecodeJob, DecoderConfig};
    use zencodec::source::ReadAt;
    use zencodec_io::{CollectSink, DriveOptions, drive_from_read_at};
    type Error = Box<dyn std::error::Error + Send + Sync>;

    pub fn positioned<S: ReadAt>(bytes: &mut S) -> Result<CollectSink, Error> {
        let mut decoder = QoiDecoderConfig::new().job().dyn_incremental_decoder(&[])?;
        let mut sink = CollectSink::new();
        // ANCHOR: positioned
        // Pass Some(pulse) to add a full borrowed reporting/cancellation control.
        drive_from_read_at(
            decoder.as_mut(),
            bytes,
            &mut sink,
            None,
            DriveOptions::default(),
        )?;
        // END: positioned
        Ok(sink)
    }
    // END EXAMPLE: positioned
}
pub use positioned_example::positioned;

mod batch_example {
    // EXAMPLE: batch
    use zencodec::source::ReadAt;
    type Error = Box<dyn std::error::Error + Send + Sync>;

    pub fn batch<S: ReadAt>(
        source: &mut S,
        first: &mut [u8],
        second: &mut [u8],
    ) -> Result<(), Error> {
        // ANCHOR: batch
        source.read_many_at(&mut [(0, first), (8192, second)])?;
        // END: batch
        Ok(())
    }
    // END EXAMPLE: batch
}
pub use batch_example::batch;

mod async_example {
    // EXAMPLE: async
    use std::sync::Arc;
    use zenbitmaps::QoiDecoderConfig;
    use zencodec::decode::DecoderConfig;
    use zencodec_io::tokio_io::{BytesSource, Inline, decode_async};
    use zencodec_io::{CollectSink, DriveOptions};
    type Error = Box<dyn std::error::Error + Send + Sync>;

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
        .await?;
        // END: async
        Ok(sink)
    }
    // END EXAMPLE: async
}
pub use async_example::asynchronous;

mod finalize_example {
    // EXAMPLE: finalize
    use std::sync::Arc;
    use zenbitmaps::{QoiDecoderConfig, QoiEncoderConfig};
    use zencodec::decode::DecoderConfig;
    use zencodec::encode::{EncodeJob, EncodeOutput, EncoderConfig};
    use zencodec_io::tokio_io::{BytesSource, Inline, decode_async};
    use zencodec_io::{DriveOptions, EncoderSink};
    type Error = Box<dyn std::error::Error + Send + Sync>;

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
        .await?;
        let output = sink.finish()?;
        // The encoder is consumed before the application awaits its output writer.
        // END: finalize
        Ok(output)
    }
    // END EXAMPLE: finalize
}
pub use finalize_example::finish_before_write;

mod bridge_example {
    // EXAMPLE: bridge
    use zenbitmaps::QoiDecoderConfig;
    use zencodec::decode::{DecoderConfig, erase_incremental};
    use zencodec::helpers::WholeFileIncremental;
    use zencodec_io::{CollectSink, drive_from_slice};
    type Error = Box<dyn std::error::Error + Send + Sync>;

    pub fn bridge(bytes: &[u8]) -> Result<CollectSink, Error> {
        let job = QoiDecoderConfig::new().job();
        // ANCHOR: bridge
        let mut decoder = erase_incremental(WholeFileIncremental::new(job, &[]));
        // END: bridge
        let mut sink = CollectSink::new();
        drive_from_slice(decoder.as_mut(), bytes, &mut sink, None, 3)?;
        Ok(sink)
    }
    // END EXAMPLE: bridge
}
pub use bridge_example::bridge;

pub fn bounded_batch_example() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use zencodec::source::SliceReadAt;
    let bytes: Vec<u8> = (0..16384).map(|i| (i % 251) as u8).collect();
    let mut source = SliceReadAt::new(&bytes);
    let (mut first, mut second) = ([0; 32], [0; 32]);
    batch(&mut source, &mut first, &mut second)?;
    assert_eq!(first, bytes[..32]);
    assert_eq!(second, bytes[8192..8224]);
    Ok(())
}

mod probe_example {
    // EXAMPLE: probe
    use zenbitmaps::QoiDecoderConfig;
    use zencodec::ImageInfo;
    use zencodec::decode::DecoderConfig;
    type Error = Box<dyn std::error::Error + Send + Sync>;

    pub fn probe(bytes: &[u8]) -> Result<ImageInfo, Error> {
        let config = QoiDecoderConfig::new();
        // ANCHOR: probe
        let info = config.probe(bytes)?;
        // END: probe
        // Probe borrows the config; another probe can reuse it.
        Ok(info)
    }
    // END EXAMPLE: probe
}
pub use probe_example::probe;

mod codecset_example {
    // EXAMPLE: codecset
    use zenbitmaps::{BmpDecoderConfig, BmpEncoderConfig, QoiDecoderConfig, QoiEncoderConfig};
    use zencodec::decode::DecodeOutput;
    use zencodec::{CodecSet, CodecSetError};

    // Build once at application startup, then share by reference or Arc.
    pub fn gallery_codecs() -> CodecSet {
        CodecSet::new()
            .with_decoder(QoiDecoderConfig::new())
            .with_decoder(BmpDecoderConfig::new())
            .with_encoder(QoiEncoderConfig::new())
            .with_encoder(BmpEncoderConfig::new())
    }

    pub fn registered_preview(
        codecs: &CodecSet,
        bytes: &[u8],
    ) -> Result<DecodeOutput, CodecSetError> {
        // ANCHOR: codecset
        let image = codecs.decode(bytes)?;
        // END: codecset
        // Detects among registered decoders; codecs remains available for the next request.
        Ok(image)
    }
    // END EXAMPLE: codecset
}
pub use codecset_example::{gallery_codecs, registered_preview};

mod codecset_encode_example {
    // EXAMPLE: codecset_encode
    use zencodec::encode::EncodeOutput;
    use zencodec::{CodecSet, CodecSetError, ImageFormat};
    use zenpixels::PixelSlice;

    pub fn registered_encode(
        codecs: &CodecSet,
        pixels: PixelSlice<'_>,
    ) -> Result<EncodeOutput, CodecSetError> {
        // ANCHOR: codecset_encode
        let output = codecs.encode(ImageFormat::Qoi, pixels)?;
        // END: codecset_encode
        // The set must already contain a QOI encoder configuration.
        Ok(output)
    }
    // END EXAMPLE: codecset_encode
}
pub use codecset_encode_example::registered_encode;

mod codecset_job_example {
    // EXAMPLE: codecset_job
    use zencodec::decode::DecodeOutput;
    use zencodec::encode::EncodeOutput;
    use zencodec::{CodecSet, ImageFormat, MetadataPolicy};
    type Error = Box<dyn std::error::Error + Send + Sync>;

    #[allow(deprecated)] // Preserve the explicit QOI compatibility example.
    pub fn registered_derivative(
        codecs: &CodecSet,
        image: &DecodeOutput,
    ) -> Result<EncodeOutput, Error> {
        // ANCHOR: codecset_job
        let mut job = codecs.encode_job(ImageFormat::Qoi)?;
        job.set_metadata_policy(image.metadata(), MetadataPolicy::Web);
        let output = job.encode(image.pixels())?;
        // END: codecset_job
        // Dyn jobs use setters; typed jobs use consuming with_* builders.
        Ok(output)
    }
    // END EXAMPLE: codecset_job
}
pub use codecset_job_example::registered_derivative;

mod codecset_incremental_example {
    // EXAMPLE: codecset_incremental
    use zencodec::CodecSet;
    use zencodec_io::{CollectSink, drive_from_slice};
    type Error = Box<dyn std::error::Error + Send + Sync>;

    pub fn registered_incremental(codecs: &CodecSet, bytes: &[u8]) -> Result<CollectSink, Error> {
        let mut sink = CollectSink::new();
        // ANCHOR: codecset_incremental
        let mut decoder = codecs.incremental_decoder(&[]);
        // None preserves any SharedPulse already configured on the set's jobs.
        drive_from_slice(decoder.as_mut(), bytes, &mut sink, None, 4096)?;
        // END: codecset_incremental
        // Detection happens as the driver supplies the initial bytes.
        Ok(sink)
    }
    // END EXAMPLE: codecset_incremental
}
pub use codecset_incremental_example::registered_incremental;

mod transcode_example {
    // EXAMPLE: transcode
    use zencodec::encode::{EncodeOutput, Fidelity};
    use zencodec::{CodecSet, CodecSetError, ColorEmitPolicy, ImageFormat, MetadataPolicy};

    pub fn registered_transcode(
        codecs: &CodecSet,
        bytes: &[u8],
    ) -> Result<EncodeOutput, CodecSetError> {
        // ANCHOR: transcode
        let output = codecs.transcode(
            bytes,
            ImageFormat::Qoi,
            Fidelity::Lossless,
            MetadataPolicy::Web,
            ColorEmitPolicy::Balanced,
        )?;
        // END: transcode
        // May refuse a descriptor, sequence, or auxiliary-image barrier.
        // This convenience performs no resize, CMS conversion, or silent first-frame selection.
        Ok(output)
    }
    // END EXAMPLE: transcode
}
pub use transcode_example::registered_transcode;
