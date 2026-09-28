#![cfg(feature = "animation")]
use zencodec::animation::FrameDuration;
use zencodec_media::{
    animation::{AnimationClock, AnimationError, TimingPolicy},
    time::Rounding,
};

#[test]
fn ntsc_frame_quantization_has_bounded_endpoint_error_after_a_million_frames() {
    for (tps, rounding) in [
        (100, Rounding::Nearest),
        (1000, Rounding::Nearest),
        (1000, Rounding::Floor),
        (1000, Rounding::Ceil),
    ] {
        let mut clock = AnimationClock::new(TimingPolicy::Quantize {
            ticks_per_second: tps,
            rounding,
        })
        .unwrap();
        let source = FrameDuration::new(1001, 30000).unwrap();
        let mut ticks = 0_u64;
        for n in 1..=1_000_000_u64 {
            ticks += clock.push(source).unwrap().ticks_at(tps).unwrap();
            let scaled = n * 1001 * u64::from(tps);
            let expected = match rounding {
                Rounding::Floor => scaled / 30000,
                Rounding::Ceil => scaled.div_ceil(30000),
                Rounding::Nearest => (scaled + 14999) / 30000,
            };
            assert_eq!(ticks, expected, "frame{n} at {tps}");
        }
    }
}

#[test]
fn variable_fractional_clocks_zeros_and_halfway_ties_are_preserved() {
    let mut exact = AnimationClock::new(TimingPolicy::Exact).unwrap();
    let mut quantized = AnimationClock::new(TimingPolicy::Quantize {
        ticks_per_second: 100,
        rounding: Rounding::Nearest,
    })
    .unwrap();
    let durations = [(1, 200), (1, 200), (1, 60), (0, 1), (1001, 30000), (1, 30)];
    // Independent common denominator is 30,000; add integer source ticks.
    let mut cumulative = 0;
    let mut previous = 0;
    for (n, d) in durations {
        let input = FrameDuration::new(n, d).unwrap();
        assert_eq!(exact.push(input).unwrap(), input);
        cumulative += n * (30000 / u64::from(d));
        let endpoint = (cumulative + 149) / 300;
        assert_eq!(
            quantized.push(input).unwrap().ticks_at(100).unwrap(),
            endpoint - previous
        );
        previous = endpoint;
    }
}

#[test]
fn unrepresentable_clock_growth_fails_without_corrupting_the_previous_state() {
    let mut clock = AnimationClock::new(TimingPolicy::Quantize {
        ticks_per_second: 1000,
        rounding: Rounding::Nearest,
    })
    .unwrap();
    let mut saw_overflow = false;
    for denominator in [
        4294967291, 4294967279, 4294967231, 4294967197, 4294967189, 4294967161,
    ] {
        let before = clock.clone();
        if matches!(
            clock.push(FrameDuration::new(1, denominator).unwrap()),
            Err(AnimationError::TimelineOverflow)
        ) {
            let mut before = before;
            // Reuse an existing denominator: adding a new factor can overflow
            // both copies legitimately even when rollback is correct.
            let next = FrameDuration::new(1, 4294967291).unwrap();
            assert_eq!(clock.push(next).unwrap(), before.push(next).unwrap());
            saw_overflow = true;
            break;
        }
    }
    assert!(saw_overflow);
    assert!(
        AnimationClock::new(TimingPolicy::Quantize {
            ticks_per_second: 0,
            rounding: Rounding::Nearest
        })
        .is_err()
    );
}
