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
    fn take_details(&mut self) -> Option<DecodeDetails> {
        self.inner.as_mut().and_then(|d| d.take_details())
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

/// Preparation and execution use the same owner. Final validation remains
/// authoritative even when a native implementation supplies incorrect facts.
pub(super) fn decode_owned(
    mut decoder: Box<dyn DynDecoder + '_>,
    required: Option<PixelDescriptor>,
    pulse: &dyn crate::Pulse,
) -> Result<crate::DecodeOutput, OperationError> {
    pulse.check().map_err(OperationError::Stopped)?;
    if let Some(requested) = required
        && let Some(actual) = decoder
            .prepare_output()
            .map_err(|e| OperationError::Codec(e.into()))?
        && actual != requested
    {
        return Err(OperationError::OutputDescriptor { requested, actual });
    }
    pulse.check().map_err(OperationError::Stopped)?;
    let output = decoder
        .decode()
        .map_err(|e| OperationError::Codec(e.into()))?;
    pulse.check().map_err(OperationError::Stopped)?;
    if let Some(requested) = required {
        let actual = output.pixels().descriptor();
        if actual != requested {
            return Err(OperationError::OutputDescriptor { requested, actual });
        }
    }
    Ok(output)
}

#[cfg(test)]
mod admission_tests {
    use super::*;
    use crate::{DecodeOutput, ImageFormat, NoPulse};
    use alloc::{rc::Rc, sync::Arc, vec};
    use core::cell::Cell;
    use core::sync::atomic::{AtomicBool, Ordering};
    use zenpixels::PixelBuffer;

    struct Native {
        prepared: Option<PixelDescriptor>,
        fail: bool,
        cancel: Option<Arc<AtomicBool>>,
        preparations: Rc<Cell<usize>>,
        executions: Rc<Cell<usize>>,
        drops: Rc<Cell<usize>>,
    }
    impl Drop for Native {
        fn drop(&mut self) {
            self.drops.set(self.drops.get() + 1);
        }
    }
    impl DynDecoder for Native {
        fn prepare_output(&mut self) -> Result<Option<PixelDescriptor>, BoxedError> {
            self.preparations.set(self.preparations.get() + 1);
            if let Some(cancel) = &self.cancel {
                cancel.store(true, Ordering::SeqCst);
            }
            if self.fail {
                return Err(Box::new(core::fmt::Error));
            }
            Ok(self.prepared)
        }
        fn decode(self: Box<Self>) -> Result<DecodeOutput, BoxedError> {
            self.executions.set(self.executions.get() + 1);
            Ok(DecodeOutput::new(
                PixelBuffer::from_vec(vec![1, 2, 3], 1, 1, PixelDescriptor::RGB8_SRGB).unwrap(),
                ImageInfo::new(1, 1, ImageFormat::Jpeg),
            ))
        }
    }
    #[test]
    fn preparation_is_optional_but_actual_output_is_always_checked() {
        let rgb = PixelDescriptor::RGB8_SRGB;
        let rgba = PixelDescriptor::RGBA8_SRGB;
        for (required, prepared, fail, prepares, executes, succeeds) in [
            (None, Some(rgba), true, 0, 1, true),
            (Some(rgb), Some(rgb), false, 1, 1, true),
            (Some(rgba), Some(rgb), false, 1, 0, false),
            (Some(rgba), Some(rgba), false, 1, 1, false),
            (Some(rgba), None, false, 1, 1, false),
            (Some(rgb), None, false, 1, 1, true),
            (Some(rgb), None, true, 1, 0, false),
        ] {
            let preparations = Rc::new(Cell::new(0));
            let executions = Rc::new(Cell::new(0));
            let drops = Rc::new(Cell::new(0));
            let native = Native {
                prepared,
                fail,
                cancel: None,
                preparations: preparations.clone(),
                executions: executions.clone(),
                drops: drops.clone(),
            };
            let result = decode_owned(Box::new(native), required, &NoPulse);
            assert_eq!(result.is_ok(), succeeds);
            assert_eq!(preparations.get(), prepares);
            assert_eq!(executions.get(), executes);
            assert_eq!(drops.get(), 1);
            if required.is_some() && fail {
                assert!(
                    crate::find_cause::<core::fmt::Error>(result.as_ref().unwrap_err()).is_some()
                );
            } else if !succeeds {
                assert!(matches!(
                    result,
                    Err(OperationError::OutputDescriptor { .. })
                ));
            }
        }
    }
    #[test]
    fn cancellation_before_or_during_preparation_prevents_execution() {
        struct Cancel(Arc<AtomicBool>, enough::StopReason);
        impl enough::Stop for Cancel {
            fn check(&self) -> Result<(), enough::StopReason> {
                if self.0.load(Ordering::SeqCst) {
                    Err(self.1)
                } else {
                    Ok(())
                }
            }
        }
        for reason in [enough::StopReason::Cancelled, enough::StopReason::TimedOut] {
            for cancelled in [false, true] {
                for prepared in [
                    None,
                    Some(PixelDescriptor::RGB8_SRGB),
                    Some(PixelDescriptor::RGBA8_SRGB),
                ] {
                    for fail in [false, true] {
                        let flag = Arc::new(AtomicBool::new(cancelled));
                        let preparations = Rc::new(Cell::new(0));
                        let executions = Rc::new(Cell::new(0));
                        let drops = Rc::new(Cell::new(0));
                        let native = Native {
                            prepared,
                            fail,
                            cancel: Some(flag.clone()),
                            preparations: preparations.clone(),
                            executions: executions.clone(),
                            drops: drops.clone(),
                        };
                        let pulse = crate::how_far::StopOnly::new(Cancel(flag, reason));
                        let result = decode_owned(
                            Box::new(native),
                            Some(PixelDescriptor::RGB8_SRGB),
                            &pulse,
                        );
                        if fail && !cancelled {
                            // Native failure remains primary even when it also raises a stop.
                            assert!(
                                crate::find_cause::<core::fmt::Error>(result.as_ref().unwrap_err())
                                    .is_some()
                            );
                        } else {
                            assert!(
                                matches!(result, Err(OperationError::Stopped(actual)) if actual == reason),
                                "successful preparation must preserve {reason:?}, prepared={prepared:?}"
                            );
                        }
                        assert_eq!(preparations.get(), usize::from(!cancelled));
                        assert_eq!(executions.get(), 0);
                        assert_eq!(drops.get(), 1);
                    }
                }
            }
        }
    }
}
