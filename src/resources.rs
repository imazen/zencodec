//! Shared per-run resource admission: [`JobResources`].
//!
//! Combines [`ResourceLimits`], one shared [`StorageBudget`] (with opaque foreign
//! backing reservations), checked cumulative byte/frame/pixel counters, and one
//! [`SharedPulse`] cancellation/progress control. Request-level overrides may only
//! tighten host limits and compose cancellation pulses (never drop host cancellation).

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use core::sync::atomic::{AtomicU64, Ordering};

use enough::{Stop, StopReason};
use how_far::{
    Child, ChildPulse, Complete, Execution, Outcome, PhaseSpec, PlanError, Pulse, Report,
    SharedPulse, Total,
};

use crate::limits::{AllocPreference, LimitExceeded, ResourceLimits, ThreadingPolicy};
use crate::storage::StorageBudget;

#[derive(Default)]
struct Counters {
    input_bytes: AtomicU64,
    output_bytes: AtomicU64,
    frames: AtomicU64,
    pixels: AtomicU64,
}

/// Composes a host pulse and a narrower child pulse so derived handles can add
/// stage/request deadlines or progress scopes without ever bypassing host
/// cancellation. Checks `host` before `child` to preserve the host's exact
/// [`StopReason`], and preserves host checking across `split` and `share`.
struct ComposedPulse {
    host: SharedPulse,
    child: SharedPulse,
}

impl Stop for ComposedPulse {
    #[inline]
    fn check(&self) -> Result<(), StopReason> {
        self.host.check()?;
        self.child.check()
    }

    #[inline]
    fn may_stop(&self) -> bool {
        self.host.may_stop() || self.child.may_stop()
    }
}

impl Report for ComposedPulse {
    #[inline]
    fn advance(&self, n: u64) {
        self.child.advance(n);
        self.host.advance(n);
    }

    #[inline]
    fn may_report(&self) -> bool {
        self.child.may_report() || self.host.may_report()
    }
}

impl Pulse for ComposedPulse {
    fn record_issue(&self, error: PlanError) {
        self.child.record_issue(error);
    }

    fn split(&self, e: Execution, parts: &[PhaseSpec<'_>]) -> Result<Vec<Child<'_>>, PlanError> {
        let children = self.child.split(e, parts)?;
        Ok(children
            .into_iter()
            .map(|child| {
                Child::new(ComposedChild {
                    host: self.host.clone(),
                    child,
                })
            })
            .collect())
    }

    fn share(&self) -> Result<SharedPulse, PlanError> {
        Ok(SharedPulse::new(ComposedPulse {
            host: self.host.share()?,
            child: self.child.share()?,
        }))
    }

    fn start(&self) -> Result<(), PlanError> {
        self.child.start()
    }

    fn set_total(&self, total: Total) -> Result<(), PlanError> {
        self.child.set_total(total)
    }
}

struct ComposedChild<'a> {
    host: SharedPulse,
    child: Child<'a>,
}

impl Stop for ComposedChild<'_> {
    #[inline]
    fn check(&self) -> Result<(), StopReason> {
        self.host.check()?;
        self.child.check()
    }

    #[inline]
    fn may_stop(&self) -> bool {
        self.host.may_stop() || self.child.may_stop()
    }
}

impl Report for ComposedChild<'_> {
    #[inline]
    fn advance(&self, n: u64) {
        self.child.advance(n);
        self.host.advance(n);
    }

    #[inline]
    fn may_report(&self) -> bool {
        self.child.may_report() || self.host.may_report()
    }
}

impl Pulse for ComposedChild<'_> {
    fn record_issue(&self, error: PlanError) {
        self.child.record_issue(error);
    }

    fn split(&self, e: Execution, parts: &[PhaseSpec<'_>]) -> Result<Vec<Child<'_>>, PlanError> {
        let children = self.child.split(e, parts)?;
        Ok(children
            .into_iter()
            .map(|child| {
                Child::new(ComposedChild {
                    host: self.host.clone(),
                    child,
                })
            })
            .collect())
    }

    fn share(&self) -> Result<SharedPulse, PlanError> {
        Ok(SharedPulse::new(ComposedPulse {
            host: self.host.share()?,
            child: self.child.share()?,
        }))
    }

    fn start(&self) -> Result<(), PlanError> {
        self.child.start()
    }

    fn set_total(&self, total: Total) -> Result<(), PlanError> {
        self.child.set_total(total)
    }
}

impl ChildPulse for ComposedChild<'_> {
    fn complete_inferred(self: Box<Self>, outcome: Outcome) {
        self.child.complete_inferred(outcome);
    }

    fn complete_as(self: Box<Self>, outcome: Outcome) {
        Complete::complete_as(self.child, outcome);
    }

    fn finish(self: Box<Self>, outcome: Outcome) -> Result<(), PlanError> {
        self.child.finish(outcome)
    }
}

