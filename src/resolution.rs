//! Checked physical density and unknown-unit pixel aspect.
use crate::{Resolution, ResolutionUnit};
use core::fmt;

/// Why a required density cannot be configured. A rejected mutable setter must
/// leave the job unchanged; the caller may then make a different explicit request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ResolutionError {
    /// Both axes must be finite and strictly positive.
    InvalidValue,
    /// Physical units and an unknown-unit aspect ratio cannot replace each other.
    UnitMismatch,
    /// The carrier cannot represent this value exactly within its integer range.
    NotRepresentable,
    /// This job has no qualified required-resolution implementation.
    Unsupported,
}
impl fmt::Display for ResolutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidValue => "resolution axes must be finite and positive",
            Self::UnitMismatch => "physical density and unknown-unit aspect are distinct",
            Self::NotRepresentable => "resolution is not exactly representable by this carrier",
            Self::Unsupported => "required resolution is unsupported by this encoder job",
        })
    }
}
impl core::error::Error for ResolutionError {}

impl Resolution {
    /// Validate a required physical density or unknown-unit aspect ratio.
    /// This does not establish that any particular container can represent it.
    pub fn validate(&self) -> Result<(), ResolutionError> {
        if self.x.is_finite() && self.y.is_finite() && self.x > 0.0 && self.y > 0.0 {
            Ok(())
        } else {
            Err(ResolutionError::InvalidValue)
        }
    }

    /// Compare physical density across units, or pixel aspect across two
    /// unknown-unit values. Invalid values never compare equivalent.
    ///
    /// This compares the represented binary values and exact unit ratios, with
    /// no rounding or epsilon. Unknown-unit `(2, 1)` and `(1000, 500)` are
    /// equivalent; neither establishes a physical size. It does not account
    /// for a pixel transform: an axis-swapping transform must swap X and Y too.
    pub fn is_equivalent_to(&self, other: &Self) -> bool {
        if self.validate().is_err() || other.validate().is_err() {
            return false;
        }
        match (factor(self.unit), factor(other.unit)) {
            (None, None) => product(self.x, other.y) == product(other.x, self.y),
            (Some((an, ad)), Some((bn, bd))) => {
                scaled(self.x, an * bd) == scaled(other.x, bn * ad)
                    && scaled(self.y, an * bd) == scaled(other.y, bn * ad)
            }
            _ => false,
        }
    }

    /// Convert exactly to a carrier's positive integer density fields.
    ///
    /// Adapter helper for such carriers as PNG pHYs and JPEG JFIF. `max` is the
    /// largest permitted value on either axis. No truncation, clamping or
    /// rounding is allowed. Unknown-unit inputs are reduced to an equivalent
    /// integer ratio; their absolute magnitudes have no physical meaning.
    /// Conversion between physical and unknown units refuses.
    pub fn try_integer_density(
        &self,
        unit: ResolutionUnit,
        max: u32,
    ) -> Result<(u32, u32), ResolutionError> {
        self.validate()?;
        match (factor(self.unit), factor(unit)) {
            (None, None) => integer_ratio(self.x, self.y, max),
            (Some((sn, sd)), Some((tn, td))) => Ok((
                integer(self.x, sn * td, sd * tn, max)?,
                integer(self.y, sn * td, sd * tn, max)?,
            )),
            _ => Err(ResolutionError::UnitMismatch),
        }
    }
}

// Exact pixels-per-metre factors: one inch is exactly 127/5000 metre.
fn factor(unit: ResolutionUnit) -> Option<(u32, u32)> {
    match unit {
        ResolutionUnit::Inch => Some((5000, 127)),
        ResolutionUnit::Centimeter => Some((100, 1)),
        ResolutionUnit::Meter => Some((1, 1)),
        ResolutionUnit::Unknown => None,
    }
}

