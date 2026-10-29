//! Expose mandatory suspensions through the existing byte cursors.

use zencodec::ImageInfo;
use zencodec::decode::{
    BoundaryAction, BoundaryId, ControlledIncrementalDecode, ControlledProgress, DecodeBoundary,
    DecodeRowSink, Event, Fused, IncrementalDecode, Input, InputRequest, StepError,
};
use zenpixels::PixelDescriptor;

use crate::{ByteStepper, Step, Stepped};

/// A controlled decoder's observable progress. A suspension is never completion.
#[derive(Debug)]
#[non_exhaustive]
pub enum ControlledEvent {
    /// Ordinary metadata, rows or completed output.
    Decode(Event),
    /// A required decision before affected native or sink work can continue.
    Suspended(DecodeBoundary),
}

/// Drive a controlled decoder with [`crate::SliceCursor`], [`crate::ReadAtCursor`]
/// or an async cursor, returning mandatory boundaries to the adapter.
///
/// Use this when a transcode adapter must prepare conversion, metadata and encode
/// together while the decoder is suspended. Ordinary callers can instead use
/// [`crate::PolicyDecoder`] to resolve each boundary automatically.
///
/// Repeated cursor calls at a suspension return the same boundary without native
/// progress or reads. Only a matching [`Self::resume`] can continue. A native or
/// sink failure, cancellation or unwind drops the native session, so retaining
/// this adapter does not retain its managed buffers after failure.
pub struct ControlledStepper<'a, D, S> {
    decoder: Option<Fused<D>>,
    sink: &'a mut S,
}

impl<'a, D: ControlledIncrementalDecode, S: DecodeRowSink> ControlledStepper<'a, D, S> {
    /// Bind a decoder to its destination; construction performs no decoding.
    pub fn new(decoder: D, sink: &'a mut S) -> Self {
        Self {
            decoder: Some(Fused::new(decoder)),
            sink,
        }
    }

    /// Inspect the adapter's destination without releasing the native session.
    pub fn sink(&self) -> &S {
        self.sink
    }

    /// Configure a destination while the native session is suspended, for
    /// example by installing the selected conversion and encoder together.
    /// This does not resume the decoder or approve an interpretation.
    pub fn sink_mut(&mut self) -> &mut S {
        self.sink
    }

    /// Facts currently available from the held native session.
    pub fn info(&self) -> Option<&ImageInfo> {
        self.decoder.as_ref().and_then(IncrementalDecode::info)
    }

    /// Proposed output layout and interpretation, if known.
    pub fn output_descriptor(&self) -> Option<PixelDescriptor> {
        self.decoder
            .as_ref()
            .and_then(IncrementalDecode::output_descriptor)
    }

    /// Apply a decision to the exact held boundary. This resumes the protocol;
    /// the cursor's next call performs subsequent native work.
    /// `None` adds no per-call control and preserves native retained controls.
    pub fn resume(
        &mut self,
        boundary: BoundaryId,
        action: BoundaryAction,
        pulse: Option<&dyn zencodec::Pulse>,
    ) -> Result<(), StepError<D::Error>> {
        let mut decoder = self.decoder.take().ok_or(StepError::Finished)?;
        decoder.resume(boundary, action, pulse)?;
        self.decoder = Some(decoder);
        Ok(())
    }
}

impl<D: ControlledIncrementalDecode, S: DecodeRowSink> ByteStepper for ControlledStepper<'_, D, S> {
    type Output = ControlledEvent;
    type Error = StepError<D::Error>;

    fn step(
        &mut self,
        input: Input<'_>,
        pulse: Option<&dyn zencodec::Pulse>,
    ) -> Result<Stepped<Self::Output>, Self::Error> {
        // Taking ownership also makes a caught native/sink unwind terminal and
        // releases the session before control returns to its caller.
        let mut decoder = self.decoder.take().ok_or(StepError::Finished)?;
        let (consumed, step) = match decoder.step_controlled(input, self.sink, pulse)? {
            ControlledProgress::Suspended { consumed, boundary } => {
                (consumed, Step::Output(ControlledEvent::Suspended(boundary)))
            }
            ControlledProgress::Decode(progress) => {
                let step = match progress.event {
                    Event::Need => Step::Need,
                    event @ Event::Done(_) => Step::Finished(ControlledEvent::Decode(event)),
                    event => Step::Output(ControlledEvent::Decode(event)),
                };
                (progress.consumed, step)
            }
        };
        self.decoder = Some(decoder);
        Ok(Stepped { consumed, step })
    }

    fn requested(&self) -> &[InputRequest] {
        self.decoder
            .as_ref()
            .map_or(&[], IncrementalDecode::requested)
    }
}
