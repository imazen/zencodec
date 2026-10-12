//! Exact animation timing. Transport source values without a browser playback
//! clamp; zero-duration display policy and format quantization belong to callers.
use core::{fmt, num::NonZeroU32};

/// A nonnegative rational number of seconds, reduced to canonical form.
///
/// This is the encoded source duration, not an inferred frame rate. Zero remains
/// zero. It does not imply that a player will show a frame for zero wall-clock
/// time; callers choose any playback minimum explicitly. A u64 numerator retains
/// native JXL ticks multiplied by its u32 clock numerator without truncation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FrameDuration {
    numerator: u64,
    denominator: NonZeroU32,
}
impl FrameDuration {
    /// Construct a reduced duration. The denominator must be positive.
    pub const fn new(numerator: u64, denominator: u32) -> Result<Self, TimingError> {
        if denominator == 0 {
            return Err(TimingError::ZeroDenominator);
        }
        let (mut a, mut b) = (numerator, denominator as u64);
        while b != 0 {
            (a, b) = (b, a % b);
        }
        Ok(Self {
            numerator: numerator / a,
            denominator: NonZeroU32::new((denominator as u64 / a) as u32).unwrap(),
        })
    }
    /// Construct a duration from the legacy whole-millisecond representation.
    pub const fn from_millis(milliseconds: u32) -> Self {
        match Self::new(milliseconds as u64, 1000) {
            Ok(value) => value,
            Err(_) => unreachable!(),
        }
    }
    /// Numerator of the reduced duration in seconds.
    pub const fn numerator(self) -> u64 {
        self.numerator
    }
    /// Positive denominator of the reduced duration in seconds.
    pub const fn denominator(self) -> u32 {
        self.denominator.get()
    }

    /// Exact ticks at a destination clock. No rounding, minimum-duration clamp,
    /// saturation or narrowing is performed. A codec must also check its own
    /// tick-field width before accepting the frame.
    pub fn ticks_at(self, ticks_per_second: u32) -> Result<u64, TimingError> {
        if ticks_per_second == 0 {
            return Err(TimingError::ZeroDenominator);
        }
        let scaled = u128::from(self.numerator) * u128::from(ticks_per_second);
        let divisor = u128::from(self.denominator.get());
        if !scaled.is_multiple_of(divisor) {
            return Err(TimingError::Inexact);
        }
        u64::try_from(scaled / divisor).map_err(|_| TimingError::Overflow)
    }

    /// Compatibility projection used by the legacy millisecond frame accessor.
    /// Sub-millisecond fractions are truncated; values above u32::MAX saturate.
    /// Exact transcodes must use the rational duration instead.
    pub(crate) fn legacy_millis(self) -> u32 {
        ((u128::from(self.numerator) * 1000 / u128::from(self.denominator.get()))
            .min(u128::from(u32::MAX))) as u32
    }
}

/// Invalid duration construction or an exact destination-clock conversion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TimingError {
    /// A duration denominator or destination tick rate is zero.
    ZeroDenominator,
    /// The destination clock cannot represent the duration without rounding.
    Inexact,
    /// The exact destination tick count exceeds u64.
    Overflow,
}
impl fmt::Display for TimingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::ZeroDenominator => "animation duration clock must be positive",
            Self::Inexact => {
                "animation duration is not exactly representable at the destination clock"
            }
            Self::Overflow => "animation duration exceeds the destination tick range",
        })
    }
}
impl core::error::Error for TimingError {}
