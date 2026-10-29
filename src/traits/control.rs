//! Mandatory suspension, separate from ignorable decode notifications.

use super::{
    ErasedError, Fused, IncrementalDecode, IncrementalDecoderShim, Input, Progress, StepError,
};
use crate::DecodeRowSink;
use alloc::boxed::Box;
use core::{
    fmt,
    sync::atomic::{AtomicUsize, Ordering},
};

use zenpixels::PixelDescriptor;

/// How much a decoder knows about the interpretation of its proposed pixels.
/// This does not describe metadata inventory or certify absence of private data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum InterpretationState {
    /// The information has not arrived yet.
    Pending,
    /// A later fact may still change the interpretation.
    Provisional,
    /// The decoder has established the interpretation for this image.
    Final,
    /// The decoder cannot establish the interpretation.
    Unavailable,
}

/// Why a decoder cannot establish the interpretation of its proposed output.
/// These codes carry no profile contents, names, paths or other input payloads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum InterpretationUnavailable {
    /// Relevant color metadata arrived outside this route's qualified scope.
    LateColorMetadata,
    /// The request forbids extracting metadata needed to interpret the samples.
    ForbiddenColorMetadata,
    /// A recognized color carrier is incomplete or malformed.
    IncompleteColorMetadata,
    /// This route cannot establish the source's color interpretation.
    UnsupportedColor,
    /// The selected sample path has not been qualified for this contract.
    UnqualifiedOutput,
}

/// Facts exposed before affected decode output. Contains no metadata payloads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct InterpretationFacts {
    /// Proposed image width.
    pub width: u32,
    /// Proposed image height.
    pub height: u32,
    /// Proposed output layout and sample interpretation, when known.
    pub descriptor: Option<PixelDescriptor>,
    /// Whether this interpretation can still change.
    pub state: InterpretationState,
    /// Structured explanation when interpretation is unavailable. Older adapters
    /// may leave this absent; absence never means the route is permitted.
    pub unavailable: Option<InterpretationUnavailable>,
}
impl InterpretationFacts {
    /// Describe the interpretation of one still image. Final facts require a
    /// descriptor; the controlled decoder rejects a missing one.
    pub const fn new(
        width: u32,
        height: u32,
        descriptor: Option<PixelDescriptor>,
        state: InterpretationState,
    ) -> Self {
        Self {
            width,
            height,
            descriptor,
            state,
            unavailable: None,
        }
    }

    /// Mark these facts unavailable and retain a payload-free explanation.
    pub const fn with_unavailable(mut self, reason: InterpretationUnavailable) -> Self {
        self.state = InterpretationState::Unavailable;
        self.unavailable = Some(reason);
        self
    }
}

/// Identity of one suspension. A reply cannot authorize another suspension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoundaryId(u64);

/// A mandatory suspension before affected conversion or output effects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecodeBoundary {
    id: BoundaryId,
    facts: InterpretationFacts,
}
impl DecodeBoundary {
    /// Issue a fresh suspension. Codec authors use this before returning it;
    /// adapters cannot invent a matching identity for another boundary.
    pub fn new(facts: InterpretationFacts) -> Result<Self, ControlError> {
        static NEXT: AtomicUsize = AtomicUsize::new(1);
        let mut current = NEXT.load(Ordering::Relaxed);
        let id = loop {
            let next = current
                .checked_add(1)
                .ok_or(ControlError::IdentityExhausted)?;
            match NEXT.compare_exchange_weak(current, next, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(id) => break id as u64,
                Err(actual) => current = actual,
            }
        };
        Ok(Self {
            id: BoundaryId(id),
            facts,
        })
    }
    /// Identity required when resuming this exact suspension.
    pub const fn id(self) -> BoundaryId {
        self.id
    }
    /// Qualified facts supplied by the decoder.
    pub const fn facts(self) -> InterpretationFacts {
        self.facts
    }
}

