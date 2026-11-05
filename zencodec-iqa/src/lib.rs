//! Small, object-safe still-image quality contract.
//!
//! A configured scorer exposes one named scalar. [`crate::score`] binds that scalar
//! to its complete configured identity and validates geometry, cancellation,
//! pixel admission and nonfinite behavior. Implementations validate the actual
//! [`zenpixels::PixelSlice`], including attached color context, alpha, precision
//! and luminance interpretation. Descriptor equality alone is insufficient.
//!
//! No conversion, decoder, metric kernel, thread pool or search loop is included.
//! Native component planes, maps, temporal pooling and batched GPU execution
//! remain outside this initial contract. No cold replay is called warm reuse.
#![no_std]
#![forbid(unsafe_code)]
#![deny(missing_docs)]

extern crate alloc;

use alloc::{borrow::Cow, boxed::Box};
use core::fmt;
pub use how_far::{self, AsStopReason, NoPulse, Pulse, SharedPulse, StopReason};
use how_far::{Complete, PhaseSpec, ProgressExt, Stages, Total};
pub use whereat::{self, At, ErrorAtExt, ResultAtExt};
use zenpixels::PixelSlice;

whereat::define_at_crate_info!(path = "zencodec-iqa");

#[cfg(feature = "codec")]
pub mod codec;

/// Result retaining a located, classified failure and its original cause.
pub type Result<T> = core::result::Result<T, At<Error>>;

/// Coarse classification; never infer this by parsing backend messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ErrorKind {
    /// Actual samples or their interpretation are unsupported.
    UnsupportedInput,
    /// Reference and candidate dimensions disagree.
    DimensionMismatch,
    /// Pixel admission or an implementation's managed resource bound failed.
    Resource,
    /// Caller cancellation or deadline, preserving the first observed reason.
    Stopped(StopReason),
    /// The requested operation is unavailable; no implicit fallback occurred.
    UnsupportedOperation,
    /// Backend failure, or a scalar outside the declared nonfinite policy.
    Backend,
}

/// Metric failure. Backend causes remain accessible through `Error::source`.
#[non_exhaustive]
pub struct Error {
    kind: ErrorKind,
    cause: Option<Box<dyn core::error::Error + Send + Sync>>,
}
impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Native payloads remain opt-in through source(), as in codec errors.
        f.debug_struct("Error")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}
impl Error {
    /// Create a classified failure without an underlying cause.
    pub fn new(kind: ErrorKind) -> Self {
        Self { kind, cause: None }
    }
    /// Classification fixed at construction.
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }
    /// Retain a concrete backend or mapping cause without formatting it.
    pub fn with_cause(mut self, cause: impl core::error::Error + Send + Sync + 'static) -> Self {
        self.cause = Some(Box::new(cause));
        self
    }
    /// Convert an already-located backend error, transferring its trace to the
    /// outer envelope and retaining the native cause. No nested `At<At<_>>`.
    #[track_caller]
    pub fn from_located<E>(kind: ErrorKind, cause: At<E>) -> At<Self>
    where
        E: core::error::Error + Send + Sync + 'static,
    {
        cause
            .map_error(|cause| Self::new(kind).with_cause(cause))
            .at()
    }
}
impl From<StopReason> for Error {
    fn from(reason: StopReason) -> Self {
        Self::new(ErrorKind::Stopped(reason))
    }
}
impl AsStopReason for Error {
    fn as_stop_reason(&self) -> Option<StopReason> {
        match self.kind {
            ErrorKind::Stopped(reason) => Some(reason),
            _ => None,
        }
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "image quality evaluation failed: {:?}", self.kind)
    }
}
impl core::error::Error for Error {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        self.cause.as_deref().map(|e| e as _)
    }
}
#[track_caller]
fn fail<T>(kind: ErrorKind) -> Result<T> {
    Err(Error::new(kind).start_at().at_crate(at_crate_info()))
}

/// Classify the foreign `At<Error>` wrapper for how-far's `*_classified`
/// handoffs. The underlying [`crate::Error`] implements [`how_far::AsStopReason`]; Rust's
/// orphan rules prevent implementing it on `whereat::At` here.
pub fn stop_reason(error: &At<Error>) -> Option<StopReason> {
    error.error().as_stop_reason()
}

