//! Scalar f64 equations. Production code must not call this reference module.
//! The checked-in Decimal vectors provide a second, independent arithmetic path.

/// Unbounded source-encoded RGB from BT.2100-style offset-binary YCbCr.
/// `kr`/`kb` select a nonconstant-luminance matrix; this is not ICtCp or CL.
pub fn ycbcr_to_rgb(codes: [f64; 3], bits: u8, full: bool, kr: f64, kb: f64) -> [f64; 3] {
    assert!((8..=16).contains(&bits));
    assert!(kr > 0.0 && kb > 0.0 && kr + kb < 1.0);
    let scale = f64::from(1_u32 << (bits - 8));
    let max = f64::from((1_u32 << bits) - 1);
    let neutral = f64::from(1_u32 << (bits - 1));
    let (y, cb, cr) = if full {
        (
            codes[0] / max,
            (codes[1] - neutral) / max,
            (codes[2] - neutral) / max,
        )
    } else {
        (
            (codes[0] - 16.0 * scale) / (219.0 * scale),
            (codes[1] - neutral) / (224.0 * scale),
            (codes[2] - neutral) / (224.0 * scale),
        )
    };
    let r = y + 2.0 * (1.0 - kr) * cr;
    let b = y + 2.0 * (1.0 - kb) * cb;
    [r, (y - kr * r - kb * b) / (1.0 - kr - kb), b]
}

/// sRGB inverse transfer, extended by odd symmetry for out-of-gamut values.
pub fn srgb_to_linear(value: f64) -> f64 {
    let x = value.abs();
    let linear = if x <= 0.04045 {
        x / 12.92
    } else {
        ((x + 0.055) / 1.055).powf(2.4)
    };
    linear.copysign(value)
}

/// sRGB forward transfer with the same signed extension.
pub fn linear_to_srgb(value: f64) -> f64 {
    let x = value.abs();
    let encoded = if x <= 0.0031308 {
        12.92 * x
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    };
    encoded.copysign(value)
}

/// PQ EOTF: normalized [0,1] signal to absolute display nits.
pub fn pq_to_nits(encoded: f64) -> f64 {
    assert!((0.0..=1.0).contains(&encoded));
    let power = encoded.powf(32.0 / 2523.0);
    10000.0
        * ((power - 3424.0 / 4096.0).max(0.0) / (2413.0 / 128.0 - 2392.0 / 128.0 * power))
            .powf(16384.0 / 2610.0)
}

/// HLG inverse OETF: scene-relative light, not display nits.
pub fn hlg_to_scene(encoded: f64) -> f64 {
    assert!((0.0..=1.0).contains(&encoded));
    let a: f64 = 0.17883277;
    let b = 1.0 - 4.0 * a;
    let c = 0.5 - a * (4.0 * a).ln();
    if encoded <= 0.5 {
        encoded * encoded / 3.0
    } else {
        (((encoded - c) / a).exp() + b) / 12.0
    }
}

/// Straight-alpha source-over in a caller-selected color domain.
/// The format adapter chooses encoded-space versus linear-space composition.
pub fn source_over(source: [f64; 4], destination: [f64; 4]) -> [f64; 4] {
    let alpha = source[3] + destination[3] * (1.0 - source[3]);
    if alpha == 0.0 {
        return [0.0; 4];
    }
    let mut result = [0.0; 4];
    for channel in 0..3 {
        result[channel] = (source[channel] * source[3]
            + destination[channel] * destination[3] * (1.0 - source[3]))
            / alpha;
    }
    result[3] = alpha;
    result
}