/// Mechanisms a controlled still decoder can execute after suspension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum BoundaryAction {
    /// Continue discovering facts. Affected output remains forbidden. Any
    /// retained data still needs storage admission by its owning adapter.
    ContinueReading,
    /// Adopt final interpretation before starting affected output.
    AdoptInterpretation,
}
impl BoundaryAction {
    /// Whether the facts permit this mechanism. This does not admit storage or
    /// authorize a conversion/encode route on behalf of an application.
    pub const fn permits(self, facts: InterpretationFacts) -> bool {
        match self {
            Self::ContinueReading => matches!(
                facts.state,
                InterpretationState::Pending | InterpretationState::Provisional
            ),
            Self::AdoptInterpretation => {
                matches!(facts.state, InterpretationState::Final)
                    && facts.descriptor.is_some()
                    && facts.unavailable.is_none()
            }
        }
    }
}

/// A control-protocol failure; terminal like every other step error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ControlError {
    /// A legacy driver reached a mandatory suspension it cannot handle.
    UnhandledBoundary,
    /// This decoder was not constructed with controlled execution enabled.
    NotControlled,
    /// No matching pending boundary exists, or this action is not permitted.
    InvalidReply,
    /// Output was attempted before adoption of final interpretation.
    OutputBeforeInterpretation,
    /// Facts changed after final interpretation was adopted for this image.
    FinalInterpretationChanged,
    /// The facts do not agree with the image information or proposed output.
    InconsistentFacts,
    /// The finite process-wide identity space was exhausted.
    IdentityExhausted,
}
impl fmt::Display for ControlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "decode control: {self:?}")
    }
}
impl core::error::Error for ControlError {}

/// The byte progress of a controlled step. A suspension is never an ignorable
/// metadata event and never a request for a byte range.
#[derive(Clone, Debug)]
pub enum ControlledProgress {
    /// Ordinary byte, row or informational progress.
    Decode(Progress),
    /// The decoder is suspended. Repeated steps return this same boundary with
    /// zero consumed bytes and no sink effects until a matching reply arrives.
    Suspended {
        /// Bytes consumed before suspending.
        consumed: usize,
        /// Facts and identity of the outstanding decision.
        boundary: DecodeBoundary,
    },
}
impl ControlledProgress {
    /// Bytes consumed from the offered input in this step.
    pub const fn consumed(&self) -> usize {
        match self {
            Self::Decode(p) => p.consumed,
            Self::Suspended { consumed, .. } => *consumed,
        }
    }
}
impl From<Progress> for ControlledProgress {
    fn from(progress: Progress) -> Self {
        Self::Decode(progress)
    }
}

/// Optional controlled protocol for decoders that can suspend before effects.
///
/// The legacy [`IncrementalDecode::step`] must refuse a mandatory boundary,
/// never acknowledge it automatically. A controlled still image starts with
/// output forbidden. `ContinueReading` permits discovery only; final facts must
/// be adopted before affected conversion or sink effects. Changing final
/// interpretation requires a new image/session, not reinterpretation of already
/// returned rows. Video epochs are a separate media contract.
///
/// The application policy belongs in the adapter. This trait exposes mechanisms
/// and facts; it never calls an application resolver from codec internals.
pub trait ControlledIncrementalDecode: IncrementalDecode {
    /// Advance or return a mandatory suspension.
    /// `None` adds no per-call control; retained job controls still apply.
    fn step_controlled(
        &mut self,
        input: Input<'_>,
        sink: &mut dyn DecodeRowSink,
        pulse: Option<&dyn crate::Pulse>,
    ) -> Result<ControlledProgress, StepError<Self::Error>>;
    /// Resume exactly one outstanding boundary. A stale, invalid or cancelled
    /// reply is terminal. This performs no sink work. `None` preserves the
    /// retained control and does not bypass cancellation at the boundary.
    fn resume(
        &mut self,
        boundary: BoundaryId,
        action: BoundaryAction,
        pulse: Option<&dyn crate::Pulse>,
    ) -> Result<(), StepError<Self::Error>>;
}

