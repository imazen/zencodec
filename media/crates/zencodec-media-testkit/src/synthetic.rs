//! Original deterministic source signal, independent of codec output.

/// Integer code pattern used by the native lossless corpus. Endpoints at the
/// top left alternate each presentation; other pixels exercise all 257 levels.
pub fn code(frame: usize, component: usize, x: usize, y: usize, bits: u8, full: bool) -> u16 {
    assert!((8..=16).contains(&bits));
    let scale = 1_u32 << (bits - 8);
    let (low, high) = if full {
        (0, (1_u32 << bits) - 1)
    } else if component == 0 {
        (16 * scale, 235 * scale)
    } else {
        (16 * scale, 240 * scale)
    };
    let level = if x == 0 && y == 0 {
        if frame.is_multiple_of(2) { 0 } else { 256 }
    } else {
        (17 * x + 29 * y + 43 * frame + 71 * component) % 257
    };
    (low + ((high - low) * level as u32 + 128) / 256) as u16
}
