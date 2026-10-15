//! Media time: instants on a clock, next to the durations in
//! [`animation`](crate::animation).
//!
//! One model, two types, one rounding rule:
//!
//! - [`FrameDuration`] is a nonnegative rational number of seconds: how long
//!   something lasts.
//! - [`Timestamp`] is a signed instant, `ticks` of a [`TimeBase`]: when
//!   something is presented or decoded. Video and audio streams carry these;
//!   a missing one is `Option<Timestamp>`, never a sentinel value.
//! - Every conversion that can lose precision takes an explicit [`Rounding`].
//!   [`Rounding::Nearest`] rounds exact halves toward +infinity, the rule
//!   [`TimingQuantizer`] applies to cumulative endpoints, so quantizing a run
//!   of durations and rescaling the instants between them agree.
//!
//! ```
//! use zencodec::time::{Rounding, TimeBase, Timestamp, duration_to_ticks};
//! use zencodec::animation::FrameDuration;
//!
//! let ntsc = TimeBase::new(1001, 30000).unwrap(); // one tick per NTSC frame
//! let frame_3 = Timestamp::new(3, ntsc);
//! let ms = frame_3.rescale(TimeBase::MILLIS, Rounding::Nearest).unwrap();
//! assert_eq!(ms.ticks(), 100); // 100.1 ms
//! let one_frame = FrameDuration::new(1001, 30000).unwrap();
//! assert_eq!(duration_to_ticks(one_frame, ntsc), Ok(1));
//! assert!(duration_to_ticks(one_frame, TimeBase::MILLIS).is_err()); // inexact
//! ```

use core::cmp::Ordering;
use core::num::NonZeroU32;

pub use crate::animation::{FrameDuration, TimingError, TimingQuantizer};

/// Seconds per tick, as a reduced fraction `numerator / denominator`.
///
/// `TimeBase::new(1, 90000)` is the 90 kHz MPEG clock; `TimeBase::new(1001,
/// 30000)` ticks once per NTSC frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TimeBase {
    num: NonZeroU32,
    den: NonZeroU32,
}

const fn gcd32(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

impl TimeBase {
    /// One millisecond per tick.
    pub const MILLIS: Self = match Self::new(1, 1000) {
        Ok(t) => t,
        Err(_) => unreachable!(),
    };

    /// One second per tick.
    pub const SECONDS: Self = match Self::new(1, 1) {
        Ok(t) => t,
        Err(_) => unreachable!(),
    };

    /// `numerator / denominator` seconds per tick, reduced. Both must be
    /// positive ([`TimingError::ZeroDenominator`] otherwise).
    pub const fn new(numerator: u32, denominator: u32) -> Result<Self, TimingError> {
        if numerator == 0 || denominator == 0 {
            return Err(TimingError::ZeroDenominator);
        }
        let g = gcd32(numerator, denominator);
        match (
            NonZeroU32::new(numerator / g),
            NonZeroU32::new(denominator / g),
        ) {
            (Some(num), Some(den)) => Ok(Self { num, den }),
            _ => Err(TimingError::ZeroDenominator),
        }
    }

    /// A clock of `ticks_per_second` ticks per second.
    pub const fn hz(ticks_per_second: u32) -> Result<Self, TimingError> {
        Self::new(1, ticks_per_second)
    }

    /// Numerator of seconds per tick.
    pub const fn numerator(self) -> u32 {
        self.num.get()
    }

    /// Denominator of seconds per tick.
    pub const fn denominator(self) -> u32 {
        self.den.get()
    }
}

/// How a conversion settles a fractional tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Rounding {
    /// Toward negative infinity.
    Floor,
    /// Toward positive infinity.
    Ceil,
    /// To the nearest tick; exact halves round toward positive infinity, the
    /// rule [`TimingQuantizer`] applies.
    Nearest,
}

/// A signed instant: `ticks` of a [`TimeBase`].
///
/// Negative instants are ordinary (pre-roll, edit lists). Derived equality
/// compares the representation (`3 @ 1/1000` differs from `30 @ 1/10000`);
/// [`compare`](Self::compare) and [`same_instant`](Self::same_instant)
/// compare physical time across bases, exactly.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Timestamp {
    ticks: i64,
    base: TimeBase,
}

impl Timestamp {
    /// `ticks` of `base`.
    pub const fn new(ticks: i64, base: TimeBase) -> Self {
        Self { ticks, base }
    }

    /// The tick count.
    pub const fn ticks(self) -> i64 {
        self.ticks
    }

