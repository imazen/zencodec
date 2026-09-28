use zencodec::animation::{FrameDuration, TimingError};
use zencodec::{AnimationFrame, OwnedAnimationFrame};
use zenpixels::{PixelBuffer, PixelDescriptor};

#[test]
fn rational_source_durations_are_canonical_and_never_clamped() {
    assert_eq!(FrameDuration::new(0, 0), Err(TimingError::ZeroDenominator));
    assert_eq!(
        FrameDuration::new(0, u32::MAX).unwrap(),
        FrameDuration::from_millis(0)
    );
    assert_eq!(
        FrameDuration::new(1001, 30000).unwrap(),
        FrameDuration::new(2002, 60000).unwrap()
    );
    let jxl = FrameDuration::new(u64::from(u32::MAX).pow(2), u32::MAX - 1).unwrap();
    assert_eq!(jxl.numerator(), u64::from(u32::MAX).pow(2));
    assert_eq!(jxl.denominator(), u32::MAX - 1);
    for milliseconds in [0, 1, 9, 10, 33, 1000, 655_350, u32::MAX] {
        assert_eq!(
            FrameDuration::from_millis(milliseconds)
                .ticks_at(1000)
                .unwrap(),
            u64::from(milliseconds)
        );
    }
}

#[test]
fn destination_clocks_are_exact_or_fail_without_wrapping() {
    let ntsc = FrameDuration::new(1001, 30000).unwrap();
    assert_eq!(ntsc.ticks_at(90000), Ok(3003));
    assert_eq!(ntsc.ticks_at(1000), Err(TimingError::Inexact));
    assert_eq!(ntsc.ticks_at(0), Err(TimingError::ZeroDenominator));
    let maximum = FrameDuration::new(u64::MAX, 1).unwrap();
    assert_eq!(maximum.ticks_at(1), Ok(u64::MAX));
    assert_eq!(maximum.ticks_at(2), Err(TimingError::Overflow));
    assert_eq!(FrameDuration::new(1, 100).unwrap().ticks_at(100), Ok(1));
    assert_eq!(
        FrameDuration::new(1, 1000).unwrap().ticks_at(100),
        Err(TimingError::Inexact)
    );
}

#[test]
fn borrowing_and_copying_frames_preserves_fractional_and_large_durations() {
    let pixels =
        PixelBuffer::from_vec(vec![1, 2, 3, 4, 5, 6], 2, 1, PixelDescriptor::RGB8_SRGB).unwrap();
    for (duration, legacy) in [
        (FrameDuration::new(1001, 30000).unwrap(), 33),
        (FrameDuration::new(1, 65535).unwrap(), 0),
        (FrameDuration::new(0, 100).unwrap(), 0),
        (FrameDuration::new(u64::MAX, 1).unwrap(), u32::MAX),
    ] {
        let borrowed = AnimationFrame::with_duration(pixels.as_slice(), duration, 7);
        let owned = borrowed.to_owned_frame();
        assert_eq!(owned.duration(), duration);
        assert_eq!(owned.duration_ms(), legacy);
        assert_eq!(owned.frame_index(), 7);
        let borrowed_again = owned.as_animation_frame();
        assert_eq!(borrowed_again.duration(), duration);
        assert_eq!(borrowed_again.duration_ms(), legacy);
        assert_eq!(borrowed_again.pixels().row(0), &[1, 2, 3, 4, 5, 6]);
    }
    let legacy = OwnedAnimationFrame::new(pixels, u32::MAX, 0);
    assert_eq!(legacy.duration(), FrameDuration::from_millis(u32::MAX));
    assert_eq!(legacy.duration_ms(), u32::MAX);
}