// The caller has checked finite, positive input. Products use at most 106 bits
// for two binary64 significands, or 73 bits for either unit cross-product.
fn parts(value: f64) -> (u128, i32) {
    let bits = value.to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i32;
    let fraction = bits & ((1u64 << 52) - 1);
    if exponent == 0 {
        (u128::from(fraction), -1074)
    } else {
        (u128::from(fraction | (1u64 << 52)), exponent - 1023 - 52)
    }
}
fn normalized(mantissa: u128, exponent: i32) -> (u128, i32) {
    let zeros = mantissa.trailing_zeros();
    (mantissa >> zeros, exponent + zeros as i32)
}
fn scaled(value: f64, factor: u32) -> (u128, i32) {
    let (m, e) = parts(value);
    normalized(m * u128::from(factor), e)
}
fn product(a: f64, b: f64) -> (u128, i32) {
    let (am, ae) = parts(a);
    let (bm, be) = parts(b);
    normalized(am * bm, ae + be)
}
fn integer(value: f64, n: u32, d: u32, max: u32) -> Result<u32, ResolutionError> {
    // A floating estimate only locates candidates. Exact integer comparisons
    // below are the authority, even if division rounded onto an integer.
    let candidate = (value * f64::from(n) / f64::from(d)) as u32;
    for c in [
        candidate,
        candidate.saturating_add(1),
        candidate.saturating_sub(1),
    ] {
        if c != 0 && c <= max && scaled(value, n) == scaled(f64::from(c), d) {
            return Ok(c);
        }
    }
    Err(ResolutionError::NotRepresentable)
}
fn integer_ratio(x: f64, y: f64, max: u32) -> Result<(u32, u32), ResolutionError> {
    let (xm, xe) = scaled(x, 1);
    let (ym, ye) = scaled(y, 1);
    let (mut a, mut b) = (xm, ym);
    while b != 0 {
        (a, b) = (b, a % b);
    }
    let (mut x, mut y) = (xm / a, ym / a);
    let shift = xe - ye;
    if shift.unsigned_abs() >= 32 {
        return Err(ResolutionError::NotRepresentable);
    }
    if shift >= 0 {
        x <<= shift;
    } else {
        y <<= -shift;
    }
    if x > u128::from(max) || y > u128::from(max) {
        return Err(ResolutionError::NotRepresentable);
    }
    Ok((x as u32, y as u32))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn r(x: f64, y: f64, unit: ResolutionUnit) -> Resolution {
        Resolution { x, y, unit }
    }
    #[test]
    fn exact_units_and_aspect_do_not_conflate_quantization_or_physical_size() {
        use ResolutionUnit::*;
        let inches = r(254.0, 127.0, Inch);
        let metres = r(10000.0, 5000.0, Meter);
        let centimetres = r(100.0, 50.0, Centimeter);
        for a in [inches, metres, centimetres] {
            for b in [inches, metres, centimetres] {
                assert!(a.is_equivalent_to(&b));
            }
            assert_eq!(a.try_integer_density(Meter, 10000), Ok((10000, 5000)));
            assert_eq!(a.try_integer_density(Inch, 254), Ok((254, 127)));
            assert_eq!(a.try_integer_density(Centimeter, 100), Ok((100, 50)));
            assert_eq!(
                a.try_integer_density(Unknown, u32::MAX),
                Err(ResolutionError::UnitMismatch)
            );
            assert!(!a.is_equivalent_to(&r(2.0, 1.0, Unknown)));
        }
        let dpi = r(300.0, 150.0, Inch);
        assert_eq!(
            dpi.try_integer_density(Meter, u32::MAX),
            Err(ResolutionError::NotRepresentable)
        );
        assert!(!dpi.is_equivalent_to(&r(11811.0, 5906.0, Meter)));
        for a in [
            r(1000.0, 500.0, Unknown),
            r(0.5, 0.25, Unknown),
            r(f64::MAX, f64::MAX / 2.0, Unknown),
        ] {
            assert_eq!(a.try_integer_density(Unknown, 2), Ok((2, 1)));
            assert!(a.is_equivalent_to(&r(2.0, 1.0, Unknown)));
        }
    }
    #[test]
    fn rounded_products_extremes_and_invalid_numbers_cannot_claim_exactness() {
        use ResolutionUnit::*;
        for v in [0.0, -0.0, -1.0, f64::INFINITY, f64::NEG_INFINITY, f64::NAN] {
            for a in [r(v, 1.0, Inch), r(1.0, v, Unknown)] {
                assert_eq!(a.validate(), Err(ResolutionError::InvalidValue));
                assert!(!a.is_equivalent_to(&a));
                assert_eq!(
                    a.try_integer_density(Meter, u32::MAX),
                    Err(ResolutionError::InvalidValue)
                );
            }
        }
        for v in [f64::from_bits(1), f64::MIN_POSITIVE, f64::MAX] {
            let a = r(v, v, Meter);
            assert!(a.is_equivalent_to(&a));
            assert_eq!(
                a.try_integer_density(Meter, u32::MAX),
                Err(ResolutionError::NotRepresentable)
            );
            assert_eq!(r(v, v, Unknown).try_integer_density(Unknown, 1), Ok((1, 1)));
        }
        assert_eq!(
            r(f64::MAX, f64::MIN_POSITIVE, Unknown).try_integer_density(Unknown, u32::MAX),
            Err(ResolutionError::NotRepresentable)
        );
        let max = r(f64::from(u32::MAX), 1.0, Meter);
        assert_eq!(max.try_integer_density(Meter, u32::MAX), Ok((u32::MAX, 1)));
        assert_eq!(
            max.try_integer_density(Meter, u32::MAX - 1),
            Err(ResolutionError::NotRepresentable)
        );
        for v in [
            f64::from_bits(300f64.to_bits() - 1),
            f64::from_bits(300f64.to_bits() + 1),
        ] {
            assert!(!r(v, 150.0, Inch).is_equivalent_to(&r(300.0, 150.0, Inch)));
            assert_eq!(
                r(v, 150.0, Inch).try_integer_density(Inch, u32::MAX),
                Err(ResolutionError::NotRepresentable)
            );
        }
    }
}