/// Shared resource limits, managed storage budget, cumulative counters, and
/// cancellation pulse for one admitted codec or pipeline run.
///
/// Cloning a [`JobResources`] shares the underlying [`StorageBudget`], cumulative
/// atomic counters, and [`SharedPulse`]. Derived handles created with
/// [`tighten_limits`](Self::tighten_limits) apply tighter numeric caps locally to
/// the returned handle (and any clones made from it) while still charging the same
/// shared counters and storage budget. Bind a tightened handle before handing out
/// clones when a request-wide cap should apply to every stage.
#[derive(Clone)]
pub struct JobResources {
    limits: ResourceLimits,
    storage: StorageBudget,
    pulse: SharedPulse,
    counters: Arc<Counters>,
}

impl JobResources {
    /// Construct a new resource bundle with explicit limits, storage budget, and pulse.
    #[must_use]
    pub fn new(limits: ResourceLimits, storage: StorageBudget, pulse: SharedPulse) -> Self {
        Self {
            limits,
            storage,
            pulse,
            counters: Arc::new(Counters::default()),
        }
    }

    /// Resource limits enforced by this handle.
    #[inline]
    #[must_use]
    pub const fn limits(&self) -> &ResourceLimits {
        &self.limits
    }

    /// Managed storage budget shared by all stages in this run.
    #[inline]
    #[must_use]
    pub const fn storage(&self) -> &StorageBudget {
        &self.storage
    }

    /// Cancellation and progress pulse bound to this handle.
    #[inline]
    #[must_use]
    pub const fn pulse(&self) -> &SharedPulse {
        &self.pulse
    }

    /// Check cooperative cancellation.
    #[inline]
    pub fn check_stop(&self) -> Result<(), StopReason> {
        self.pulse.check()
    }

    /// Return a derived handle sharing the same storage budget, cumulative counters,
    /// and pulse while tightening caps to the stricter of `self.limits` and `stricter`.
    ///
    /// - Numeric caps (`max_width`, `max_height`, `max_pixels`, `max_memory_bytes`,
    ///   `max_input_bytes`, `max_output_bytes`, `max_frames`, `max_animation_ms`,
    ///   `max_total_pixels`) are handle-local on the returned [`JobResources`] while
    ///   the recorded counters remain shared across parent and derived handles. Bind
    ///   the tightened handle before cloning if all stages should observe the tighter cap.
    /// - Threading narrows monotonically (`ThreadingPolicy::Sequential` overrides `Parallel`).
    /// - Allocation preference merges conservatively (`Fallible` > `CodecDefault` >
    ///   `Infallible`): tightening never weakens `Fallible` or `CodecDefault` to
    ///   `Infallible`.
    #[must_use]
    pub fn tighten_limits(&self, stricter: ResourceLimits) -> Self {
        let mut merged = self.limits;
        merged.max_width = min_opt_u32(merged.max_width, stricter.max_width);
        merged.max_height = min_opt_u32(merged.max_height, stricter.max_height);
        merged.max_pixels = min_opt_u64(merged.max_pixels, stricter.max_pixels);
        merged.max_memory_bytes = min_opt_u64(merged.max_memory_bytes, stricter.max_memory_bytes);
        merged.max_input_bytes = min_opt_u64(merged.max_input_bytes, stricter.max_input_bytes);
        merged.max_output_bytes = min_opt_u64(merged.max_output_bytes, stricter.max_output_bytes);
        merged.max_frames = min_opt_u32(merged.max_frames, stricter.max_frames);
        merged.max_animation_ms = min_opt_u64(merged.max_animation_ms, stricter.max_animation_ms);
        merged.max_total_pixels = min_opt_u64(merged.max_total_pixels, stricter.max_total_pixels);
        if stricter.threading == ThreadingPolicy::Sequential {
            merged.threading = ThreadingPolicy::Sequential;
        }
        merged.prefer_fallible_allocations = merge_alloc_preference(
            merged.prefer_fallible_allocations,
            stricter.prefer_fallible_allocations,
        );
        Self {
            limits: merged,
            storage: self.storage.clone(),
            pulse: self.pulse.clone(),
            counters: Arc::clone(&self.counters),
        }
    }

    /// Return a derived handle composing `child` with this handle's pulse while
    /// preserving the shared storage budget, counters, and limits.
    ///
    /// Host cancellation is checked before `child` (preserving the host's exact
    /// [`StopReason`]), including across `Pulse::split` and `Pulse::share`. A derived
    /// handle can add stricter per-stage cancellation or progress reporting, but can
    /// never bypass or replace host cancellation.
    #[must_use]
    pub fn with_child_pulse(&self, child: SharedPulse) -> Self {
        let composed = SharedPulse::new(ComposedPulse {
            host: self.pulse.clone(),
            child,
        });
        Self {
            limits: self.limits,
            storage: self.storage.clone(),
            pulse: composed,
            counters: Arc::clone(&self.counters),
        }
    }

