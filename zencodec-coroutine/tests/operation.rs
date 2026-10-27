//! Authored registration control: proves the request/erasure/driver boundary.
//! Native codec qualification remains in the separate JPEG harness.
use std::{
    borrow::Cow,
    convert::Infallible,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use zencodec::{
    At, CodecSet, ImageFormat, ImageFormatDefinition, ImageInfo, ResourceLimits, SharedPulse,
    UnsupportedOperation,
    decode::*,
    operation::{Decision, DecodeRequest, OperationError, PolicySet, Question},
    source::{AccessPattern, SliceReadAt},
    storage::StorageBudget,
};
use zencodec_coroutine::{CoroutineDecoder, CoroutineError};
use zencodec_io::{CollectSink, DriveOptions, PolicyDecoder, drive_from_read_at, drive_from_slice};
use zenpixels::{PixelDescriptor, PixelSlice};

const RGBA: PixelDescriptor = PixelDescriptor::RGBA8_SRGB;
static FORMAT: ImageFormatDefinition = ImageFormatDefinition::new(
    "controlled-fixture",
    None,
    "Controlled protocol fixture",
    "ctl",
    &["ctl"],
    "application/x-controlled-fixture",
    &["application/x-controlled-fixture"],
    true,
    false,
    true,
    false,
    4,
    |bytes| bytes.starts_with(b"CTL1"),
);
const IMAGE: ImageFormat = ImageFormat::Custom(&FORMAT);

#[derive(Clone)]
struct Config {
    work: Arc<AtomicUsize>,
    limits: ResourceLimits,
}
impl DecoderConfig for Config {
    type Error = CoroutineError;
    type Job<'a> = Self;
    fn formats() -> &'static [ImageFormat] {
        &[IMAGE]
    }
    fn supported_descriptors() -> &'static [PixelDescriptor] {
        &[RGBA]
    }
    fn job<'a>(self) -> Self::Job<'a> {
        self
    }
}
enum Never {}
impl Decode for Never {
    type Error = CoroutineError;
    fn decode(self) -> Result<DecodeOutput, Self::Error> {
        match self {}
    }
}
impl<'a> DecodeJob<'a> for Config {
    type Error = CoroutineError;
    type Dec = Never;
    type StreamDec = zencodec::Unsupported<CoroutineError>;
    type AnimationFrameDec = zencodec::Unsupported<CoroutineError>;
    fn with_pulse(self, _: SharedPulse) -> Self {
        self
    }
    fn with_limits(mut self, limits: ResourceLimits) -> Self {
        self.limits = limits;
        self
    }
    fn with_policy(self, _: DecodePolicy) -> Self {
        panic!("checked path must not call the best-effort setter")
    }
    fn probe(&self, _: &[u8]) -> Result<ImageInfo, Self::Error> {
        Ok(ImageInfo::new(2, 2, IMAGE))
    }
    fn output_info(&self, _: &[u8]) -> Result<OutputInfo, Self::Error> {
        Ok(OutputInfo::full_decode(2, 2, RGBA))
    }
    fn decoder(self, _: Cow<'a, [u8]>, _: &[PixelDescriptor]) -> Result<Self::Dec, Self::Error> {
        panic!("no silent one-shot fallback")
    }
    fn push_decoder(
        self,
        _: Cow<'a, [u8]>,
        _: &mut dyn DecodeRowSink,
        _: &[PixelDescriptor],
    ) -> Result<OutputInfo, Self::Error> {
        panic!("no silent push fallback")
    }
    fn streaming_decoder(
        self,
        _: Cow<'a, [u8]>,
        _: &[PixelDescriptor],
    ) -> Result<Self::StreamDec, Self::Error> {
        panic!("no silent streaming fallback")
    }
    fn animation_frame_decoder(
        self,
        _: Cow<'a, [u8]>,
        _: &[PixelDescriptor],
    ) -> Result<Self::AnimationFrameDec, Self::Error> {
        panic!("no animation fallback")
    }
    fn dyn_controlled_decoder(
        self,
        _: &[PixelDescriptor],
        storage: StorageBudget,
    ) -> Result<Box<DynControlledDecoder>, BoxedError> {
        self.limits.check_dimensions(2, 2)?;
        Ok(erase_controlled(CoroutineDecoder::controlled_with_storage(
            32,
            AccessPattern::RandomAccess,
            storage,
            move |mut ctx| async move {
                let mut magic = [0; 4];
                ctx.read_exact_at(0, &mut magic).await?;
                assert_eq!(magic, *b"CTL1");
                ctx.set_info(ImageInfo::new(2, 2, IMAGE));
                ctx.set_output_descriptor(RGBA);
                ctx.interpretation(InterpretationFacts::new(
                    2,
                    2,
                    Some(RGBA),
                    InterpretationState::Final,
                ))
                .await?;
                self.work.fetch_add(1, Ordering::SeqCst);
                let mut pixels = [0; 16];
                ctx.read_exact_at(4, &mut pixels).await?;
                ctx.write_rows(
                    BatchMeta::final_rows(0),
                    &PixelSlice::new(&pixels, 2, 2, 8, RGBA).unwrap(),
                )
                .await?;
                Ok::<_, CoroutineError>(OutputInfo::full_decode(2, 2, RGBA))
            },
        )?))
    }
}
fn codecs(work: Arc<AtomicUsize>) -> CodecSet {
    CodecSet::new().with_decoder(Config {
        work,
        limits: ResourceLimits::none(),
    })
}
fn budget() -> StorageBudget {
    StorageBudget::new(4096, 3)
}
fn input() -> Vec<u8> {
    [b"CTL1".as_slice(), &(0..16).collect::<Vec<u8>>()].concat()
}

