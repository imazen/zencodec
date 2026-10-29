//! Completion facts must agree with the interpretation already adopted.
use zencodec::{ImageInfo, decode::*, how_far::NoPulse, operation::PolicySet};
use zencodec_io::{CollectSink, PolicyDecoder};
use zenpixels::PixelDescriptor;

struct ChangedCompletion {
    boundary: DecodeBoundary,
    adopted: bool,
    done: OutputInfo,
}
impl IncrementalDecode for ChangedCompletion {
    type Error = core::fmt::Error;
    fn step(
        &mut self,
        _: Input<'_>,
        _: &mut dyn DecodeRowSink,
        _: Option<&dyn zencodec::Pulse>,
    ) -> Result<Progress, StepError<Self::Error>> {
        Err(StepError::Control(ControlError::UnhandledBoundary))
    }
    fn requested(&self) -> &[InputRequest] {
        &[]
    }
    fn info(&self) -> Option<&ImageInfo> {
        None
    }
}
impl ControlledIncrementalDecode for ChangedCompletion {
    fn step_controlled(
        &mut self,
        _: Input<'_>,
        sink: &mut dyn DecodeRowSink,
        _: Option<&dyn zencodec::Pulse>,
    ) -> Result<ControlledProgress, StepError<Self::Error>> {
        if !self.adopted {
            return Ok(ControlledProgress::Suspended {
                consumed: 0,
                boundary: self.boundary,
            });
        }
        sink.begin(1, 1, PixelDescriptor::RGB8_SRGB)
            .map_err(StepError::Sink)?;
        let _ = sink
            .provide_next_buffer(0, 1, 1, PixelDescriptor::RGB8_SRGB)
            .map_err(StepError::Sink)?;
        sink.finish().map_err(StepError::Sink)?;
        // All sink calls match the adopted facts. Only the successful completion
        // contradicts them; the wrapper must refuse the successful completion.
        Ok(Progress::new(0, Event::Done(self.done.clone())).into())
    }
    fn resume(
        &mut self,
        id: BoundaryId,
        action: BoundaryAction,
        _: Option<&dyn zencodec::Pulse>,
    ) -> Result<(), StepError<Self::Error>> {
        assert_eq!(id, self.boundary.id());
        assert_eq!(action, BoundaryAction::AdoptInterpretation);
        self.adopted = true;
        Ok(())
    }
}

#[test]
fn policy_decoder_rejects_done_facts_that_contradict_adopted_output() {
    for done in [
        OutputInfo::full_decode(2, 1, PixelDescriptor::RGB8_SRGB),
        OutputInfo::full_decode(1, 2, PixelDescriptor::RGB8_SRGB),
        OutputInfo::full_decode(1, 1, PixelDescriptor::RGBA8_SRGB),
    ] {
        let decoder = ChangedCompletion {
            boundary: DecodeBoundary::new(InterpretationFacts::new(
                1,
                1,
                Some(PixelDescriptor::RGB8_SRGB),
                InterpretationState::Final,
            ))
            .unwrap(),
            adopted: false,
            done,
        };
        let mut decoder = PolicyDecoder::new(decoder, PolicySet::new());
        let mut sink = CollectSink::new();
        decoder
            .step(Input::empty(), &mut sink, Some(&NoPulse))
            .unwrap();
        let result = decoder.step(Input::empty(), &mut sink, Some(&NoPulse));
        assert!(
            matches!(
                result,
                Err(StepError::Control(ControlError::InconsistentFacts))
            ),
            "successful completion must match the already adopted facts: {result:?}"
        );
        assert!(matches!(
            decoder.step(Input::empty(), &mut sink, Some(&NoPulse)),
            Err(StepError::Finished)
        ));
    }
}
