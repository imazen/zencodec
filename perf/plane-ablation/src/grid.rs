//! Private sample-center registration model. No rounding, resampling or clipping.
use core::num::NonZeroU64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Error {
    ZeroStep,
    ZeroDenominator,
    Overflow,
}

/// Center of sample i is (origin + i * step) / denominator in the
/// image's reference coordinates. Reference sample (0,0) has center (0,0).
/// Axes remain in stored orientation; orientation is a separate image operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Axis {
    origin: i64,
    step: u64,
    denominator: NonZeroU64,
}
fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}
impl Axis {
    pub(crate) fn new(origin: i64, step: u64, denominator: u64) -> Result<Self, Error> {
        if step == 0 {
            return Err(Error::ZeroStep);
        }
        if denominator == 0 {
            return Err(Error::ZeroDenominator);
        }
        let divisor = gcd(gcd(origin.unsigned_abs(), step), denominator);
        Ok(Self {
            origin: (i128::from(origin) / i128::from(divisor)) as i64,
            step: step / divisor,
            denominator: NonZeroU64::new(denominator / divisor).unwrap(),
        })
    }
    /// Return an exact coordinate pair; consumers must not truncate the fraction.
    pub(crate) fn center(self, index: u64) -> Result<(i128, u64), Error> {
        let numerator = i128::from(index)
            .checked_mul(i128::from(self.step))
            .and_then(|v| v.checked_add(i128::from(self.origin)))
            .ok_or(Error::Overflow)?;
        Ok((numerator, self.denominator.get()))
    }
    /// Borrow a plane starting at `first_sample`, while the image's coordinate
    /// origin moves by `reference_offset`. Neither operation changes sample values.
    pub(crate) fn rebase(self, first_sample: u64, reference_offset: i64) -> Result<Self, Error> {
        let (numerator, denominator) = self.center(first_sample)?;
        let numerator = numerator
            .checked_sub(i128::from(reference_offset) * i128::from(denominator))
            .ok_or(Error::Overflow)?;
        // Normalize before narrowing: a valid result can have a large intermediate.
        let divisor = gcd(
            gcd(
                (numerator.unsigned_abs() % u128::from(denominator)) as u64,
                self.step,
            ),
            denominator,
        );
        let origin = i64::try_from(numerator / i128::from(divisor)).map_err(|_| Error::Overflow)?;
        Self::new(origin, self.step / divisor, denominator / divisor)
    }
}

/// Unknown is information, not a centered/co-sited default. Equal plane sizes
/// do not establish registration. Separable axes cover native planar sampling;
/// this experiment does not promise arbitrary affine transforms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Grid {
    Unknown,
    Axes { x: Axis, y: Axis },
}
impl Grid {
    pub(crate) fn rebase(self, first: (u64, u64), reference: (i64, i64)) -> Result<Self, Error> {
        match self {
            Self::Unknown => Ok(Self::Unknown),
            Self::Axes { x, y } => Ok(Self::Axes {
                x: x.rebase(first.0, reference.0)?,
                y: y.rebase(first.1, reference.1)?,
            }),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn odd_crop_preserves_centered_and_cosited_chroma_phase() {
        // Luma crop starts at source (1,3); chroma view starts at sample (0,1).
        for (origin, expected) in [(1, (-1, -1)), (0, (-2, -2))] {
            let axis = Axis::new(origin, 4, 2).unwrap();
            let Grid::Axes { x, y } = Grid::Axes { x: axis, y: axis }
                .rebase((0, 1), (1, 3))
                .unwrap()
            else {
                panic!()
            };
            for i in 0..10 {
                let (xnum, xden) = x.center(i).unwrap();
                let (ynum, yden) = y.center(i).unwrap();
                assert_eq!(
                    xnum * 2,
                    (i128::from(expected.0) + 4 * i128::from(i)) * i128::from(xden)
                );
                assert_eq!(
                    ynum * 2,
                    (i128::from(expected.1) + 4 * i128::from(i)) * i128::from(yden)
                );
            }
        }
    }
    #[test]
    fn crops_compose_exactly_including_negative_placement() {
        let original = Axis::new(-7, 10, 3).unwrap();
        for a in 0..19 {
            for b in 0..17 {
                let nested = original.rebase(a, 7).unwrap().rebase(b, -13).unwrap();
                let direct = original.rebase(a + b, -6).unwrap();
                assert_eq!(nested, direct);
            }
        }
    }
    #[test]
    fn unknown_does_not_gain_registration_from_a_crop() {
        assert_eq!(Grid::Unknown.rebase((12, 9), (1, -3)), Ok(Grid::Unknown));
    }
    #[test]
    fn normalized_forms_and_limits_are_explicit() {
        assert_eq!(Axis::new(2, 8, 4), Axis::new(1, 4, 2));
        assert_eq!(
            Axis::new(i64::MIN, 2, 2).unwrap().center(0),
            Ok((i128::from(i64::MIN) / 2, 1))
        );
        assert_eq!(Axis::new(0, 0, 1), Err(Error::ZeroStep));
        assert_eq!(Axis::new(0, 1, 0), Err(Error::ZeroDenominator));
        assert_eq!(
            Axis::new(0, u64::MAX, 1).unwrap().center(u64::MAX),
            Err(Error::Overflow)
        );
        assert_eq!(
            Axis::new(i64::MAX, 1, 1).unwrap().rebase(1, 0),
            Err(Error::Overflow)
        );
    }
}