#[test]
fn stored_request_detects_and_executes_through_slice_and_positional_drivers() {
    let work = Arc::new(AtomicUsize::new(0));
    let codecs = codecs(work.clone());
    let request = DecodeRequest::new().with_output(RGBA);
    for read_at in [false, true] {
        let storage = budget();
        let mut calls = 0;
        let before = work.load(Ordering::SeqCst);
        let mut resolver = |q: Question<'_>| -> Result<Decision, At<Infallible>> {
            let Question::Boundary { facts, .. } = q else {
                panic!()
            };
            assert_eq!(facts.descriptor, Some(RGBA));
            assert_eq!(work.load(Ordering::SeqCst), before);
            calls += 1;
            Ok(Decision::UsePreset)
        };
        let mut decoder = PolicyDecoder::new(
            request.start_controlled(&codecs, storage.clone()).unwrap(),
            &mut resolver,
        );
        let mut sink = CollectSink::new();
        let bytes = input();
        if read_at {
            drive_from_read_at(
                &mut decoder,
                &mut SliceReadAt::new(&bytes),
                &mut sink,
                None,
                DriveOptions::new(0, 3),
            )
            .unwrap();
        } else {
            drive_from_slice(&mut decoder, &bytes, &mut sink, None, 1).unwrap();
        }
        assert_eq!(sink.bytes(), &bytes[4..]);
        assert_eq!(work.load(Ordering::SeqCst), before + 1);
        assert_eq!(decoder.report().decisions().count(), 1);
        assert_eq!(storage.reserved_bytes(), 0);
        drop(decoder);
        assert_eq!(calls, 1);
    }
}

#[test]
fn required_descriptor_refuses_before_policy_adoption_and_pixel_work() {
    let work = Arc::new(AtomicUsize::new(0));
    let codecs = codecs(work.clone());
    let request = DecodeRequest::new().with_output(PixelDescriptor::RGB8_SRGB);
    let mut resolver = |_q: Question<'_>| -> Result<Decision, At<Infallible>> {
        panic!("an unmet requirement is not a policy choice")
    };
    let mut decoder = PolicyDecoder::new(
        request.start_controlled(&codecs, budget()).unwrap(),
        &mut resolver,
    );
    let mut sink = CollectSink::new();
    let error = drive_from_slice(&mut decoder, &input(), &mut sink, None, 7).unwrap_err();
    assert!(matches!(
        zencodec::find_cause::<OperationError>(&error),
        Some(OperationError::OutputDescriptor { .. })
    ));
    assert!(sink.as_pixel_slice().is_none());
    assert_eq!(work.load(Ordering::SeqCst), 0);
    let preferred = DecodeRequest::new().preferring_output(vec![PixelDescriptor::RGB8_SRGB]);
    let mut decoder = PolicyDecoder::new(
        preferred.start_controlled(&codecs, budget()).unwrap(),
        PolicySet::new(),
    );
    drive_from_slice(&mut decoder, &input(), &mut sink, None, 7).unwrap();
    assert_eq!(sink.as_pixel_slice().unwrap().descriptor(), RGBA);
}

#[test]
fn known_format_checks_inherited_policy_and_limits_without_reading_input() {
    let codecs = codecs(Arc::new(AtomicUsize::new(0))).with_decode_policy(DecodePolicy::strict());
    let request = DecodeRequest::new().with_format(IMAGE);
    let error = match request.start_controlled(&codecs, budget()) {
        Err(e) => e,
        Ok(_) => panic!("unchecked policy"),
    };
    assert_eq!(
        zencodec::find_cause::<UnsupportedOperation>(&error),
        Some(&UnsupportedOperation::DecodePolicy)
    );
    let request = request.with_policy(DecodePolicy::none());
    let codecs = codecs.with_limits(ResourceLimits::none().with_max_pixels(3));
    let error = match request.start_controlled(&codecs, budget()) {
        Err(e) => e,
        Ok(_) => panic!("lost limits"),
    };
    assert!(zencodec::find_cause::<zencodec::LimitExceeded>(&error).is_some());
}