/// Erased controlled decoder, retaining native error causes.
pub type DynControlledDecoder = dyn ControlledIncrementalDecode<Error = ErasedError> + Send;

impl<D: ControlledIncrementalDecode + ?Sized> ControlledIncrementalDecode for Box<D> {
    fn step_controlled(
        &mut self,
        input: Input<'_>,
        sink: &mut dyn DecodeRowSink,
        pulse: Option<&dyn crate::Pulse>,
    ) -> Result<ControlledProgress, StepError<Self::Error>> {
        (**self).step_controlled(input, sink, pulse)
    }
    fn resume(
        &mut self,
        boundary: BoundaryId,
        action: BoundaryAction,
        pulse: Option<&dyn crate::Pulse>,
    ) -> Result<(), StepError<Self::Error>> {
        (**self).resume(boundary, action, pulse)
    }
}
impl<D: ControlledIncrementalDecode + ?Sized> ControlledIncrementalDecode for &mut D {
    fn step_controlled(
        &mut self,
        input: Input<'_>,
        sink: &mut dyn DecodeRowSink,
        pulse: Option<&dyn crate::Pulse>,
    ) -> Result<ControlledProgress, StepError<Self::Error>> {
        (**self).step_controlled(input, sink, pulse)
    }
    fn resume(
        &mut self,
        boundary: BoundaryId,
        action: BoundaryAction,
        pulse: Option<&dyn crate::Pulse>,
    ) -> Result<(), StepError<Self::Error>> {
        (**self).resume(boundary, action, pulse)
    }
}
impl<D: ControlledIncrementalDecode> ControlledIncrementalDecode for IncrementalDecoderShim<D> {
    fn step_controlled(
        &mut self,
        input: Input<'_>,
        sink: &mut dyn DecodeRowSink,
        pulse: Option<&dyn crate::Pulse>,
    ) -> Result<ControlledProgress, StepError<ErasedError>> {
        self.0
            .step_controlled(input, sink, pulse)
            .map_err(|e| e.map_codec(|c| ErasedError::new(Box::new(c))))
    }
    fn resume(
        &mut self,
        boundary: BoundaryId,
        action: BoundaryAction,
        pulse: Option<&dyn crate::Pulse>,
    ) -> Result<(), StepError<ErasedError>> {
        self.0
            .resume(boundary, action, pulse)
            .map_err(|e| e.map_codec(|c| ErasedError::new(Box::new(c))))
    }
}

/// Erase a controlled decoder and enforce terminal errors and cancellation.
pub fn erase_controlled<D: ControlledIncrementalDecode + Send + 'static>(
    decoder: D,
) -> Box<DynControlledDecoder> {
    Box::new(IncrementalDecoderShim(Fused::controlled(decoder)))
}