/// Complete identity of the configured arithmetic, retained in every result.
///
/// `configuration` is an implementation-defined canonical string binding all
/// score-affecting options, preprocessing, model/graph hashes and display
/// conditions. `backend` identifies arithmetic/runtime separately from the
/// reproduced method. These strings are identities, not abbreviated hashes.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct MetricId {
    /// Semantic family and selected model/profile, e.g. `zensim/ECodec`.
    pub method: Cow<'static, str>,
    /// Reproduced method/version, distinct from the execution engine.
    pub reference: Cow<'static, str>,
    /// Implementing crate and version/build identity.
    pub implementation: Cow<'static, str>,
    /// Bump whenever any output can change.
    pub score_revision: u32,
    /// Execution backend/runtime affecting arithmetic.
    pub backend: Cow<'static, str>,
    /// Canonical complete score-affecting configuration.
    pub configuration: Cow<'static, str>,
}
impl MetricId {
    /// Identify a CPU implementation; refine its backend and complete
    /// score-affecting configuration with the builders before use.
    pub fn new(
        method: impl Into<Cow<'static, str>>,
        reference: impl Into<Cow<'static, str>>,
        implementation: impl Into<Cow<'static, str>>,
        score_revision: u32,
    ) -> Self {
        Self {
            method: method.into(),
            reference: reference.into(),
            implementation: implementation.into(),
            score_revision,
            backend: Cow::Borrowed("cpu"),
            configuration: Cow::Borrowed(""),
        }
    }
    /// Record the execution backend/runtime separately from the method.
    pub fn with_backend(mut self, backend: impl Into<Cow<'static, str>>) -> Self {
        self.backend = backend.into();
        self
    }
    /// Bind every score-affecting option, model and preprocessing identity.
    pub fn with_configuration(mut self, configuration: impl Into<Cow<'static, str>>) -> Self {
        self.configuration = configuration.into();
        self
    }
}

/// Which direction represents better quality; no common normalized scale.
/// Consumers retain a wildcard when matching future directions.
///
/// ```compile_fail
/// fn label(direction: zencodec_iqa::Direction) -> &'static str {
///     match direction {
///         zencodec_iqa::Direction::Higher => "higher",
///         zencodec_iqa::Direction::Lower => "lower",
///     }
/// }
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Direction {
    /// Larger is better.
    Higher,
    /// Smaller is better.
    Lower,
}
/// Permitted nonfinite scalar behavior. NaN is always a failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum NonFinite {
    /// Every successful result is finite.
    Refuse,
    /// Positive infinity is meaningful, e.g. PSNR on identical samples.
    PositiveInfinity,
}
/// The one configured output exposed by a scorer instance.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ScoreSpec {
    /// Stable output key within the complete metric identity.
    pub key: Cow<'static, str>,
    /// Comparison direction.
    pub direction: Direction,
    /// Native unit/scale, e.g. `dB` or `JOD`; never inferred from direction.
    pub unit: Cow<'static, str>,
    /// Successful nonfinite behavior.
    pub nonfinite: NonFinite,
}
impl ScoreSpec {
    /// Declare a named scalar that requires finite successful results.
    pub fn new(
        key: impl Into<Cow<'static, str>>,
        direction: Direction,
        unit: impl Into<Cow<'static, str>>,
    ) -> Self {
        Self {
            key: key.into(),
            direction,
            unit: unit.into(),
            nonfinite: NonFinite::Refuse,
        }
    }
    /// Declare meaningful nonfinite results explicitly.
    pub fn with_nonfinite(mut self, nonfinite: NonFinite) -> Self {
        self.nonfinite = nonfinite;
        self
    }
}
/// Configured scorer identity and output contract.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Descriptor {
    /// Complete configured identity.
    pub id: MetricId,
    /// Selected output's semantics.
    pub output: ScoreSpec,
}
impl Descriptor {
    /// Bind an output specification to the complete configured metric identity.
    pub fn new(id: MetricId, output: ScoreSpec) -> Self {
        Self { id, output }
    }
}
/// An owned scalar and its complete configured identity.
/// Input/preparation provenance remains the caller's responsibility.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Evaluation {
    /// Identity copied before execution.
    pub descriptor: Descriptor,
    /// Native scalar; no normalization, clipping or substituted fallback.
    pub value: f64,
}