    /// Cumulative input bytes recorded so far across all handles sharing this run.
    #[inline]
    #[must_use]
    pub fn recorded_input_bytes(&self) -> u64 {
        self.counters.input_bytes.load(Ordering::Acquire)
    }

    /// Cumulative output bytes recorded so far across all handles sharing this run.
    #[inline]
    #[must_use]
    pub fn recorded_output_bytes(&self) -> u64 {
        self.counters.output_bytes.load(Ordering::Acquire)
    }

    /// Cumulative frames recorded so far across all handles sharing this run.
    #[inline]
    #[must_use]
    pub fn recorded_frames(&self) -> u64 {
        self.counters.frames.load(Ordering::Acquire)
    }

    /// Cumulative pixels recorded so far across all handles sharing this run.
    #[inline]
    #[must_use]
    pub fn recorded_pixels(&self) -> u64 {
        self.counters.pixels.load(Ordering::Acquire)
    }

    /// Atomically add `bytes` to cumulative input bytes, refusing if `limits.max_input_bytes`
    /// would be exceeded or arithmetic would overflow.
    pub fn record_input_bytes(&self, bytes: u64) -> Result<u64, LimitExceeded> {
        add_checked(
            &self.counters.input_bytes,
            bytes,
            self.limits.max_input_bytes,
            |actual, max| LimitExceeded::InputSize { actual, max },
        )
    }

    /// Atomically add `bytes` to cumulative output bytes, refusing if `limits.max_output_bytes`
    /// would be exceeded or arithmetic would overflow.
    pub fn record_output_bytes(&self, bytes: u64) -> Result<u64, LimitExceeded> {
        add_checked(
            &self.counters.output_bytes,
            bytes,
            self.limits.max_output_bytes,
            |actual, max| LimitExceeded::OutputSize { actual, max },
        )
    }

    /// Atomically add `frames` to cumulative frame count, refusing if `limits.max_frames`
    /// would be exceeded or arithmetic would overflow.
    pub fn record_frames(&self, frames: u64) -> Result<u64, LimitExceeded> {
        add_checked(
            &self.counters.frames,
            frames,
            self.limits.max_frames.map(u64::from),
            |actual, max| LimitExceeded::Frames {
                actual: actual.min(u64::from(u32::MAX)) as u32,
                max: max.min(u64::from(u32::MAX)) as u32,
            },
        )
    }

    /// Atomically add `pixels` to cumulative pixel count, refusing if `limits.max_total_pixels`
    /// would be exceeded or arithmetic would overflow.
    pub fn record_pixels(&self, pixels: u64) -> Result<u64, LimitExceeded> {
        add_checked(
            &self.counters.pixels,
            pixels,
            self.limits.max_total_pixels,
            |actual, max| LimitExceeded::TotalPixels { actual, max },
        )
    }
}

impl fmt::Debug for JobResources {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JobResources")
            .field("limits", &self.limits)
            .field("storage", &self.storage)
            .field("input_bytes", &self.recorded_input_bytes())
            .field("output_bytes", &self.recorded_output_bytes())
            .field("frames", &self.recorded_frames())
            .field("pixels", &self.recorded_pixels())
            .finish_non_exhaustive()
    }
}

fn merge_alloc_preference(a: AllocPreference, b: AllocPreference) -> AllocPreference {
    match (a, b) {
        (AllocPreference::Fallible, _) | (_, AllocPreference::Fallible) => {
            AllocPreference::Fallible
        }
        (AllocPreference::CodecDefault, _) | (_, AllocPreference::CodecDefault) => {
            AllocPreference::CodecDefault
        }
        (AllocPreference::Infallible, AllocPreference::Infallible) => AllocPreference::Infallible,
    }
}

fn min_opt_u32(a: Option<u32>, b: Option<u32>) -> Option<u32> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (Some(x), None) | (None, Some(x)) => Some(x),
        (None, None) => None,
    }
}

fn min_opt_u64(a: Option<u64>, b: Option<u64>) -> Option<u64> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (Some(x), None) | (None, Some(x)) => Some(x),
        (None, None) => None,
    }
}

fn add_checked(
    counter: &AtomicU64,
    delta: u64,
    limit: Option<u64>,
    make_err: fn(u64, u64) -> LimitExceeded,
) -> Result<u64, LimitExceeded> {
    let mut cur = counter.load(Ordering::Acquire);
    loop {
        let Some(next) = cur.checked_add(delta) else {
            return Err(make_err(u64::MAX, limit.unwrap_or(u64::MAX)));
        };
        if let Some(cap) = limit
            && next > cap
        {
            return Err(make_err(next, cap));
        }
        match counter.compare_exchange_weak(cur, next, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return Ok(next),
            Err(actual) => cur = actual,
        }
    }
}
