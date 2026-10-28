//! Serve mandatory decoder suspensions through one fallible policy resolver.

use core::fmt;

use zencodec::decode::{
    ControlledIncrementalDecode, ControlledProgress, DecodeRowSink, Event, Fused,
    IncrementalDecode, Input, InputRequest, MetadataUpdate, Progress, StepError,
};
use zencodec::operation::{
    OperationPolicy, OperationReport, PolicyFailure, PolicySet, resolve_boundary,
};
use zencodec::source::AccessPattern;
use zencodec::{At, ImageInfo};
use zenpixels::PixelDescriptor;

/// Native failure or traced application-policy failure during controlled decode.
#[derive(Debug)]
#[non_exhaustive]
pub enum PolicyDecodeError<E> {
    /// The native decoder's original error.
    Native(E),
    /// Callback refusal/failure, invalid choice, or cancellation at a decision.
    Policy(At<PolicyFailure>),
}
impl<E: fmt::Display> fmt::Display for PolicyDecodeError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Native(e) => e.fmt(f),
            Self::Policy(e) => e.fmt(f),
        }
    }
}
impl<E: core::error::Error + 'static> core::error::Error for PolicyDecodeError<E> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        Some(match self {
            Self::Native(e) => e,
            Self::Policy(e) => e,
        })
    }
}

/// Connect a controlled decoder to every existing slice, file and network
/// driver. Decisions run only at actual suspensions, before native work resumes.
/// The decoder remains terminal after a callback failure or caught unwind.
///
/// `ContinueReading` leaves affected output blocked. This adapter does not
/// certify native workspace or admit a codec's internal staging. Its bounded
/// report contains facts and decisions, not metadata or application payloads.
pub struct PolicyDecoder<D, R> {
    decoder: Option<Fused<D>>,
    resolver: R,
    preset: PolicySet,
    report: OperationReport,
    failed: bool,
}
impl<D, R> PolicyDecoder<D, R> {
    /// Use the resolver at mandatory boundaries and the default built-in policy
    /// when it returns `UsePreset`.
    pub fn new(decoder: D, resolver: R) -> Self {
        Self {
            decoder: Some(Fused::new(decoder)),
            resolver,
            preset: PolicySet::new(),
            report: OperationReport::default(),
            failed: false,
        }
    }
    /// Configure the built-in policy used for explicit delegation.
    /// During migration, these requirements and the policy argument
    /// both apply. A bound policy supplies preferences in place of these.
    #[deprecated(note = "Pass PolicySet or BoundPolicy as the operation policy argument")]
    pub fn with_policy_set(mut self, preset: PolicySet) -> Self {
        self.preset = preset;
        self
    }
    /// Decisions made before success or failure. Available after a driver error.
    pub const fn report(&self) -> &OperationReport {
        &self.report
    }
}
impl<D: ControlledIncrementalDecode, R: OperationPolicy> IncrementalDecode for PolicyDecoder<D, R> {
    type Error = PolicyDecodeError<D::Error>;
    fn step(
        &mut self,
        input: Input<'_>,
        sink: &mut dyn DecodeRowSink,
        pulse: &dyn zencodec::Pulse,
    ) -> Result<Progress, StepError<Self::Error>> {
        if self.failed {
            return Err(StepError::Finished);
        }
        // Pessimistic poisoning covers panics in native code and the callback.
        self.failed = true;
        // Keep the native session on this stack while doing fallible work. An
        // error or unwind drops its retained storage even if the caller keeps
        // this adapter to inspect the report. Only a successful step restores it.
        let mut decoder = self.decoder.take().ok_or(StepError::Finished)?;
        let progress = match decoder
            .step_controlled(input, sink, pulse)
            .map_err(|e| e.map_codec(PolicyDecodeError::Native))?
        {
            ControlledProgress::Decode(progress) => progress,
            ControlledProgress::Suspended { consumed, boundary } => {
                let (action, record) =
                    match resolve_boundary(boundary, &mut self.resolver, self.preset, pulse) {
                        Ok(answer) => answer,
                        Err(error) => {
                            self.report.record(error.error().record());
                            return Err(StepError::Codec(PolicyDecodeError::Policy(error)));
                        }
                    };
                self.report.record(record);
                decoder
                    .resume(boundary.id(), action, pulse)
                    .map_err(|e| e.map_codec(PolicyDecodeError::Native))?;
                // The decision is already handled. Let the outer driver observe
                // current metadata and schedule its next turn before more work.
                Progress::new(consumed, Event::Metadata(MetadataUpdate::Updated))
            }
        };
        self.decoder = Some(decoder);
        self.failed = false;
        Ok(progress)
    }
    fn requested(&self) -> &[InputRequest] {
        if self.failed {
            &[]
        } else {
            self.decoder.as_ref().map_or(&[], |d| d.requested())
        }
    }
    fn info(&self) -> Option<&ImageInfo> {
        self.decoder.as_ref().and_then(|d| d.info())
    }
    fn output_descriptor(&self) -> Option<PixelDescriptor> {
        self.decoder.as_ref().and_then(|d| d.output_descriptor())
    }
    fn access_pattern(&self) -> AccessPattern {
        self.decoder
            .as_ref()
            .map_or(AccessPattern::default(), |d| d.access_pattern())
    }
}