/// Controls for one operation. The pixel bound covers each active image, not
/// backend scratch, retained references or native decoder allocations.
/// Construct through [`crate::EvalOpts::new`] and its builders so future controls can
/// be added compatibly.
///
/// ```compile_fail
/// // Public fields remain readable, but adding controls must not break callers.
/// let options = zencodec_iqa::EvalOpts {
///     pulse: &zencodec_iqa::NoPulse,
///     max_pixels: None,
/// };
/// ```
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct EvalOpts<'a> {
    /// Cooperative cancellation/progress; implementations poll within work.
    pub pulse: &'a dyn Pulse,
    /// Maximum active pixels in each image; `None` means no ingress ceiling.
    pub max_pixels: Option<u64>,
}
impl Default for EvalOpts<'_> {
    fn default() -> Self {
        Self {
            pulse: &NoPulse,
            max_pixels: None,
        }
    }
}
impl<'a> EvalOpts<'a> {
    /// Default controls: no observation/cancellation and no ingress ceiling.
    pub fn new() -> EvalOpts<'static> {
        EvalOpts::default()
    }
    /// Borrow the complete Pulse protocol, including reporting, planning and
    /// sharing. Implementations may use `pulse.share()` for owned workers and
    /// must join them before returning. This grants no completion rights over
    /// the caller's pulse.
    pub fn with_pulse<'b>(self, pulse: &'b dyn Pulse) -> EvalOpts<'b> {
        EvalOpts {
            pulse,
            max_pixels: self.max_pixels,
        }
    }
    /// Set the enforced active-pixel ceiling for each input image.
    pub fn with_max_pixels(mut self, max_pixels: u64) -> Self {
        self.max_pixels = Some(max_pixels);
        self
    }
    /// Observe caller cancellation without losing its reason.
    #[track_caller]
    pub fn checkpoint(&self) -> Result<()> {
        self.pulse
            .check()
            .map_err(|reason| Error::from(reason).start_at().at_crate(at_crate_info()))
    }
    /// Report actual completed work and check cancellation. Algorithms choose
    /// their native units (rows, tiles, etc.); the wrapper invents no work count.
    #[track_caller]
    pub fn step(&self, completed: u64) -> Result<()> {
        self.pulse
            .step(completed)
            .map_err(|reason| Error::from(reason).start_at().at_crate(at_crate_info()))
    }
    #[track_caller]
    fn admit(&self, pixels: &PixelSlice<'_>) -> Result<()> {
        self.checkpoint()?;
        if pixels.width() == 0 || pixels.rows() == 0 {
            return fail(ErrorKind::UnsupportedInput);
        }
        let count = u64::from(pixels.width()) * u64::from(pixels.rows());
        if self.max_pixels.is_some_and(|limit| count > limit) {
            return fail(ErrorKind::Resource);
        }
        Ok(())
    }
}

// One owned child per checked operation. Backends can split/report through the
// child; only this owner completes it with the actual result. Failed planning
// remains diagnostic through how-far's untracked cancellation-preserving path.
#[track_caller]
fn operation<T>(
    opts: EvalOpts<'_>,
    name: &'static str,
    work: impl FnOnce(EvalOpts<'_>) -> Result<T>,
) -> Result<T> {
    let mut stages = Stages::new(opts.pulse, &[PhaseSpec::new(name, 1, Total::Unknown)]);
    let result = stages.run_classified(
        |error| stop_reason(error).is_some(),
        |pulse| work(opts.with_pulse(pulse)),
    );
    stages
        .complete_classified(result, |error| stop_reason(error).is_some())
        .at()
}

/// Mutable configured full-reference scorer, with reusable backend scratch.
///
/// Call [`crate::score`] for checked admission/result envelopes. Implementations must
/// validate both actual views and their color contexts before arithmetic, read
/// active rows at their native strides, and poll `opts` during long work.
/// No implicit color, alpha, precision or geometry conversion is permitted.
/// Do not retain the operation pulse in prepared work: each later evaluation
/// receives its own pulse and completion lifetime.
/// Implementors remain open; future optional operations must have default
/// refusal implementations so existing scorers continue compiling.
pub trait Metric: Send {
    /// Stable configured identity; evaluation must not silently change it.
    fn descriptor(&self) -> &Descriptor;
    /// Native computation. Prefer the checked [`crate::score`] entry point as a caller.
    fn evaluate(
        &mut self,
        reference: PixelSlice<'_>,
        candidate: PixelSlice<'_>,
        opts: EvalOpts<'_>,
    ) -> Result<f64>;
    /// Prepare actual reference-work reuse with exact cold-score parity.
    /// Default refuses; it does not copy/replay the reference implicitly.
    fn prepare_reference(
        &mut self,
        _reference: PixelSlice<'_>,
        _opts: EvalOpts<'_>,
    ) -> Result<Box<dyn WarmReference + '_>> {
        fail(ErrorKind::UnsupportedOperation)
    }
}
/// Real reference-work reuse. Candidate validation and cancellation obligations
/// are identical to [`crate::Metric::evaluate`]. A failed call must leave the session
/// reusable or return an error on subsequent calls; never return stale results.
pub trait WarmReference: Send {
    /// Score a candidate against the held reference work, preserving cold bits.
    fn evaluate(&mut self, candidate: PixelSlice<'_>, opts: EvalOpts<'_>) -> Result<f64>;
}
/// Native no-reference scoring; no synthetic reference image is required.
pub trait NoReferenceMetric: Send {
    /// Stable identity and selected scalar contract.
    fn descriptor(&self) -> &Descriptor;
    /// Compute from one actual view, with the same interpretation obligations.
    fn evaluate(&mut self, image: PixelSlice<'_>, opts: EvalOpts<'_>) -> Result<f64>;
}

