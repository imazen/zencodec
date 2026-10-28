//! Encoder-driven row acquisition without owning the source or its I/O.

use core::fmt;
use zenpixels::PixelSliceMut;

/// A native encoder's temporary destination for one consecutive strip.
///
/// The loan ends when [`EncodeRows::fill`] returns. It cannot survive a pending
/// async read: an asynchronous adapter needs an explicit ownership bridge.
/// Pass ordinals start at zero; every pass covers the complete declared image
/// in row order. A pass label is diagnostic only, never a scheduling authority.
#[non_exhaustive]
pub struct RowDemand<'a> {
    /// Zero-based source traversal, increasing by one between complete passes.
    pub pass: u32,
    /// This is the last traversal. Once true, no later pass may be requested.
    pub final_pass: bool,
    /// Codec-specific diagnostic label; the driver must not interpret its value.
    pub label: u8,
    /// First absolute image row in this strip.
    pub first: u32,
    /// Native-owned strided storage with the declared sample interpretation.
    pub rows: PixelSliceMut<'a>,
}

impl<'a> RowDemand<'a> {
    /// Ask for a strip; final-pass status and diagnostic label start unset.
    pub fn new(pass: u32, first: u32, rows: PixelSliceMut<'a>) -> Self {
        Self {
            pass,
            final_pass: false,
            label: 0,
            first,
            rows,
        }
    }

    /// Identify requests in the final traversal, independently of its label.
    pub const fn with_final_pass(mut self, final_pass: bool) -> Self {
        self.final_pass = final_pass;
        self
    }

    /// Attach a codec diagnostic label without changing pass ordering.
    pub const fn with_label(mut self, label: u8) -> Self {
        self.label = label;
        self
    }
}

/// The driver failed to fill the strip and retains the original typed cause.
///
/// This marker is not the application error. A driver must retain its first
/// source, policy, cancellation or verification failure and return that cause
/// even when a codec translates or swallows this marker. After failure, native
/// code must stop; the driver must not drain newly pending bytes or finish.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowFillError;

impl fmt::Display for RowFillError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("row fill failed; the driver retains the original cause")
    }
}
impl core::error::Error for RowFillError {}

/// Driver-owned row acquisition used by [`crate::encode::Encoder::pull_rows`].
///
/// Fill every active sample or fail. Padding is not a sample. The driver
/// validates pass order, geometry, interpretation and any exact-depth assertion
/// before returning success; errors may leave partially written native storage.
/// This contract is not memory isolation from a codec reading a failed buffer.
pub trait EncodeRows {
    /// Fill the entire strip, retaining the first original error on failure.
    fn fill(&mut self, demand: RowDemand<'_>) -> Result<(), RowFillError>;
}

/// Progress of one encoder-driven row step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PullProgress {
    /// Exactly one fill succeeded. Pending encoded output may now be inspected.
    Filled,
    /// No fill occurred; all declared passes completed. Finish the encoder.
    Done,
}
