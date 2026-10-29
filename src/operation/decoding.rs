//! Stored requests bind configuration before starting a controlled native job.
use super::{DecodeRequest, OperationError, Output};
use crate::{
    CodecSet, CodecSetError, ImageFormat, ImageInfo, decode::*, source::AccessPattern,
    storage::StorageBudget,
};
use alloc::boxed::Box;

use zenpixels::PixelDescriptor;

pub(super) struct CheckedDecoder {
    request: DecodeRequest,
    codecs: CodecSet,
    storage: StorageBudget,
    inner: Option<Box<DynControlledDecoder>>,
    probe: [InputRequest; 1],
}
impl CheckedDecoder {
    pub(super) fn new(
        request: DecodeRequest,
        codecs: CodecSet,
        storage: StorageBudget,
    ) -> Result<Self, OperationError> {
        codecs
            .check_operation_stop()
            .map_err(OperationError::Stopped)?;
        let mut session = Self {
            request,
            codecs,
            storage,
            inner: None,
            probe: [InputRequest::at_least(
                0,
                ImageFormat::RECOMMENDED_PROBE_BYTES,
            )],
        };
        if let Some(format) = session.request.format {
            session.prepare(format)?;
        }
        Ok(session)
    }
    fn prepare(&mut self, format: ImageFormat) -> Result<(), OperationError> {
        let preferred = match &self.request.output {
            Output::Native => &[][..],
            Output::Prefer(list) => list,
            Output::Require(descriptor) => core::slice::from_ref(descriptor),
        };
        let job = self
            .codecs
            .checked_decode_job(format, self.request.policy)
            .map_err(OperationError::Codec)?;
        self.inner = Some(
            job.into_controlled_decoder(preferred, self.storage.clone())
                .map_err(|e| OperationError::Codec(CodecSetError::Codec(e)))?,
        );
        Ok(())
    }
}
impl IncrementalDecode for CheckedDecoder {
    type Error = OperationError;
    fn step(
        &mut self,
        _: Input<'_>,
        _: &mut dyn DecodeRowSink,
        _: Option<&dyn crate::Pulse>,
    ) -> Result<Progress, StepError<Self::Error>> {
        Err(StepError::Control(ControlError::UnhandledBoundary))
    }
    fn requested(&self) -> &[InputRequest] {
        self.inner.as_ref().map_or(&self.probe, |d| d.requested())
    }
    fn info(&self) -> Option<&ImageInfo> {
        self.inner.as_ref().and_then(|d| d.info())
    }
    fn output_descriptor(&self) -> Option<PixelDescriptor> {
        self.inner.as_ref().and_then(|d| d.output_descriptor())
    }
    fn access_pattern(&self) -> AccessPattern {
        self.inner.as_ref().map_or_else(
            || self.codecs.operation_access_pattern(),
            |d| d.access_pattern(),
        )
    }
}
impl ControlledIncrementalDecode for CheckedDecoder {
    fn step_controlled(
        &mut self,
        input: Input<'_>,
        sink: &mut dyn DecodeRowSink,
        pulse: Option<&dyn crate::Pulse>,
    ) -> Result<ControlledProgress, StepError<Self::Error>> {
        self.codecs.check_operation_stop()?;
        if self.inner.is_none() {
            let Some(format) = self
                .codecs
                .detect_incremental(&input)
                .map_err(|e| StepError::Codec(OperationError::Codec(e)))?
            else {
                return Ok(ControlledProgress::Decode(Progress::need(0)));
            };
            self.prepare(format).map_err(StepError::Codec)?;
        }
        let progress = self
            .inner
            .as_mut()
            .expect("prepared above")
            .step_controlled(input, sink, pulse)
            .map_err(|e| {
                e.map_codec(|e| OperationError::Codec(CodecSetError::Codec(e.into_inner())))
            })?;
        if let ControlledProgress::Suspended { boundary, .. } = &progress
            && boundary.facts().state == InterpretationState::Final
            && let Output::Require(requested) = self.request.output
            && let Some(actual) = boundary.facts().descriptor
            && actual != requested
        {
            return Err(StepError::Codec(OperationError::OutputDescriptor {
                requested,
                actual,
            }));
        }
        Ok(progress)
    }
    fn resume(
        &mut self,
        boundary: BoundaryId,
        action: BoundaryAction,
        pulse: Option<&dyn crate::Pulse>,
    ) -> Result<(), StepError<Self::Error>> {
        self.codecs.check_operation_stop()?;
        self.inner
            .as_mut()
            .ok_or(StepError::Control(ControlError::InvalidReply))?
            .resume(boundary, action, pulse)
            .map_err(|e| {
                e.map_codec(|e| OperationError::Codec(CodecSetError::Codec(e.into_inner())))
            })
    }
}
