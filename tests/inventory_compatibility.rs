//! A published-style implementor with a native error that cannot construct an
//! UnsupportedOperation. Adding inventory must not impose a new trait bound.
use std::borrow::Cow;
use zencodec::decode::{
    AnimationFrameDecoder, Decode, DecodeJob, DecodeOutput, DecodeRowSink, DecoderConfig,
    DynDecoderConfig, OutputInfo, SinkError, StreamingDecode,
};
use zencodec::{
    AnimationFrame, CodecErrorExt, ErrorCategory, ImageFormat, ImageInfo, RequestError,
    ResourceLimits, SharedPulse, UnsupportedOperation,
};
use zenpixels::{PixelDescriptor, PixelSlice};

#[derive(Debug)]
struct NativeError;
impl std::fmt::Display for NativeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("native decoder error")
    }
}
impl std::error::Error for NativeError {}

struct LegacyDecoder(ImageInfo);
impl Decode for LegacyDecoder {
    type Error = NativeError;
    fn decode(self) -> Result<DecodeOutput, NativeError> {
        Err(NativeError)
    }
}
impl StreamingDecode for LegacyDecoder {
    type Error = NativeError;
    fn next_batch(&mut self) -> Result<Option<(u32, PixelSlice<'_>)>, NativeError> {
        Err(NativeError)
    }
    fn info(&self) -> &ImageInfo {
        &self.0
    }
}
impl AnimationFrameDecoder for LegacyDecoder {
    type Error = NativeError;
    fn wrap_sink_error(_: SinkError) -> NativeError {
        NativeError
    }
    fn render_next_frame_to_sink(
        &mut self,
        _: Option<&dyn zencodec::Pulse>,
        _: &mut dyn DecodeRowSink,
    ) -> Result<Option<OutputInfo>, NativeError> {
        Err(NativeError)
    }
    fn info(&self) -> &ImageInfo {
        &self.0
    }
    fn render_next_frame(
        &mut self,
        _: Option<&dyn zencodec::Pulse>,
    ) -> Result<Option<AnimationFrame<'_>>, NativeError> {
        Err(NativeError)
    }
}

#[derive(Clone)]
struct LegacyConfig;
impl DecoderConfig for LegacyConfig {
    type Error = NativeError;
    type Job<'a> = LegacyJob;
    fn formats() -> &'static [ImageFormat] {
        &[ImageFormat::Png]
    }
    fn supported_descriptors() -> &'static [PixelDescriptor] {
        &[]
    }
    fn job<'a>(self) -> Self::Job<'a> {
        LegacyJob
    }
}
struct LegacyJob;
impl<'a> DecodeJob<'a> for LegacyJob {
    type Error = NativeError;
    type Dec = LegacyDecoder;
    type StreamDec = LegacyDecoder;
    type AnimationFrameDec = LegacyDecoder;
    fn with_pulse(self, _: SharedPulse) -> Self {
        self
    }
    fn with_limits(self, _: ResourceLimits) -> Self {
        self
    }
    fn probe(&self, _: &[u8]) -> Result<ImageInfo, NativeError> {
        Err(NativeError)
    }
    fn output_info(&self, _: &[u8]) -> Result<OutputInfo, NativeError> {
        Err(NativeError)
    }
    fn decoder(
        self,
        _: Cow<'a, [u8]>,
        _: &[PixelDescriptor],
    ) -> Result<LegacyDecoder, NativeError> {
        Err(NativeError)
    }
    fn push_decoder(
        self,
        _: Cow<'a, [u8]>,
        _: &mut dyn DecodeRowSink,
        _: &[PixelDescriptor],
    ) -> Result<OutputInfo, NativeError> {
        Err(NativeError)
    }
    fn streaming_decoder(
        self,
        _: Cow<'a, [u8]>,
        _: &[PixelDescriptor],
    ) -> Result<LegacyDecoder, NativeError> {
        Err(NativeError)
    }
    fn animation_frame_decoder(
        self,
        _: Cow<'a, [u8]>,
        _: &[PixelDescriptor],
    ) -> Result<LegacyDecoder, NativeError> {
        Err(NativeError)
    }
    // Intentionally no inventory override, error conversion or error taxonomy impl.
}

#[test]
fn old_implementor_inherits_recoverable_unsupported_inventory() {
    assert!(!<LegacyConfig as DecoderConfig>::capabilities().inventory());
    let expected = Some(ErrorCategory::Request(RequestError::Unsupported(
        UnsupportedOperation::Inventory,
    )));
    for bytes in [b"".as_slice(), b"not a PNG"] {
        let generic = LegacyConfig.job().inventory(bytes).unwrap_err();
        assert_eq!(generic.error_category(), expected);
        assert_eq!(
            generic.unsupported_operation(),
            Some(&UnsupportedOperation::Inventory)
        );
        let config: &dyn DynDecoderConfig = &LegacyConfig;
        let erased = config.dyn_job().inventory(bytes).unwrap_err();
        assert_eq!(erased.error_category(), expected);
        assert_eq!(
            erased.unsupported_operation(),
            Some(&UnsupportedOperation::Inventory)
        );
    }
}
