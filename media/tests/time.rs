use std::cmp::Ordering;
use zencodec_media::time::{Rounding, TimeBase, TimeError, Timestamp};

#[test]
fn exact_ntsc_timeline_does_not_accumulate_millisecond_rounding() {
    let ntsc = TimeBase::new(1001, 30000).unwrap();
    let milliseconds = TimeBase::new(1, 1000).unwrap();
    // One hour of 29.97-frame/s ticks: exact 3,603,600 ms, not 3,564,000 ms.
    let time = Timestamp::new(108000, ntsc);
    assert_eq!(
        time.rescale(milliseconds, Rounding::Nearest)
            .unwrap()
            .ticks(),
        3603600
    );
    assert_eq!(TimeBase::new(2002, 60000).unwrap(), ntsc);
    assert_eq!(TimeBase::new(0, 1), Err(TimeError::ZeroTimeBase));
    assert_eq!(TimeBase::new(1, 0), Err(TimeError::ZeroTimeBase));
}

#[test]
fn negative_rounding_and_ties_choose_earlier_presentation() {
    let half = TimeBase::new(1, 2).unwrap();
    let seconds = TimeBase::new(1, 1).unwrap();
    for (input, floor, ceil, nearest) in [
        (-3, -2, -1, -2),
        (-1, -1, 0, -1),
        (1, 0, 1, 0),
        (3, 1, 2, 1),
    ] {
        let input = Timestamp::new(input, half);
        for (mode, expected) in [
            (Rounding::Floor, floor),
            (Rounding::Ceil, ceil),
            (Rounding::Nearest, nearest),
        ] {
            assert_eq!(input.rescale(seconds, mode).unwrap().ticks(), expected);
        }
    }
    let third = TimeBase::new(1, 3).unwrap();
    assert_eq!(
        Timestamp::new(-1, third)
            .rescale(seconds, Rounding::Nearest)
            .unwrap()
            .ticks(),
        0
    );
    assert_eq!(
        Timestamp::new(-2, third)
            .rescale(seconds, Rounding::Nearest)
            .unwrap()
            .ticks(),
        -1
    );
}

#[test]
fn mixed_timebases_and_integer_extremes_never_wrap() {
    let a = TimeBase::new(u32::MAX, u32::MAX - 1).unwrap();
    let b = TimeBase::new(u32::MAX - 2, u32::MAX).unwrap();
    for ticks in [i64::MIN, -1, 0, 1, i64::MAX] {
        let time = Timestamp::new(ticks, a);
        assert_eq!(time.rescale(a, Rounding::Nearest).unwrap(), time);
        assert_eq!(time.compare(time), Ordering::Equal);
        let expected = ticks.cmp(&0);
        assert_eq!(time.compare(Timestamp::new(ticks, b)), expected);
    }
    let fine = TimeBase::new(1, u32::MAX).unwrap();
    let coarse = TimeBase::new(u32::MAX, 1).unwrap();
    assert_eq!(
        Timestamp::new(i64::MAX, coarse).rescale(fine, Rounding::Nearest),
        Err(TimeError::Overflow)
    );
    assert_eq!(
        Timestamp::new(i64::MIN, coarse).rescale(fine, Rounding::Nearest),
        Err(TimeError::Overflow)
    );
    assert_eq!(
        Timestamp::new(i64::MIN, a)
            .checked_add_ticks(u64::MAX)
            .unwrap()
            .ticks(),
        i64::MAX
    );
    assert_eq!(
        Timestamp::new(0, a).checked_add_ticks(u64::MAX),
        Err(TimeError::Overflow)
    );
}
