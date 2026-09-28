//! Exact media time. No frame-rate guesses or floating-point seek arithmetic.

use std::cmp::Ordering;
use std::fmt;
use std::num::NonZeroU32;

/// Seconds per tick, stored as a reduced positive rational.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TimeBase {
    numerator: NonZeroU32,
    denominator: NonZeroU32,
}

impl TimeBase {
    pub fn new(numerator: u32, denominator: u32) -> Result<Self, TimeError> {
        if numerator == 0 || denominator == 0 {
            return Err(TimeError::ZeroTimeBase);
        }
        let mut a = numerator;
        let mut b = denominator;
        while b != 0 {
            (a, b) = (b, a % b);
        }
        Ok(Self {
            numerator: NonZeroU32::new(numerator / a).unwrap(),
            denominator: NonZeroU32::new(denominator / a).unwrap(),
        })
    }

    pub fn numerator(self) -> u32 {
        self.numerator.get()
    }
    pub fn denominator(self) -> u32 {
        self.denominator.get()
    }
}

/// An explicitly known signed presentation or decode time.
/// Missing timestamps are represented by `Option<Timestamp>`, never a sentinel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Timestamp {
    ticks: i64,
    time_base: TimeBase,
}

impl Timestamp {
    pub fn new(ticks: i64, time_base: TimeBase) -> Self {
        Self { ticks, time_base }
    }
    pub fn ticks(self) -> i64 {
        self.ticks
    }
    pub fn time_base(self) -> TimeBase {
        self.time_base
    }

    /// Compare physical time exactly, including negative timestamps and mixed clocks.
    /// Equal physical times can have different tick representations (`PartialEq`).
    pub fn compare(self, other: Self) -> Ordering {
        // i64 * u32 * u32 fits i128 even at all input extrema.
        let left = i128::from(self.ticks)
            * i128::from(self.time_base.numerator())
            * i128::from(other.time_base.denominator());
        let right = i128::from(other.ticks)
            * i128::from(other.time_base.numerator())
            * i128::from(self.time_base.denominator());
        left.cmp(&right)
    }

    pub fn rescale(self, target: TimeBase, rounding: Rounding) -> Result<Self, TimeError> {
        let numerator = i128::from(self.ticks)
            * i128::from(self.time_base.numerator())
            * i128::from(target.denominator());
        let denominator = i128::from(self.time_base.denominator()) * i128::from(target.numerator());
        let floor = numerator.div_euclid(denominator);
        let remainder = numerator.rem_euclid(denominator);
        let up = match rounding {
            Rounding::Floor => false,
            Rounding::Ceil => remainder != 0,
            // Ties select the earlier time, including for negative input.
            Rounding::Nearest => remainder * 2 > denominator,
        };
        let ticks = i64::try_from(floor + i128::from(up)).map_err(|_| TimeError::Overflow)?;
        Ok(Self::new(ticks, target))
    }

    /// Add a nonnegative duration expressed in this timestamp's own ticks.
    pub fn checked_add_ticks(self, duration: u64) -> Result<Self, TimeError> {
        let ticks = i64::try_from(i128::from(self.ticks) + i128::from(duration))
            .map_err(|_| TimeError::Overflow)?;
        Ok(Self::new(ticks, self.time_base))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rounding {
    Floor,
    Ceil,
    Nearest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TimeError {
    ZeroTimeBase,
    Overflow,
}

impl fmt::Display for TimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::ZeroTimeBase => "time base numerator and denominator must be positive",
            Self::Overflow => "timestamp does not fit the destination clock",
        })
    }
}
impl std::error::Error for TimeError {}
