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

/// Rounds cumulative presentation endpoints to a fixed destination clock.
///
/// Ties round upward. Zero is not clamped to a playback minimum. The fractional
/// phase is retained as an exact rational, so repeated fractional frames do not
/// drift. Arithmetic and destination-field overflow return an error without
/// changing state. For a failed pixel encode, discard the advanced copy too:
/// copy this value, quantize, submit the frame, then commit the copy on success.
#[derive(Clone, Copy, Debug)]
pub struct TimingQuantizer {
    clock: NonZeroU32,
    max_ticks: u64,
    phase_num: u128,
    phase_den: u128,
    rounded_up: bool,
}

impl TimingQuantizer {
    /// Select the destination ticks per second and maximum ticks per frame.
    pub const fn new(clock: u32, max_ticks: u64) -> Result<Self, TimingError> {
        let Some(clock) = NonZeroU32::new(clock) else {
            return Err(TimingError::ZeroDenominator);
        };
        Ok(Self {
            clock,
            max_ticks,
            phase_num: 0,
            phase_den: 1,
            rounded_up: false,
        })
    }

    /// Return the duration actually to encode and advance only on success.
    ///
    /// Mixed source clocks are supported using bounded u128 phase arithmetic.
    /// If the intermediate common denominator cannot fit, return overflow
    /// without advancing state, even if arbitrary-precision reduction might fit.
    /// A duration exceeding the configured maximum ticks is also rejected.
    pub fn quantize(&mut self, duration: FrameDuration) -> Result<FrameDuration, TimingError> {
        let scaled = u128::from(duration.numerator()) * u128::from(self.clock.get());
        let mut den = u128::from(duration.denominator());
        let whole = scaled / den;
        let mut frac = scaled % den;
        // Reduce in destination ticks before combining phases. In particular,
        // an integral tick duration must not grow the stored denominator.
        let reduce = gcd(frac, den);
        frac /= reduce;
        den /= reduce;
        let common = gcd(self.phase_den, den);
        let left_scale = den / common;
        let right_scale = self.phase_den / common;
        let sum_den = self
            .phase_den
            .checked_mul(left_scale)
            .ok_or(TimingError::Overflow)?;
        let left = self
            .phase_num
            .checked_mul(left_scale)
            .ok_or(TimingError::Overflow)?;
        let right = frac.checked_mul(right_scale).ok_or(TimingError::Overflow)?;
        // Each fraction is below one, but their sum may need 129 bits.
        // Subtract the denominator before adding when the sum carries a tick.
        let (carry, remainder) = if left >= sum_den - right {
            (1, left - (sum_den - right))
        } else {
            (0, left + right)
        };
        // Compare without overflowing on 2 * remainder.
        let rounded_up = remainder >= sum_den - remainder;
        let ticks = whole
            .checked_add(carry)
            .and_then(|n| n.checked_add(u128::from(rounded_up)))
            .and_then(|n| n.checked_sub(u128::from(self.rounded_up)))
            .and_then(|n| u64::try_from(n).ok())
            .filter(|&n| n <= self.max_ticks)
            .ok_or(TimingError::Overflow)?;
        let result = FrameDuration::new(ticks, self.clock.get())?;
        let reduce = gcd(remainder, sum_den);
        self.phase_num = remainder / reduce;
        self.phase_den = sum_den / reduce;
        self.rounded_up = rounded_up;
        Ok(result)
    }
}

fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// Invalid duration construction, destination-clock conversion or quantization.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TimingError {
    /// A duration denominator or destination tick rate is zero.
    ZeroDenominator,
    /// The destination clock cannot represent the duration without rounding.
    Inexact,
    /// The tick count exceeds u64 or the configured maximum, or quantization
    /// exceeds the bounded intermediate phase arithmetic.
    Overflow,
    /// A duration between two instants would be negative: the later one
    /// comes first ([`Timestamp::duration_until`](crate::time::Timestamp::duration_until)).
    Negative,
}
impl fmt::Display for TimingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::ZeroDenominator => "animation duration clock must be positive",
            Self::Inexact => {
                "animation duration is not exactly representable at the destination clock"
            }
            Self::Overflow => "animation duration exceeds the destination tick range",
            Self::Negative => "the later timestamp comes before the earlier one",
        })
    }
}
impl core::error::Error for TimingError {}