#[track_caller]
fn finish(descriptor: Descriptor, value: f64, opts: EvalOpts<'_>) -> Result<Evaluation> {
    opts.checkpoint()?;
    if !value.is_finite()
        && !(value == f64::INFINITY && descriptor.output.nonfinite == NonFinite::PositiveInfinity)
    {
        return fail(ErrorKind::Backend);
    }
    Ok(Evaluation { descriptor, value })
}
/// Admit both views, evaluate and bind the result to the configured identity.
/// Equal dimensions do not certify equal color or display interpretation.
/// The wrapper owns and completes one child phase with the final result;
/// the backend receives that child's full Pulse. The borrowed parent remains
/// the caller's responsibility. Phase labels describe work, not stable IDs.
#[track_caller]
pub fn score(
    metric: &mut dyn Metric,
    reference: PixelSlice<'_>,
    candidate: PixelSlice<'_>,
    opts: EvalOpts<'_>,
) -> Result<Evaluation> {
    operation(opts, "score", |opts| {
        opts.admit(&reference).at()?;
        opts.admit(&candidate).at()?;
        if (reference.width(), reference.rows()) != (candidate.width(), candidate.rows()) {
            return fail(ErrorKind::DimensionMismatch);
        }
        let descriptor = metric.descriptor().clone();
        let value = metric.evaluate(reference, candidate, opts).at()?;
        finish(descriptor, value, opts).at()
    })
}
/// Admit a single image and return a native no-reference scalar envelope.
#[track_caller]
pub fn score_no_reference(
    metric: &mut dyn NoReferenceMetric,
    image: PixelSlice<'_>,
    opts: EvalOpts<'_>,
) -> Result<Evaluation> {
    operation(opts, "score no reference", |opts| {
        opts.admit(&image).at()?;
        let descriptor = metric.descriptor().clone();
        let value = metric.evaluate(image, opts).at()?;
        finish(descriptor, value, opts).at()
    })
}
/// Prepared work retaining the original descriptor and active reference size.
#[non_exhaustive]
pub struct PreparedReference<'a> {
    descriptor: Descriptor,
    size: (u32, u32),
    inner: Box<dyn WarmReference + 'a>,
}
impl PreparedReference<'_> {
    /// Admit the candidate and score against actual retained reference work.
    #[track_caller]
    pub fn score(&mut self, candidate: PixelSlice<'_>, opts: EvalOpts<'_>) -> Result<Evaluation> {
        operation(opts, "score prepared reference", |opts| {
            opts.admit(&candidate).at()?;
            if self.size != (candidate.width(), candidate.rows()) {
                return fail(ErrorKind::DimensionMismatch);
            }
            let value = self.inner.evaluate(candidate, opts).at()?;
            finish(self.descriptor.clone(), value, opts).at()
        })
    }
}
/// Admit a reference and request real reuse. Backend retained allocations must
/// be fallible and controlled by implementation-specific construction limits.
#[track_caller]
pub fn prepare_reference<'a>(
    metric: &'a mut dyn Metric,
    reference: PixelSlice<'_>,
    opts: EvalOpts<'_>,
) -> Result<PreparedReference<'a>> {
    operation(opts, "prepare reference", |opts| {
        opts.admit(&reference).at()?;
        let descriptor = metric.descriptor().clone();
        let size = (reference.width(), reference.rows());
        let inner = metric.prepare_reference(reference, opts).at()?;
        opts.checkpoint().at()?;
        Ok(PreparedReference {
            descriptor,
            size,
            inner,
        })
    })
}
