//! Serve mandatory decoder suspensions through one fallible policy resolver.

use core::fmt;

use zencodec::decode::{
    ControlledIncrementalDecode, ControlledProgress, DecodeRowSink, Event, Fused,
    IncrementalDecode, Input, InputRequest, MetadataUpdate, Progress, StepError,
};
use zencodec::operation::{
    OperationPolicy, OperationReport, PolicyFailure, PolicySet, UnmetRequirements, resolve_boundary,
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
    /// Required workspace is unqualified before native polling.
    Requirements(At<UnmetRequirements>),
    /// Callback refusal/failure, invalid choice, or cancellation at a decision.
    Policy(At<PolicyFailure>),
}
impl<E: fmt::Display> fmt::Display for PolicyDecodeError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Native(e) => e.fmt(f),
            Self::Requirements(e) => e.fmt(f),
            Self::Policy(e) => e.fmt(f),
        }
    }
}
impl<E: core::error::Error + 'static> core::error::Error for PolicyDecodeError<E> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        Some(match self {
            Self::Native(e) => e,
            Self::Requirements(e) => e,
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
/// A workspace requirement refuses before the first native step because this
/// adapter has no qualified native bound. Construction of the supplied decoder
/// precedes this check and remains the caller/adapter's admission responsibility.
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
impl<D, R: OperationPolicy> PolicyDecoder<D, R> {
    /// Admit requirements before constructing a native decoder.
    ///
    /// Prefer this to [`Self::new`] when native construction may allocate. The
    /// factory is invoked exactly once on successful admission and never on
    /// admission refusal. Its original error is retained as
    /// [`PolicyDecodeError::Native`]. The same policy is moved into the returned
    /// adapter, where metadata and interpretation decisions still occur later.
    ///
    /// This generic adapter has no qualified native workspace bound, so any
    /// configured workspace limit refuses with `Workspace(Unknown)` before the
    /// factory runs. Other requirements remain pending for their boundaries;
    /// success here does not certify metadata, fidelity or output scheduling.
    /// Configure requirements on `policy` before this call, not with a later
    /// legacy setter. Values evaluated before the closure is invoked (including
    /// captured allocations) cannot be admitted by this method.
    ///
    /// ```ignore
    /// let decoder = PolicyDecoder::try_new(
    ///     || request.start_controlled(&codecs, storage),
    ///     policy,
    ///     pulse,
    /// )?;
    /// ```
    ///
    /// Cancellation is checked before and after construction. If cancellation
    /// arrives during a successful factory call, the decoder is dropped before
    /// return. Lengthy factories must poll the same pulse themselves; this check
    /// cannot preempt synchronous work. `None` skips these per-call checks only.
    pub fn try_new<E>(
        factory: impl FnOnce() -> Result<D, E>,
        policy: R,
        pulse: Option<&dyn zencodec::Pulse>,
    ) -> Result<Self, StepError<PolicyDecodeError<E>>> {
        let pulse = pulse.unwrap_or(&zencodec::NoPulse);
        pulse.check()?;
        let decoder = Self::from_admitted_factory(factory, policy).map_err(StepError::Codec)?;
        pulse.check()?;
        Ok(decoder)
    }

    /// Construct after workspace admission, without per-call cancellation.
    #[deprecated(
        note = "Use try_new(factory, policy, pulse); pass None when no per-call control is needed"
    )]
    pub fn try_from_factory<E>(
        factory: impl FnOnce() -> Result<D, E>,
        policy: R,
    ) -> Result<Self, PolicyDecodeError<E>> {
        Self::from_admitted_factory(factory, policy)
    }

    fn from_admitted_factory<E>(
        factory: impl FnOnce() -> Result<D, E>,
        policy: R,
    ) -> Result<Self, PolicyDecodeError<E>> {
        if let Some(requirements) = policy.requirements() {
            requirements.check_workspace(None).map_err(|error| {
                PolicyDecodeError::Requirements(
                    At::wrap(error).at_named("controlled decoder factory admission"),
                )
            })?;
        }
        let decoder = factory().map_err(PolicyDecodeError::Native)?;
        Ok(Self::new(decoder, policy))
    }
}
impl<D: ControlledIncrementalDecode, R: OperationPolicy> IncrementalDecode for PolicyDecoder<D, R> {
    type Error = PolicyDecodeError<D::Error>;
    fn step(
        &mut self,
        input: Input<'_>,
        sink: &mut dyn DecodeRowSink,
        pulse: Option<&dyn zencodec::Pulse>,
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
        // Deferring metadata discovery must not defer a resource deadline. This
        // adapter has no qualified native workspace evidence. Check both entry
        // policies before even polling the native coroutine; a discovery step
        // may already allocate. The taken owner drops on refusal or unwind.
        for requirements in [Some(self.preset), self.resolver.requirements()]
            .into_iter()
            .flatten()
        {
            requirements.check_workspace(None).map_err(|error| {
                StepError::Codec(PolicyDecodeError::Requirements(
                    At::wrap(error).at_named("controlled decode workspace admission"),
                ))
            })?;
        }
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
    fn take_details(&mut self) -> Option<zencodec::decode::DecodeDetails> {
        if self.failed {
            return None;
        }
        self.decoder.as_mut().and_then(|d| d.take_details())
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
