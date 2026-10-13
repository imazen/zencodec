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

#[test]
fn quantization_preserves_timeline_and_reports_encoded_durations() {
    use zencodec::animation::TimingQuantizer;
    let source = FrameDuration::new(1001, 30000).unwrap();
    for clock in [100, 1000, 90000] {
        let mut quantizer = TimingQuantizer::new(clock, u64::MAX).unwrap();
        let mut encoded_ticks = 0_u128;
        for frame in 1..=30000_u128 {
            let actual = quantizer.quantize(source).unwrap();
            encoded_ticks += u128::from(actual.ticks_at(clock).unwrap());
            // Independent endpoint oracle: every prefix is within half a tick.
            let target = frame * 1001 * u128::from(clock);
            assert!(2 * (encoded_ticks * 30000).abs_diff(target) <= 30000);
        }
        assert_eq!(encoded_ticks, 1001 * u128::from(clock));
    }
}

#[test]
fn mixed_source_clocks_do_not_accumulate_independent_rounding_error() {
    use zencodec::animation::TimingQuantizer;
    let mut quantizer = TimingQuantizer::new(100, u16::MAX.into()).unwrap();
    let mut source_units = 0_u128;
    let mut actual_ticks = 0_u128;
    for _ in 0..231 {
        for (denominator, units) in [(3, 77), (7, 33), (11, 21)] {
            let actual = quantizer
                .quantize(FrameDuration::new(1, denominator).unwrap())
                .unwrap();
            actual_ticks += u128::from(actual.ticks_at(100).unwrap());
            source_units += units;
            assert!(2 * (actual_ticks * 231).abs_diff(source_units * 100) <= 231);
        }
    }
    assert_eq!(actual_ticks, 13100);
}

#[test]
fn zero_and_rejection_do_not_corrupt_rounding_state() {
    use zencodec::animation::TimingQuantizer;
    let mut quantizer = TimingQuantizer::new(100, 65535).unwrap();
    let fractional = FrameDuration::new(1, 30).unwrap();
    quantizer.quantize(fractional).unwrap();
    let mut control = quantizer;
    assert_eq!(
        quantizer.quantize(FrameDuration::new(u64::MAX, 1).unwrap()),
        Err(TimingError::Overflow)
    );
    assert_eq!(
        quantizer.quantize(FrameDuration::from_millis(0)).unwrap(),
        FrameDuration::from_millis(0)
    );
    assert_eq!(quantizer.quantize(fractional), control.quantize(fractional));
    assert_eq!(quantizer.quantize(fractional), control.quantize(fractional));
    assert!(TimingQuantizer::new(0, 65535).is_err());
}

#[test]
fn integer_ticks_do_not_expand_a_large_fractional_phase() {
    use zencodec::animation::TimingQuantizer;
    let mut quantizer = TimingQuantizer::new(1000, u64::MAX).unwrap();
    for denominator in [4294967291, 4294967279, 4294967231, 4294967197] {
        quantizer
            .quantize(FrameDuration::new(1, denominator).unwrap())
            .unwrap();
    }
    assert_eq!(
        quantizer
            .quantize(FrameDuration::new(1, 100).unwrap())
            .unwrap(),
        FrameDuration::from_millis(10)
    );
    let mut control = quantizer;
    assert_eq!(
        quantizer.quantize(FrameDuration::new(1, 4294967189).unwrap()),
        Err(TimingError::Overflow)
    );
    assert_eq!(
        quantizer.quantize(FrameDuration::from_millis(7)),
        control.quantize(FrameDuration::from_millis(7))
    );
}

#[test]
fn mixed_timing_matches_an_independent_integer_endpoint_oracle() {
    use zencodec::animation::TimingQuantizer;
    const PERIOD: u128 = 9_699_690; // lcm of the source clocks below
    for clock in [1, 100, 1000, u32::MAX] {
        let mut quantizer = TimingQuantizer::new(clock, u64::MAX).unwrap();
        let mut units = 0u128;
        let mut encoded = 0u128;
        let mut random = 17u32;
        for index in 0..4096 {
            random = random.wrapping_mul(1664525).wrapping_add(1013904223);
            let denominator = [2, 3, 5, 7, 11, 13, 17, 19][index % 8];
            let duration = FrameDuration::new(u64::from(random), denominator).unwrap();
            units += u128::from(random) * (PERIOD / u128::from(denominator));
            encoded += u128::from(
                quantizer
                    .quantize(duration)
                    .unwrap()
                    .ticks_at(clock)
                    .unwrap(),
            );
            assert_eq!(encoded, (units * u128::from(clock) + PERIOD / 2) / PERIOD);
        }
    }
}

#[test]
fn adding_large_phases_carries_without_a_129_bit_intermediate() {
    use zencodec::animation::TimingQuantizer;
    let mut quantizer = TimingQuantizer::new(1, u64::MAX).unwrap();
    for denominator in [4294967291, 4294967279, 4294967231, 4294967197] {
        assert_eq!(
            quantizer
                .quantize(FrameDuration::new(u64::from(denominator - 1), denominator).unwrap())
                .unwrap(),
            FrameDuration::new(1, 1).unwrap()
        );
    }
}