    /// The clock.
    pub const fn base(self) -> TimeBase {
        self.base
    }

    /// Exact physical ordering, whatever the bases and signs.
    pub fn compare(self, other: Self) -> Ordering {
        // i64 * u32 * u32 fits in i128.
        let l = i128::from(self.ticks)
            * i128::from(self.base.numerator())
            * i128::from(other.base.denominator());
        let r = i128::from(other.ticks)
            * i128::from(other.base.numerator())
            * i128::from(self.base.denominator());
        l.cmp(&r)
    }

    /// Whether both name the same instant, whatever their bases.
    pub fn same_instant(self, other: Self) -> bool {
        self.compare(other) == Ordering::Equal
    }

    /// The same instant on `target`, rounded as `rounding` says.
    ///
    /// # Errors
    ///
    /// [`TimingError::Overflow`] when the result does not fit `i64` ticks.
    pub fn rescale(self, target: TimeBase, rounding: Rounding) -> Result<Self, TimingError> {
        let num = i128::from(self.ticks)
            * i128::from(self.base.numerator())
            * i128::from(target.denominator());
        let den = i128::from(self.base.denominator()) * i128::from(target.numerator());
        let floor = num.div_euclid(den);
        let rem = num.rem_euclid(den);
        let up = match rounding {
            Rounding::Floor => false,
            Rounding::Ceil => rem != 0,
            Rounding::Nearest => rem * 2 >= den,
        };
        let ticks = i64::try_from(floor + i128::from(up)).map_err(|_| TimingError::Overflow)?;
        Ok(Self::new(ticks, target))
    }

    /// This instant moved later by `duration`, which must be a whole number
    /// of ticks of this base.
    ///
    /// # Errors
    ///
    /// [`TimingError::Inexact`] when it is not; [`TimingError::Overflow`]
    /// when the result does not fit `i64` ticks.
    pub fn checked_add(self, duration: FrameDuration) -> Result<Self, TimingError> {
        let ticks = duration_to_ticks(duration, self.base)?;
        let sum = i128::from(self.ticks) + i128::from(ticks);
        let ticks = i64::try_from(sum).map_err(|_| TimingError::Overflow)?;
        Ok(Self::new(ticks, self.base))
    }

    /// The exact time from this instant to `later`, whatever their bases.
    ///
    /// # Errors
    ///
    /// [`TimingError::Negative`] when `later` comes first;
    /// [`TimingError::Overflow`] when the reduced duration does not fit
    /// [`FrameDuration`].
    pub fn duration_until(self, later: Self) -> Result<FrameDuration, TimingError> {
        let num = i128::from(later.ticks)
            * i128::from(later.base.numerator())
            * i128::from(self.base.denominator())
            - i128::from(self.ticks)
                * i128::from(self.base.numerator())
                * i128::from(later.base.denominator());
        if num < 0 {
            return Err(TimingError::Negative);
        }
        let num = num.unsigned_abs();
        let den = u128::from(later.base.denominator()) * u128::from(self.base.denominator());
        let g = gcd128(num, den).max(1);
        let num = u64::try_from(num / g).map_err(|_| TimingError::Overflow)?;
        let den = u32::try_from(den / g).map_err(|_| TimingError::Overflow)?;
        FrameDuration::new(num, den)
    }
}

