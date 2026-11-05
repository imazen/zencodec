//! Small, object-safe still-image quality contract.
//!
//! A configured scorer exposes one named scalar. [`score`] binds that scalar
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
use how_far::{NoPulse, Pulse};
use whereat::At;
use zenpixels::PixelSlice;

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
    Stopped(enough::StopReason),
    /// The requested operation is unavailable; no implicit fallback occurred.
    UnsupportedOperation,
    /// Backend failure, or a scalar outside the declared nonfinite policy.
    Backend,
}

/// Metric failure. Backend causes remain accessible through `Error::source`.
#[derive(Debug)]
pub struct Error {
    /// Classification fixed at construction.
    pub kind: ErrorKind,
    cause: Option<Box<dyn core::error::Error + Send + Sync>>,
}
impl Error {
    /// Create a classified failure without an underlying cause.
    pub fn new(kind: ErrorKind) -> Self {
        Self { kind, cause: None }
    }
    /// Retain a concrete backend or mapping cause without formatting it.
    pub fn with_cause(mut self, cause: impl core::error::Error + Send + Sync + 'static) -> Self {
        self.cause = Some(Box::new(cause));
        self
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
fn fail<T>(kind: ErrorKind) -> Result<T> {
    Err(At::wrap(Error::new(kind)))
}

/// Complete identity of the configured arithmetic, retained in every result.
///
/// `configuration` is an implementation-defined canonical string binding all
/// score-affecting options, preprocessing, model/graph hashes and display
/// conditions. `backend` identifies arithmetic/runtime separately from the
/// reproduced method. These strings are identities, not abbreviated hashes.
#[derive(Clone, Debug, PartialEq, Eq)]
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

/// Which direction represents better quality; no common normalized scale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// Larger is better.
    Higher,
    /// Smaller is better.
    Lower,
}
/// Permitted nonfinite scalar behavior. NaN is always a failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NonFinite {
    /// Every successful result is finite.
    Refuse,
    /// Positive infinity is meaningful, e.g. PSNR on identical samples.
    PositiveInfinity,
}
/// The one configured output exposed by a scorer instance.
#[derive(Clone, Debug, PartialEq, Eq)]
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
/// Configured scorer identity and output contract.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Descriptor {
    /// Complete configured identity.
    pub id: MetricId,
    /// Selected output's semantics.
    pub output: ScoreSpec,
}
/// An owned scalar and its complete configured identity.
/// Input/preparation provenance remains the caller's responsibility.
#[derive(Clone, Debug, PartialEq)]
pub struct Evaluation {
    /// Identity copied before execution.
    pub descriptor: Descriptor,
    /// Native scalar; no normalization, clipping or substituted fallback.
    pub value: f64,
}

/// Controls for one operation. The pixel bound covers each active image, not
/// backend scratch, retained references or native decoder allocations.
#[derive(Clone, Copy)]
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
impl EvalOpts<'_> {
    /// Observe caller cancellation without losing its reason.
    pub fn checkpoint(&self) -> Result<()> {
        self.pulse
            .check()
            .map_err(|reason| At::wrap(Error::new(ErrorKind::Stopped(reason))))
    }
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

/// Mutable configured full-reference scorer, with reusable backend scratch.
///
/// Call [`score`] for checked admission/result envelopes. Implementations must
/// validate both actual views and their color contexts before arithmetic, read
/// active rows at their native strides, and poll `opts` during long work.
/// No implicit color, alpha, precision or geometry conversion is permitted.
pub trait Metric: Send {
    /// Stable configured identity; evaluation must not silently change it.
    fn descriptor(&self) -> &Descriptor;
    /// Native computation. Prefer the checked [`score`] entry point as a caller.
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
/// are identical to [`Metric::evaluate`]. A failed call must leave the session
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
pub fn score(
    metric: &mut dyn Metric,
    reference: PixelSlice<'_>,
    candidate: PixelSlice<'_>,
    opts: EvalOpts<'_>,
) -> Result<Evaluation> {
    opts.admit(&reference)?;
    opts.admit(&candidate)?;
    if (reference.width(), reference.rows()) != (candidate.width(), candidate.rows()) {
        return fail(ErrorKind::DimensionMismatch);
    }
    let descriptor = metric.descriptor().clone();
    let value = metric.evaluate(reference, candidate, opts)?;
    finish(descriptor, value, opts)
}
/// Admit a single image and return a native no-reference scalar envelope.
pub fn score_no_reference(
    metric: &mut dyn NoReferenceMetric,
    image: PixelSlice<'_>,
    opts: EvalOpts<'_>,
) -> Result<Evaluation> {
    opts.admit(&image)?;
    let descriptor = metric.descriptor().clone();
    let value = metric.evaluate(image, opts)?;
    finish(descriptor, value, opts)
}
/// Prepared work retaining the original descriptor and active reference size.
pub struct PreparedReference<'a> {
    descriptor: Descriptor,
    size: (u32, u32),
    inner: Box<dyn WarmReference + 'a>,
}
impl PreparedReference<'_> {
    /// Admit the candidate and score against actual retained reference work.
    pub fn score(&mut self, candidate: PixelSlice<'_>, opts: EvalOpts<'_>) -> Result<Evaluation> {
        opts.admit(&candidate)?;
        if self.size != (candidate.width(), candidate.rows()) {
            return fail(ErrorKind::DimensionMismatch);
        }
        let value = self.inner.evaluate(candidate, opts)?;
        finish(self.descriptor.clone(), value, opts)
    }
}
/// Admit a reference and request real reuse. Backend retained allocations must
/// be fallible and controlled by implementation-specific construction limits.
pub fn prepare_reference<'a>(
    metric: &'a mut dyn Metric,
    reference: PixelSlice<'_>,
    opts: EvalOpts<'_>,
) -> Result<PreparedReference<'a>> {
    opts.admit(&reference)?;
    let descriptor = metric.descriptor().clone();
    let size = (reference.width(), reference.rows());
    let inner = metric.prepare_reference(reference, opts)?;
    opts.checkpoint()?;
    Ok(PreparedReference {
        descriptor,
        size,
        inner,
    })
}