/// No sink effects before adoption, even if a faulty codec ignores a refusal.
pub(super) struct GuardedSink<'a> {
    inner: &'a mut dyn DecodeRowSink,
    facts: Option<InterpretationFacts>,
    pub(super) refused: Option<ControlError>,
}
impl<'a> GuardedSink<'a> {
    pub(super) fn new(
        inner: &'a mut dyn DecodeRowSink,
        facts: Option<InterpretationFacts>,
    ) -> Self {
        Self {
            inner,
            facts,
            refused: None,
        }
    }
    fn check(
        &mut self,
        width: Option<u32>,
        extent: Option<(u32, u32)>,
        descriptor: Option<PixelDescriptor>,
    ) -> Result<(), crate::sink::SinkError> {
        let error = match self.facts {
            None => Some(ControlError::OutputBeforeInterpretation),
            Some(facts)
                if width.is_some_and(|w| w != facts.width)
                    || descriptor.is_some_and(|d| facts.descriptor != Some(d))
                    || extent.is_some_and(|(y, rows)| {
                        y.checked_add(rows).is_none_or(|end| end > facts.height)
                    }) =>
            {
                Some(ControlError::InconsistentFacts)
            }
            _ => None,
        };
        if let Some(error) = self.refused.or(error) {
            self.refused = Some(error);
            return Err(Box::new(error));
        }
        Ok(())
    }
}
impl DecodeRowSink for GuardedSink<'_> {
    fn max_buffer_rows(&self) -> core::num::NonZeroU32 {
        self.inner.max_buffer_rows()
    }

    fn begin(
        &mut self,
        width: u32,
        height: u32,
        descriptor: PixelDescriptor,
    ) -> Result<(), crate::sink::SinkError> {
        self.check(Some(width), Some((0, height)), Some(descriptor))?;
        if self.facts.is_some_and(|facts| facts.height != height) {
            self.refused = Some(ControlError::InconsistentFacts);
            return Err(Box::new(ControlError::InconsistentFacts));
        }
        self.inner.begin(width, height, descriptor)
    }
    fn provide_next_buffer(
        &mut self,
        y: u32,
        height: u32,
        width: u32,
        descriptor: PixelDescriptor,
    ) -> Result<zenpixels::PixelSliceMut<'_>, crate::sink::SinkError> {
        self.check(Some(width), Some((y, height)), Some(descriptor))?;
        self.inner.provide_next_buffer(y, height, width, descriptor)
    }
    fn provide_pass_buffer(
        &mut self,
        meta: super::BatchMeta,
        height: u32,
        width: u32,
        descriptor: PixelDescriptor,
    ) -> Result<zenpixels::PixelSliceMut<'_>, crate::sink::SinkError> {
        self.check(Some(width), Some((meta.y, height)), Some(descriptor))?;
        self.inner
            .provide_pass_buffer(meta, height, width, descriptor)
    }
    fn accepts_provisional(&self) -> bool {
        self.facts.is_some() && self.refused.is_none() && self.inner.accepts_provisional()
    }
    fn finish(&mut self) -> Result<(), crate::sink::SinkError> {
        self.check(None, None, None)?;
        self.inner.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ImageInfo;
    use crate::decode::{Event, InputRequest, OutputInfo, SinkError};
    use alloc::sync::Arc;
    use zenpixels::PixelSliceMut;

    struct Hostile {
        calls: Arc<AtomicUsize>,
        boundary: DecodeBoundary,
        early_output: u8,
    }
    impl IncrementalDecode for Hostile {
        type Error = core::fmt::Error;
        fn step(
            &mut self,
            _: Input<'_>,
            sink: &mut dyn DecodeRowSink,
            _: Option<&dyn crate::Pulse>,
        ) -> Result<Progress, StepError<Self::Error>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            sink.begin(1, 1, PixelDescriptor::RGBA8_SRGB)
                .map_err(StepError::Sink)?;
            Ok(Progress::new(
                0,
                Event::Done(OutputInfo::full_decode(1, 1, PixelDescriptor::RGBA8_SRGB)),
            ))
        }
        fn requested(&self) -> &[InputRequest] {
            &[]
        }
        fn info(&self) -> Option<&ImageInfo> {
            None
        }
    }
    impl ControlledIncrementalDecode for Hostile {
        fn step_controlled(
            &mut self,
            _: Input<'_>,
            sink: &mut dyn DecodeRowSink,
            _: Option<&dyn crate::Pulse>,
        ) -> Result<ControlledProgress, StepError<Self::Error>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match self.early_output {
                1 => {
                    let _ = sink.begin(1, 1, PixelDescriptor::RGBA8_SRGB);
                }
                2 => {
                    let _ = sink.provide_next_buffer(0, 1, 1, PixelDescriptor::RGBA8_SRGB);
                }
                3 => {
                    let _ = sink.provide_pass_buffer(
                        super::super::BatchMeta::new(0, false),
                        1,
                        1,
                        PixelDescriptor::RGBA8_SRGB,
                    );
                }
                4 => {
                    let _ = sink.finish();
                }
                _ => {}
            }
            Ok(ControlledProgress::Suspended {
                consumed: 0,
                boundary: self.boundary,
            })
        }
        fn resume(
            &mut self,
            _: BoundaryId,
            _: BoundaryAction,
            _: Option<&dyn crate::Pulse>,
        ) -> Result<(), StepError<Self::Error>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(()) // Intentionally accepts every reply: the framework must check.
        }
    }
    struct NoEffects;
    impl DecodeRowSink for NoEffects {
        fn finish(&mut self) -> Result<(), SinkError> {
            panic!("unexpected finish")
        }
        fn provide_pass_buffer(
            &mut self,
            _: super::super::BatchMeta,
            _: u32,
            _: u32,
            _: PixelDescriptor,
        ) -> Result<PixelSliceMut<'_>, SinkError> {
            panic!("unexpected provisional rows")
        }
        fn begin(&mut self, _: u32, _: u32, _: PixelDescriptor) -> Result<(), SinkError> {
            panic!("unexpected sink effect")
        }
        fn provide_next_buffer(
            &mut self,
            _: u32,
            _: u32,
            _: u32,
            _: PixelDescriptor,
        ) -> Result<PixelSliceMut<'_>, SinkError> {
            panic!("unexpected rows")
        }
    }
    fn fixture() -> (Arc<AtomicUsize>, Fused<Hostile>, DecodeBoundary) {
        let calls = Arc::new(AtomicUsize::new(0));
        let boundary = DecodeBoundary::new(InterpretationFacts::new(
            1,
            1,
            Some(PixelDescriptor::RGBA8_SRGB),
            InterpretationState::Final,
        ))
        .unwrap();
        (
            calls.clone(),
            Fused::new(Hostile {
                calls,
                boundary,
                early_output: 0,
            }),
            boundary,
        )
    }
    #[test]
    fn fused_control_checks_replies_and_blocks_uncooperative_native_progress() {
        for invalid in [false, true] {
            let (calls, mut dec, boundary) = fixture();
            let mut sink = NoEffects;
            for _ in 0..3 {
                assert!(
                    matches!(dec.step_controlled(Input::empty(), &mut sink, Some(&crate::how_far::NoPulse)).unwrap(), ControlledProgress::Suspended { consumed: 0, boundary: b } if b == boundary)
                );
            }
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            if invalid {
                assert!(matches!(
                    dec.resume(
                        boundary.id(),
                        BoundaryAction::ContinueReading,
                        Some(&crate::how_far::NoPulse)
                    ),
                    Err(StepError::Control(ControlError::InvalidReply))
                ));
            } else {
                assert!(matches!(
                    dec.step(Input::empty(), &mut sink, Some(&crate::how_far::NoPulse)),
                    Err(StepError::Control(ControlError::UnhandledBoundary))
                ));
            }
            assert!(matches!(
                dec.step_controlled(Input::empty(), &mut sink, Some(&crate::how_far::NoPulse)),
                Err(StepError::Finished)
            ));
            assert_eq!(calls.load(Ordering::SeqCst), 1);
        }
        let (calls, mut dec, boundary) = fixture();
        dec.step_controlled(
            Input::empty(),
            &mut NoEffects,
            Some(&crate::how_far::NoPulse),
        )
        .unwrap();
        dec.resume(
            boundary.id(),
            BoundaryAction::AdoptInterpretation,
            Some(&crate::how_far::NoPulse),
        )
        .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
    #[test]
    fn controlled_wrapper_blocks_early_effects_even_when_native_swallows_the_error() {
        for effect in 1..=4 {
            let (calls, dec, _) = fixture();
            let mut inner = dec.into_inner();
            inner.early_output = effect;
            let mut dec = Fused::new(inner);
            assert!(matches!(
                dec.step_controlled(
                    Input::empty(),
                    &mut NoEffects,
                    Some(&crate::how_far::NoPulse)
                ),
                Err(StepError::Control(ControlError::OutputBeforeInterpretation))
            ));
            assert_eq!(calls.load(Ordering::SeqCst), 1);
        }
    }
}