fn gcd128(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// The whole number of `base` ticks in `duration`.
///
/// # Errors
///
/// [`TimingError::Inexact`] when `duration` is not a whole number of ticks;
/// [`TimingError::Overflow`] when the count does not fit `u64`.
pub fn duration_to_ticks(duration: FrameDuration, base: TimeBase) -> Result<u64, TimingError> {
    let n = u128::from(duration.numerator()) * u128::from(base.denominator());
    let d = u128::from(duration.denominator()) * u128::from(base.numerator());
    if !n.is_multiple_of(d) {
        return Err(TimingError::Inexact);
    }
    u64::try_from(n / d).map_err(|_| TimingError::Overflow)
}

/// The exact duration of `ticks` ticks of `base`.
///
/// # Errors
///
/// [`TimingError::Overflow`] when the reduced duration does not fit
/// [`FrameDuration`].
pub fn ticks_to_duration(ticks: u64, base: TimeBase) -> Result<FrameDuration, TimingError> {
    let num = u128::from(ticks) * u128::from(base.numerator());
    let den = u128::from(base.denominator());
    let g = gcd128(num, den).max(1);
    let num = u64::try_from(num / g).map_err(|_| TimingError::Overflow)?;
    // `den / g` divides a u32, so it fits.
    FrameDuration::new(num, (den / g) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tb(n: u32, d: u32) -> TimeBase {
        TimeBase::new(n, d).unwrap()
    }

    #[test]
    fn time_bases_reduce_and_refuse_zero() {
        assert_eq!(tb(2, 2000), TimeBase::MILLIS);
        assert_eq!(TimeBase::new(0, 5), Err(TimingError::ZeroDenominator));
        assert_eq!(TimeBase::new(5, 0), Err(TimingError::ZeroDenominator));
        assert_eq!(TimeBase::hz(90_000).unwrap(), tb(1, 90_000));
    }

    #[test]
    fn nearest_ties_round_toward_plus_infinity_for_both_signs() {
        let half_ms = tb(1, 2000);
        for (ticks, want) in [(1, 1), (3, 2), (-1, 0), (-3, -1), (2, 1), (-2, -1)] {
            let got = Timestamp::new(ticks, half_ms)
                .rescale(TimeBase::MILLIS, Rounding::Nearest)
                .unwrap();
            assert_eq!(got.ticks(), want, "{ticks} half-ms");
        }
        let t = Timestamp::new(-3, half_ms);
        assert_eq!(
            t.rescale(TimeBase::MILLIS, Rounding::Floor)
                .unwrap()
                .ticks(),
            -2
        );
        assert_eq!(
            t.rescale(TimeBase::MILLIS, Rounding::Ceil).unwrap().ticks(),
            -1
        );
    }

    #[test]
    fn comparison_is_exact_across_bases() {
        let a = Timestamp::new(3, TimeBase::MILLIS);
        let b = Timestamp::new(30, tb(1, 10_000));
        assert_ne!(a, b);
        assert!(a.same_instant(b));
        let ntsc = tb(1001, 30000);
        assert_eq!(
            Timestamp::new(30, ntsc).compare(Timestamp::new(1001, TimeBase::MILLIS)),
            Ordering::Equal
        );
        assert_eq!(
            Timestamp::new(-1, ntsc).compare(Timestamp::new(0, TimeBase::SECONDS)),
            Ordering::Less
        );
    }

    #[test]
    fn durations_and_ticks_convert_exactly_or_refuse() {
        let ntsc = tb(1001, 30000);
        let frame = FrameDuration::new(1001, 30000).unwrap();
        assert_eq!(duration_to_ticks(frame, ntsc), Ok(1));
        assert_eq!(duration_to_ticks(frame, tb(1, 90_000)), Ok(3003));
        assert_eq!(
            duration_to_ticks(frame, TimeBase::MILLIS),
            Err(TimingError::Inexact)
        );
        assert_eq!(ticks_to_duration(3003, tb(1, 90_000)), Ok(frame));
        assert_eq!(
            ticks_to_duration(u64::MAX, tb(u32::MAX, 1)),
            Err(TimingError::Overflow)
        );
        let start = Timestamp::new(-2, ntsc);
        let end = Timestamp::new(1001, TimeBase::MILLIS);
        assert_eq!(
            start.duration_until(end),
            FrameDuration::new(32 * 1001, 30000)
        );
        assert_eq!(end.duration_until(start), Err(TimingError::Negative));
        assert_eq!(start.checked_add(frame).unwrap(), Timestamp::new(-1, ntsc));
        assert_eq!(
            Timestamp::new(i64::MAX, ntsc).checked_add(frame),
            Err(TimingError::Overflow)
        );
    }

    #[test]
    fn rescale_overflow_is_an_error() {
        let t = Timestamp::new(i64::MAX, TimeBase::SECONDS);
        assert_eq!(
            t.rescale(TimeBase::MILLIS, Rounding::Nearest),
            Err(TimingError::Overflow)
        );
    }

    /// Quantizing a run of durations and rescaling the instants between
    /// them use one rule: every cumulative endpoint agrees.
    #[test]
    fn cumulative_endpoints_agree_between_quantizer_and_rescale() {
        let frame = FrameDuration::new(1001, 30000).unwrap();
        let ntsc = tb(1001, 30000);
        let mut q = TimingQuantizer::new(1000, u64::MAX).unwrap();
        let mut sum_ms: u64 = 0;
        for i in 1..=300i64 {
            let ms = q.quantize(frame).unwrap();
            sum_ms += ms.ticks_at(1000).unwrap();
            let rescaled = Timestamp::new(i, ntsc)
                .rescale(TimeBase::MILLIS, Rounding::Nearest)
                .unwrap();
            assert_eq!(sum_ms as i64, rescaled.ticks(), "endpoint {i}");
        }
    }
}
