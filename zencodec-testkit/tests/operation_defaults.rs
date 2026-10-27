//! Adapter defaults must not erase explicit requirements during preparation.
use std::sync::{Arc, Mutex};
use zencodec::{
    CodecSet, ColorEmitPolicy, ImageFormat, Metadata, ResourceLimits, SharedPulse,
    UnsupportedOperation,
    encode::{EncodeJob, EncodePolicy, EncoderConfig, Fidelity},
    operation::EncodeRequest,
};
use zencodec_testkit::{
    ReferenceEncoderConfig,
    reference::{RefEnc, RefEncodeJob, RefError},
};
use zenpixels::PixelDescriptor;

#[derive(Clone)]
struct Recorder(Arc<Mutex<Vec<EncodePolicy>>>);
struct Job {
    seen: Arc<Mutex<Vec<EncodePolicy>>>,
    inner: RefEncodeJob,
}
impl EncoderConfig for Recorder {
    type Error = RefError;
    type Job = Job;
    fn format() -> ImageFormat {
        ImageFormat::Pnm
    }
    fn supported_descriptors() -> &'static [PixelDescriptor] {
        &[PixelDescriptor::RGB8_SRGB]
    }
    fn with_fidelity(self, _: Fidelity) -> Self {
        self
    }
    fn job(self) -> Job {
        Job {
            seen: self.0,
            inner: ReferenceEncoderConfig::new().job(),
        }
    }
}
impl EncodeJob for Job {
    type Error = RefError;
    type Enc = RefEnc;
    type AnimationFrameEnc = ();
    fn with_pulse(mut self, stop: SharedPulse) -> Self {
        self.inner = self.inner.with_pulse(stop);
        self
    }
    fn with_limits(mut self, limits: ResourceLimits) -> Self {
        self.inner = self.inner.with_limits(limits);
        self
    }
    #[allow(deprecated)]
    fn with_metadata(mut self, metadata: Metadata) -> Self {
        self.inner = self.inner.with_metadata(metadata);
        self
    }
    fn try_set_policy(&mut self, policy: EncodePolicy) -> Result<(), UnsupportedOperation> {
        self.seen.lock().unwrap().push(policy);
        Ok(())
    }
    fn encoder(self) -> Result<RefEnc, RefError> {
        self.inner.encoder()
    }
    fn animation_frame_encoder(self) -> Result<(), RefError> {
        Err(RefError::Unsupported(UnsupportedOperation::AnimationEncode))
    }
}

#[test]
fn required_fields_win_for_local_and_transferable_jobs() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let inherited = EncodePolicy::none().with_embed_icc(false).with_strict(true);
    let defaults = EncodePolicy::preserve_all()
        .with_color(ColorEmitPolicy::Verbatim)
        .with_strict(false);
    let set = CodecSet::new()
        .with_send_encoder(Recorder(seen.clone()))
        .with_encode_policy(inherited);
    let request = EncodeRequest::new(ImageFormat::Pnm).with_policy_defaults(defaults);
    request.prepare(&set).unwrap();
    request.prepare_send(&set.clone()).unwrap();
    let expected = EncodePolicy::preserve_all()
        .with_embed_icc(false)
        .with_color(ColorEmitPolicy::Verbatim)
        .with_strict(true);
    assert_eq!(*seen.lock().unwrap(), vec![expected, expected]);

    // The explicit request replaces the inherited policy, then fills holes.
    let explicit = EncodePolicy::none()
        .with_color(ColorEmitPolicy::Compact)
        .with_embed_exif(false);
    let request = request.with_policy(explicit);
    request.prepare(&set).unwrap();
    request.prepare_send(&set).unwrap();
    let expected = EncodePolicy::preserve_all()
        .with_color(ColorEmitPolicy::Compact)
        .with_embed_exif(false)
        .with_strict(false);
    assert_eq!(&seen.lock().unwrap()[2..], &[expected, expected]);
}

#[test]
fn absent_defaults_do_not_touch_a_backend_template() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let set = CodecSet::new().with_send_encoder(Recorder(seen.clone()));
    let request = EncodeRequest::new(ImageFormat::Pnm).with_policy_defaults(EncodePolicy::none());
    request.prepare(&set).unwrap();
    request.prepare_send(&set).unwrap();
    assert!(seen.lock().unwrap().is_empty());
}

#[test]
fn unsupported_adapter_defaults_refuse_both_paths() {
    let set = CodecSet::new().with_send_encoder(ReferenceEncoderConfig::new());
    let request = EncodeRequest::new(ImageFormat::Pnm)
        .with_policy_defaults(EncodePolicy::none().with_color(ColorEmitPolicy::Verbatim));
    let errors = [
        request.prepare(&set).unwrap_err(),
        request.prepare_send(&set).unwrap_err(),
    ];
    for error in errors {
        assert_eq!(
            zencodec::find_cause::<UnsupportedOperation>(&error),
            Some(&UnsupportedOperation::EncodePolicy)
        );
    }
}
