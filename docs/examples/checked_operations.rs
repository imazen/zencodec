//! Executable checked-operation recipes, qualified by the native JPEG harness.
#![forbid(unsafe_code)]

mod registration {
    // EXAMPLE: checked_registry
    use zencodec::CodecSet;
    use zencodecs::{JpegDecoderConfig, PngDecoderConfig, PngEncoderConfig};

    pub fn gallery_codecs() -> CodecSet {
        // ANCHOR: checked_registry
        CodecSet::new()
            .with_decoder(JpegDecoderConfig::new())
            .with_decoder(PngDecoderConfig::new())
            .with_send_encoder(PngEncoderConfig::new())
        // END: checked_registry
    }
    // END EXAMPLE: checked_registry
}
pub(crate) use registration::gallery_codecs;

mod ordinary {
    // EXAMPLE: checked_decode
    use zencodec::{
        At, CodecSet,
        decode::DecodeOutput,
        operation::{DecodeRequest, OperationError, Outcome},
    };

    pub fn preview(
        codecs: &CodecSet,
        bytes: &[u8],
    ) -> Result<Outcome<DecodeOutput>, At<OperationError>> {
        // ANCHOR: checked_decode
        let image = DecodeRequest::new().decode(codecs, bytes)?;
        // END: checked_decode
        // Keep image.pixels().descriptor() and image.info().source_color with the samples.
        Ok(image)
    }
    // END EXAMPLE: checked_decode
}
pub(crate) use ordinary::preview;

mod automatic {
    // EXAMPLE: checked_default
    use zencodec::{At, CodecSet, ImageFormat, encode::EncodeOutput, operation::Outcome};
    use zencodecs::checked_transcode::{Error, TranscodeRequest};

    pub fn derivative(codecs: &CodecSet, bytes: &[u8]) -> Result<Outcome<EncodeOutput>, At<Error>> {
        // ANCHOR: checked_default
        let output = TranscodeRequest::new(ImageFormat::Png).transcode(codecs, bytes)?;
        // END: checked_default
        // Default retention can refuse when the target cannot preserve a reported field.
        // No file or HTTP body has been written by this operation.
        Ok(output)
    }
    // END EXAMPLE: checked_default
}
pub(crate) use automatic::derivative;

mod print {
    // EXAMPLE: checked_resolution
    use zencodec::{
        At, CodecSet, ImageFormat, Resolution,
        encode::EncodeOutput,
        operation::{EncodeRequest, OperationError, Outcome},
    };
    use zenpixels::PixelSlice;

    pub fn with_density(
        codecs: &CodecSet,
        pixels: PixelSlice<'_>,
        resolution: Resolution,
    ) -> Result<Outcome<EncodeOutput>, At<OperationError>> {
        // ANCHOR: checked_resolution
        let output = EncodeRequest::new(ImageFormat::Png)
            .with_resolution(resolution)
            .encode(codecs, pixels)?;
        // END: checked_resolution
        // Required means exact: 254 DPI fits PNG; 300 DPI refuses.
        // No implicit rounding or unit change may alter the represented density.
        Ok(output)
    }
    // END EXAMPLE: checked_resolution
}
pub(crate) use print::with_density;

mod web {
    // EXAMPLE: checked_web
    use zencodec::{
        ImageFormat,
        operation::{MetadataTransfer, PolicySet},
    };
    use zencodecs::checked_transcode::TranscodeRequest;
    use zenpixels::{Cicp, ConvertOptions, PixelFormat};
    use zenpixels_convert::{OutputProfile, cms_moxcms::MoxCms};

    pub fn web_request() -> (TranscodeRequest<'static>, PolicySet) {
        // ANCHOR: checked_web
        let request = TranscodeRequest::new(ImageFormat::Png)
            .with_cms(&MoxCms)
            .with_output(
                OutputProfile::Named(Cicp::SRGB),
                PixelFormat::Rgb8,
                ConvertOptions::retain_channels(),
            )
            .with_metadata(MetadataTransfer::DiscardDescriptive);
        let policy = PolicySet::new()
            .require_metadata(MetadataTransfer::DiscardDescriptive)
            .allow_opaque_profiles(false);
        (request, policy)
        // END: checked_web
        // Accept JPEG's RGB samples here. Alpha removal and depth loss are not permitted.
        // Current color is converted; orientation needed for rendering is retained.
    }
    // END EXAMPLE: checked_web
}
pub(crate) use web::web_request;

mod tenant {
    // EXAMPLE: checked_callback
    use zencodec::{
        At, CodecSet,
        encode::EncodeOutput,
        operation::{BoundPolicy, Decision, Outcome, PolicySet, Question},
    };
    use zencodecs::checked_transcode::{Error, TranscodeRequest};

