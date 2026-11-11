//! Resource gates, resource targets, and what a codec expects to use.
//!
//! A gate ([`ResourceLimits`]) is a promise to the caller: the codec stays
//! inside it or refuses with [`ErrorKind::Limit`]. A target
//! ([`ResourceTargets`]) is a preference the codec aims for and reports on.
//! Both arrive in the spec, so they are checked once, before native work.

use crate::error::{CodecError, Error, ErrorKind, LimitKind, Result};
use core::fmt;
use whereat::At;

/// Hard gates. `None` means no gate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ResourceLimits {
    /// Largest image width.
    pub max_width: Option<u32>,
    /// Largest image height.
    pub max_height: Option<u32>,
    /// Most pixels in one frame.
    pub max_pixels: Option<u64>,
    /// Most frames in a sequence.
    pub max_frames: Option<u32>,
    /// Most input bytes a decoder may ask a driver to hold at once.
    pub max_input_bytes: Option<u64>,
    /// Most bytes an encoder may produce, drained bytes included.
    pub max_output_bytes: Option<u64>,
    /// Most working memory the codec may own. A codec that cannot bound its
    /// memory refuses when this is set.
    pub max_memory_bytes: Option<u64>,
}

impl ResourceLimits {
    /// No gates.
    pub const fn none() -> Self {
        Self {
            max_width: None,
            max_height: None,
            max_pixels: None,
            max_frames: None,
            max_input_bytes: None,
            max_output_bytes: None,
            max_memory_bytes: None,
        }
    }

    /// Gate the dimensions of one frame.
    pub const fn with_max_dimensions(mut self, width: u32, height: u32) -> Self {
        self.max_width = Some(width);
        self.max_height = Some(height);
        self
    }

    /// Gate the pixels of one frame.
    pub const fn with_max_pixels(mut self, pixels: u64) -> Self {
        self.max_pixels = Some(pixels);
        self
    }

    /// Gate the frame count.
    pub const fn with_max_frames(mut self, frames: u32) -> Self {
        self.max_frames = Some(frames);
        self
    }

    /// Gate the input a driver holds.
    pub const fn with_max_input_bytes(mut self, bytes: u64) -> Self {
        self.max_input_bytes = Some(bytes);
        self
    }

    /// Gate the encoded size.
    pub const fn with_max_output_bytes(mut self, bytes: u64) -> Self {
        self.max_output_bytes = Some(bytes);
        self
    }

    /// Gate the codec's working memory.
    pub const fn with_max_memory_bytes(mut self, bytes: u64) -> Self {
        self.max_memory_bytes = Some(bytes);
        self
    }

    /// Refuse a frame size the gates do not admit.
    #[track_caller]
    pub fn check_dimensions(&self, width: u32, height: u32) -> Result<()> {
        gate(
            LimitKind::Width,
            self.max_width.map(u64::from),
            width.into(),
        )?;
        gate(
            LimitKind::Height,
            self.max_height.map(u64::from),
            height.into(),
        )?;
        gate(
            LimitKind::Pixels,
            self.max_pixels,
            u64::from(width) * u64::from(height),
        )
    }

    /// Refuse a frame count the gate does not admit.
    #[track_caller]
    pub fn check_frames(&self, frames: u32) -> Result<()> {
        gate(
            LimitKind::Frames,
            self.max_frames.map(u64::from),
            frames.into(),
        )
    }

    /// Refuse working memory the gate does not admit. An unknown bound is
    /// refused whenever a gate is set: the gate is a promise, not a hope.
    #[track_caller]
    pub fn check_memory(&self, bound: Bound) -> Result<()> {
        match (self.max_memory_bytes, bound) {
            (None, _) => Ok(()),
            (Some(limit), Bound::AtMost(bytes)) => gate(LimitKind::Memory, Some(limit), bytes),
            (Some(limit), Bound::Unknown) => Err(limit_error(LimitKind::Memory, limit, None)),
        }
    }

    /// Refuse an output size the gate does not admit.
    #[track_caller]
    pub fn check_output_bytes(&self, bytes: u64) -> Result<()> {
        gate(LimitKind::OutputBytes, self.max_output_bytes, bytes)
    }

    /// Refuse an input size the gate does not admit.
    #[track_caller]
    pub fn check_input_bytes(&self, bytes: u64) -> Result<()> {
        gate(LimitKind::InputBytes, self.max_input_bytes, bytes)
    }
}

#[track_caller]
fn gate(kind: LimitKind, limit: Option<u64>, actual: u64) -> Result<()> {
    match limit {
        Some(limit) if actual > limit => Err(limit_error(kind, limit, Some(actual))),
        _ => Ok(()),
    }
}

#[track_caller]
fn limit_error(kind: LimitKind, limit: u64, actual: Option<u64>) -> Error {
    At::wrap(
        CodecError::new(ErrorKind::Limit(kind)).with_native(LimitExceeded {
            kind,
            limit,
            actual,
        }),
    )
    .at()
}

/// The numbers behind an [`ErrorKind::Limit`]; attached as the cause.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LimitExceeded {
    /// Which gate.
    pub kind: LimitKind,
    /// The gate's value.
    pub limit: u64,
    /// What was needed; `None` when the codec could not bound it.
    pub actual: Option<u64>,
}

impl fmt::Display for LimitExceeded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.actual {
            Some(actual) => write!(f, "{:?} {actual} exceeds limit {}", self.kind, self.limit),
            None => write!(f, "{:?} is unbounded; limit {}", self.kind, self.limit),
        }
    }
}

impl core::error::Error for LimitExceeded {}

/// How many threads a codec may use.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Threads {
    /// The codec's choice.
    #[default]
    Auto,
    /// The calling thread only.
    Serial,
    /// At most this many.
    AtMost(u16),
}

/// Soft targets. A codec aims for them and reports what it chose in its plan.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ResourceTargets {
    /// Thread use.
    pub threads: Threads,
    /// Working memory to aim for, when a slower low-memory path exists.
    pub memory_bytes: Option<u64>,
}

impl ResourceTargets {
    /// The codec's own choices.
    pub const fn auto() -> Self {
        Self {
            threads: Threads::Auto,
            memory_bytes: None,
        }
    }

    /// Aim for this thread use.
    pub const fn with_threads(mut self, threads: Threads) -> Self {
        self.threads = threads;
        self
    }

    /// Aim for this much working memory.
    pub const fn with_memory_bytes(mut self, bytes: u64) -> Self {
        self.memory_bytes = Some(bytes);
        self
    }
}

/// An upper bound, or the admission that there is none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Bound {
    /// The codec cannot bound this.
    #[default]
    Unknown,
    /// No more than this.
    AtMost(u64),
}

/// What a prepared codec expects to use, stated before it does the work.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ResourceEstimate {
    /// Working memory the codec owns at its peak, excluding the caller's buffers.
    pub memory_bytes: Bound,
    /// Encoded size; decoders leave this unknown.
    pub output_bytes: Bound,
    /// Threads it will use.
    pub threads: u16,
}

impl ResourceEstimate {
    /// A single-threaded codec holding at most `memory_bytes`.
    pub const fn serial(memory_bytes: Bound) -> Self {
        Self {
            memory_bytes,
            output_bytes: Bound::Unknown,
            threads: 1,
        }
    }

    /// State the encoded-size bound.
    pub const fn with_output_bytes(mut self, bound: Bound) -> Self {
        self.output_bytes = bound;
        self
    }
}
