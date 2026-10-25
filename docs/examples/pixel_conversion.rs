//! Executed migration-guide examples; no published API.
#![forbid(unsafe_code)]

mod display {
    // EXAMPLE: convert_display
    use zencodec::{CodecSet, Orientation, ResourceLimits, decode::BoxedError};
    use zenjpeg::JpegDecoderConfig;
    use zenpixels::{PixelBuffer, PixelDescriptor};
    use zenpixels_convert::PixelBufferConvertExt;

    pub(crate) fn decode_for_display(
        bytes: &[u8],
    ) -> Result<(PixelBuffer, Orientation), BoxedError> {
        let codecs = CodecSet::new()
            .with_limits(ResourceLimits::for_untrusted_input().with_max_pixels(4_000_000))
            .with_decoder(JpegDecoderConfig::new());
        // ANCHOR: convert_display
        let decoded = codecs.decode(bytes)?;
        let orientation = decoded.info().orientation;
        let bgra = decoded
            .into_buffer()
            .convert_to(PixelDescriptor::BGRA8_SRGB)?;
        // END: convert_display
        // Storage conversion does not apply orientation. The display owns that step.
        // An unsupported color transform is an error, never a request to relabel.
        Ok((bgra, orientation))
    }
    // END EXAMPLE: convert_display
}
pub(crate) use display::decode_for_display;

mod framebuffer {
    // EXAMPLE: convert_framebuffer
    use zencodec::{
        ResourceLimits,
        decode::BoxedError,
        encode::{EncodeJob, EncoderConfig},
    };
    use zenjpeg::JpegEncoderConfig;
    use zenpixels::{AlphaPolicy, ConvertOptions, PixelBuffer, PixelDescriptor, PixelSlice};
    use zenpixels_convert::{RowConverter, adapt::check_opaque};

    #[derive(Debug)]
    pub(crate) struct ExpectedSrgbFramebuffer;
    impl std::fmt::Display for ExpectedSrgbFramebuffer {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("expected caller-rendered BGRA8 sRGB without an attached color profile")
        }
    }
    impl std::error::Error for ExpectedSrgbFramebuffer {}

    pub(crate) fn rgb_for_jpeg(pixels: PixelSlice<'_>) -> Result<PixelBuffer, BoxedError> {
        // This adapter's input is an already rendered framebuffer, not arbitrary
        // decoded ICC/HDR/unknown-color pixels. Those need a color-aware path.
        if pixels.descriptor() != PixelDescriptor::BGRA8_SRGB || pixels.color_context().is_some() {
            return Err(ExpectedSrgbFramebuffer.into());
        }
        ResourceLimits::for_untrusted_input()
            .with_max_pixels(4_000_000)
            .check_dimensions(pixels.width(), pixels.rows())?;
        // ANCHOR: convert_framebuffer
        check_opaque(pixels.clone())?; // Verify every row before authorizing alpha removal.
        let options =
            ConvertOptions::forbid_lossy().with_alpha_policy(AlphaPolicy::DiscardUnchecked); // Proven above.
        let mut converter =
            RowConverter::new_explicit(pixels.descriptor(), PixelDescriptor::RGB8_SRGB, &options)?;
        converter.prepare(pixels.width())?;
        let mut rgb =
            PixelBuffer::try_new(pixels.width(), pixels.rows(), PixelDescriptor::RGB8_SRGB)?;
        {
            let mut destination = rgb.as_slice_mut();
            for y in 0..pixels.rows() {
                converter.try_convert_row(pixels.row(y), destination.row_mut(y), pixels.width())?;
            }
        }
        // END: convert_framebuffer
        Ok(rgb)
    }

    pub(crate) fn encode_framebuffer(pixels: PixelSlice<'_>) -> Result<Vec<u8>, BoxedError> {
        let rgb = rgb_for_jpeg(pixels)?;
        // Conversion preserves these RGB samples. JPEG encoding is intentionally lossy.
        let output = JpegEncoderConfig::new()
            .with_generic_quality(85.0)
            .job()
            .with_limits(ResourceLimits::for_untrusted_input())
            .encode(rgb.as_slice())?;
        Ok(output.into_vec())
    }
    // END EXAMPLE: convert_framebuffer
}
pub(crate) use framebuffer::{ExpectedSrgbFramebuffer, encode_framebuffer, rgb_for_jpeg};
