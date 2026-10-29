//! Scalar metadata math; the std feature retains the existing platform path.

#[inline]
pub(super) fn pow2(value: f64) -> f64 {
    #[cfg(feature = "std")]
    {
        2.0f64.powf(value)
    }
    #[cfg(not(feature = "std"))]
    {
        libm::pow(2.0, value)
    }
}

#[inline]
pub(super) fn floor(value: f64) -> f64 {
    #[cfg(feature = "std")]
    {
        value.floor()
    }
    #[cfg(not(feature = "std"))]
    {
        libm::floor(value)
    }
}

#[inline]
pub(super) fn round(value: f64) -> f64 {
    #[cfg(feature = "std")]
    {
        value.round()
    }
    #[cfg(not(feature = "std"))]
    {
        libm::round(value)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn portable_rounding_matches_platform_bits() {
        for value in [
            f64::NEG_INFINITY,
            -0.0,
            0.0,
            f64::MIN_POSITIVE,
            -f64::MIN_POSITIVE,
            f64::MAX,
            -f64::MAX,
            f64::INFINITY,
        ] {
            assert_eq!(libm::floor(value).to_bits(), value.floor().to_bits());
            assert_eq!(libm::round(value).to_bits(), value.round().to_bits());
        }
        let mut bits = 0x51d734f03e991b65u64;
        for _ in 0..16384 {
            bits ^= bits << 13;
            bits ^= bits >> 7;
            bits ^= bits << 17;
            let value = f64::from_bits(bits);
            if value.is_nan() {
                continue;
            } // NaN payload propagation is unspecified.
            assert_eq!(libm::floor(value).to_bits(), value.floor().to_bits());
            assert_eq!(libm::round(value).to_bits(), value.round().to_bits());
        }
        for n in -1024..=1024 {
            let value = f64::from(n) * 0.5;
            assert_eq!(libm::round(value).to_bits(), value.round().to_bits());
        }
    }

    #[test]
    fn portable_integer_headroom_is_exact_including_subnormals() {
        for exponent in -1074..=1023 {
            let expected = if exponent < -1022 {
                f64::from_bits(1 << (exponent + 1074))
            } else {
                f64::from_bits(((exponent + 1023) as u64) << 52)
            };
            assert_eq!(
                libm::pow(2.0, f64::from(exponent)).to_bits(),
                expected.to_bits()
            );
            assert_eq!(
                super::pow2(f64::from(exponent)).to_bits(),
                expected.to_bits()
            );
        }
        assert_eq!(super::pow2(-1075.0), 0.0);
        assert_eq!(super::pow2(1024.0), f64::INFINITY);
    }
}