    #[derive(Debug)]
    pub struct WorkspaceEvidenceRequired;
    impl std::fmt::Display for WorkspaceEvidenceRequired {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("tenant requires a qualified native workspace bound")
        }
    }
    impl std::error::Error for WorkspaceEvidenceRequired {}

    pub fn for_tenant(
        request: &TranscodeRequest<'_>,
        codecs: &CodecSet,
        bytes: &[u8],
        requirements: PolicySet,
        require_workspace_evidence: bool,
    ) -> Result<Outcome<EncodeOutput>, At<Error>> {
        // ANCHOR: checked_callback
        let mut policy = BoundPolicy::new(
            requirements,
            |question: Question<'_>| -> Result<Decision, At<WorkspaceEvidenceRequired>> {
                if let Question::Transcode { choices, .. } = question
                    && require_workspace_evidence
                {
                    let choice = choices
                        .iter()
                        .find(|choice| choice.value().workspace_upper_bound.is_some())
                        .ok_or_else(|| At::wrap(WorkspaceEvidenceRequired))?;
                    return Ok(Decision::choose(choice));
                }
                Ok(Decision::UsePreset)
            },
        );
        request.transcode_with(codecs, bytes, &mut policy)
        // END: checked_callback
        // The current native route has an unknown workspace bound, so the strict
        // tenant gets this traced refusal. Managed row storage is not that bound.
    }
    // END EXAMPLE: checked_callback
}
pub(crate) use tenant::{WorkspaceEvidenceRequired, for_tenant};

mod transports {
    // EXAMPLE: checked_file
    use zencodec::{
        CodecSet,
        encode::EncodeOutput,
        operation::{OperationPolicy, Outcome},
    };
    use zencodecs::checked_transcode::TranscodeRequest;

    pub fn from_file<P: OperationPolicy + ?Sized>(
        request: &TranscodeRequest<'_>,
        codecs: &CodecSet,
        path: &std::path::Path,
        policy: &mut P,
    ) -> Result<Outcome<EncodeOutput>, Box<dyn std::error::Error + Send + Sync>> {
        // ANCHOR: checked_file
        let mut source = zencodec_io::FileReadAt::open(path)?;
        let output = request.transcode_from_read_at_with(codecs, &mut source, policy)?;
        // END: checked_file
        // FileReadAt keeps positional reads below the codec; the request is unchanged.
        Ok(output)
    }
    // END EXAMPLE: checked_file
}
pub(crate) use transports::from_file;

mod network {
    // EXAMPLE: checked_async
    use zencodec::{
        At, CodecSet,
        encode::EncodeOutput,
        operation::{OperationPolicy, Outcome},
    };
    use zencodec_io::tokio_io::AsyncSource;
    use zencodecs::checked_transcode::Error;
    use zencodecs::checked_transcode::TranscodeRequest;

    pub async fn from_network<S: AsyncSource + ?Sized, P: OperationPolicy + ?Sized>(
        request: &TranscodeRequest<'_>,
        codecs: &CodecSet,
        source: &mut S,
        policy: &mut P,
    ) -> Result<Outcome<EncodeOutput>, At<Error>> {
        // ANCHOR: checked_async
        let output = request.transcode_async_with(codecs, source, policy).await?;
        // END: checked_async
        // Send the finished, checked bytes afterwards. Codec CPU work runs on
        // this task: use a suitable worker runtime for expensive configurations.
        Ok(output)
    }
    // END EXAMPLE: checked_async
}
pub(crate) use network::from_network;

mod cancellation {
    // EXAMPLE: checked_registry_pulse
    use zencodec::{CodecSet, ResourceLimits, SharedPulse};
    use zencodecs::{JpegDecoderConfig, PngDecoderConfig, PngEncoderConfig};

    pub fn gallery_codecs_with_cancellation(pulse: SharedPulse, max_pixels: u64) -> CodecSet {
        // ANCHOR: checked_registry_pulse
        CodecSet::new()
            .with_limits(ResourceLimits::for_untrusted_input().with_max_pixels(max_pixels))
            .with_pulse(pulse)
            .with_decoder(JpegDecoderConfig::new())
            .with_decoder(PngDecoderConfig::new())
            .with_send_encoder(PngEncoderConfig::new())
        // END: checked_registry_pulse
    }
    // Keep the application's signalling handle so it can cancel during work.
    // Passing SharedPulse preserves progress reporting and phase planning too.
    // Adapt a legacy Stop at the application boundary with StopOnly when needed.
    // END EXAMPLE: checked_registry_pulse
}
pub(crate) use cancellation::gallery_codecs_with_cancellation;

mod publication {
    // EXAMPLE: checked_png_file
    use std::path::Path;
    use zencodec::{
        CodecSet,
        encode::EncodeOutput,
        operation::{OperationPolicy, Outcome},
    };
    use zencodecs::checked_transcode::TranscodeRequest;

    pub fn publish_completed(
        codecs: &CodecSet,
        output: &EncodeOutput,
        destination: &Path,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Cancellation observed here must not create or truncate the file.
        codecs.pulse().check()?;
        std::fs::write(destination, output.data())?;
        Ok(())
    }

    pub fn transcode_to_file<P: OperationPolicy + ?Sized>(
        request: &TranscodeRequest<'_>,
        codecs: &CodecSet,
        bytes: &[u8],
        destination: &Path,
        policy: &mut P,
    ) -> Result<Outcome<EncodeOutput>, Box<dyn std::error::Error + Send + Sync>> {
        // ANCHOR: checked_png_file
        codecs.pulse().check()?;
        let output = request.transcode_with(codecs, bytes, policy)?;
        publish_completed(codecs, output.output(), destination)?;
        // END: checked_png_file
        // Pass the PNG request from the preceding recipes, along with its
        // mandatory PolicySet or BoundPolicy. No policy is replaced or relaxed.
        // The successful report remains available to the application.
        // fs::write is synchronous: cancellation during that call is not
        // preemptive, and an I/O failure may leave partial destination bytes.
        Ok(output)
    }
    // END EXAMPLE: checked_png_file
}
pub(crate) use publication::{publish_completed, transcode_to_file};
