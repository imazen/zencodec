//! Private presentation-map experiment; not a proposed public API yet.
//! Segment order drives acquisition. Projecting frames does not cache, seek,
//! decode, blend gaps, resample audio, or apply codec priming.
#![no_std]
#![forbid(unsafe_code)]
extern crate alloc;
use alloc::vec::Vec;
use zencodec::time::{FrameDuration, TimeBase, Timestamp};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Overflow,
    Inexact,
    InvalidInterval,
    UnsupportedRate,
    Allocation,
    OpenEndedMustBeLast,
    UnsupportedExtent,
}

/// Exact seconds; never implicitly rounds onto a destination clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Time {
    n: i128,
    d: i128,
}
fn gcd(mut a: i128, mut b: i128) -> i128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}
impl Time {
    pub fn new(n: i128, d: i128) -> Result<Self, Error> {
        if d <= 0 || n == i128::MIN {
            return Err(Error::Overflow);
        }
        let g = gcd(n.abs(), d);
        Ok(Self { n: n / g, d: d / g })
    }
    pub fn timestamp(t: Timestamp) -> Self {
        Self::new(
            i128::from(t.ticks()) * i128::from(t.base().numerator()),
            i128::from(t.base().denominator()),
        )
        .unwrap()
    }
    pub fn duration(t: FrameDuration) -> Self {
        Self::new(i128::from(t.numerator()), i128::from(t.denominator())).unwrap()
    }
    pub fn on_clock(self, clock: TimeBase) -> Result<Timestamp, Error> {
        let ticks = self.mul(Self::new(
            i128::from(clock.denominator()),
            i128::from(clock.numerator()),
        )?)?;
        if ticks.d != 1 {
            return Err(Error::Inexact);
        }
        Ok(Timestamp::new(
            i64::try_from(ticks.n).map_err(|_| Error::Overflow)?,
            clock,
        ))
    }
    fn add(self, b: Self) -> Result<Self, Error> {
        let g = gcd(self.d, b.d);
        let n = self
            .n
            .checked_mul(b.d / g)
            .and_then(|a| b.n.checked_mul(self.d / g).and_then(|b| a.checked_add(b)))
            .ok_or(Error::Overflow)?;
        Self::new(n, self.d.checked_mul(b.d / g).ok_or(Error::Overflow)?)
    }
    fn sub(self, b: Self) -> Result<Self, Error> {
        self.add(Self { n: -b.n, d: b.d })
    }
    fn mul(self, b: Self) -> Result<Self, Error> {
        let a_g = gcd(self.n.abs(), b.d);
        let b_g = gcd(b.n.abs(), self.d);
        Self::new(
            (self.n / a_g)
                .checked_mul(b.n / b_g)
                .ok_or(Error::Overflow)?,
            (self.d / b_g)
                .checked_mul(b.d / a_g)
                .ok_or(Error::Overflow)?,
        )
    }
    fn cmp(self, b: Self) -> Result<core::cmp::Ordering, Error> {
        let g = gcd(self.d, b.d);
        Ok(self
            .n
            .checked_mul(b.d / g)
            .ok_or(Error::Overflow)?
            .cmp(&b.n.checked_mul(self.d / g).ok_or(Error::Overflow)?))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Interval {
    start: Time,
    end: Time,
}
impl Interval {
    pub fn start(self) -> Time {
        self.start
    }
    pub fn end(self) -> Time {
        self.end
    }
    pub fn new(start: Time, end: Time) -> Result<Self, Error> {
        if start.cmp(end)? != core::cmp::Ordering::Less {
            return Err(Error::InvalidInterval);
        }
        Ok(Self { start, end })
    }
    fn contains(self, t: Time) -> Result<bool, Error> {
        Ok(self.start.cmp(t)? != core::cmp::Ordering::Greater
            && t.cmp(self.end)? == core::cmp::Ordering::Less)
    }
}

/// Container facts. Rate zero is a generic dwell; negative rates are retained
/// here but explicitly refused by this execution experiment.
#[derive(Clone, Copy, Debug)]
pub enum Content {
    Gap,
    Media {
        start: Timestamp,
        rate_num: i32,
        rate_den: u32,
    },
}
/// An explicit open-ended segment is different from a finite zero-length edit.
#[derive(Clone, Copy, Debug)]
pub enum Extent {
    Finite(FrameDuration),
    Remaining,
}
#[derive(Clone, Copy, Debug)]
pub struct Edit {
    pub extent: Extent,
    pub content: Content,
}
#[derive(Clone, Copy, Debug)]
pub struct Window {
    pub start: Time,
    pub end: Option<Time>,
}
#[derive(Clone, Copy, Debug)]
pub struct Segment {
    pub output: Window,
    source: Option<(Time, Option<Time>, Time)>,
}
/// What the driver must obtain before projecting this segment. Repeated or
/// backward ranges require admitted seek/replay; gaps do not request decoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Acquisition {
    Gap,
    FrameAt(Time),
    Span(Interval),
    From(Time),
}
impl Segment {
    pub fn acquisition(&self) -> Acquisition {
        match self.source {
            None => Acquisition::Gap,
            Some((start, _, rate)) if rate.n == 0 => Acquisition::FrameAt(start),
            Some((start, Some(end), _)) => Acquisition::Span(Interval { start, end }),
            Some((start, None, _)) => Acquisition::From(start),
        }
    }
    /// A frame may contribute to several edits. Call per segment, in edit order;
    /// the driver must acquire/replay that segment, rather than retain all frames.
    pub fn project(&self, frame: Interval) -> Result<Option<Interval>, Error> {
        let Some((start, end, rate)) = self.source else {
            return Ok(None);
        };
        if rate.n == 0 {
            let end = self.output.end.ok_or(Error::UnsupportedExtent)?;
            return Ok(frame.contains(start)?.then_some(Interval {
                start: self.output.start,
                end,
            }));
        }
        let lo = if frame.start.cmp(start)?.is_lt() {
            start
        } else {
            frame.start
        };
        let hi = match end {
            Some(end) if frame.end.cmp(end)?.is_gt() => end,
            _ => frame.end,
        };
        if !lo.cmp(hi)?.is_lt() {
            return Ok(None);
        }
        let inverse = Time::new(rate.d, rate.n)?;
        Ok(Some(Interval::new(
            self.output.start.add(lo.sub(start)?.mul(inverse)?)?,
            self.output.start.add(hi.sub(start)?.mul(inverse)?)?,
        )?))
    }
    pub fn is_gap(&self) -> bool {
        self.source.is_none()
    }
}

/// Owns only edit descriptors. Successful construction validates every edit
/// before acquisition; it proves no workspace bound for codecs or replay.
#[derive(Debug)]
pub struct Plan {
    segments: Vec<Segment>,
}
impl Plan {
    pub fn new(origin: Timestamp, edits: &[Edit]) -> Result<Self, Error> {
        let mut segments = Vec::new();
        segments
            .try_reserve_exact(edits.len())
            .map_err(|_| Error::Allocation)?;
        let mut cursor = Time::timestamp(origin);
        for (index, edit) in edits.iter().enumerate() {
            let duration = match edit.extent {
                Extent::Finite(duration) => {
                    if duration.numerator() == 0 {
                        return Err(Error::InvalidInterval);
                    }
                    Some(Time::duration(duration))
                }
                Extent::Remaining => {
                    if index + 1 != edits.len() {
                        return Err(Error::OpenEndedMustBeLast);
                    }
                    None
                }
            };
            let output = Window {
                start: cursor,
                end: duration.map(|d| cursor.add(d)).transpose()?,
            };
            let source = match edit.content {
                Content::Gap => {
                    if duration.is_none() {
                        return Err(Error::UnsupportedExtent);
                    }
                    None
                }
                Content::Media {
                    start,
                    rate_num,
                    rate_den,
                } => {
                    if rate_num < 0 || rate_den == 0 {
                        return Err(Error::UnsupportedRate);
                    }
                    let rate = Time::new(i128::from(rate_num), i128::from(rate_den))?;
                    let start = Time::timestamp(start);
                    if rate.n == 0 && duration.is_none() {
                        return Err(Error::UnsupportedExtent);
                    }
                    Some((
                        start,
                        duration.map(|d| start.add(d.mul(rate)?)).transpose()?,
                        rate,
                    ))
                }
            };
            segments.push(Segment { output, source });
            if let Some(end) = output.end {
                cursor = end;
            }
        }
        Ok(Self { segments })
    }
    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn time(n: i64, hz: u32) -> Time {
        Time::timestamp(stamp(n, hz))
    }
    fn stamp(n: i64, hz: u32) -> Timestamp {
        Timestamp::new(n, TimeBase::hz(hz).unwrap())
    }
    fn span(a: i64, b: i64, hz: u32) -> Interval {
        Interval::new(time(a, hz), time(b, hz)).unwrap()
    }
    fn edit(duration: u64, hz: u32, start: i64, source_hz: u32, rate: i32) -> Edit {
        Edit {
            extent: Extent::Finite(FrameDuration::new(duration, hz).unwrap()),
            content: Content::Media {
                start: stamp(start, source_hz),
                rate_num: rate,
                rate_den: 1,
            },
        }
    }
    #[test]
    fn repeated_media_is_separate_presentations() {
        let p = Plan::new(
            stamp(0, 600),
            &[edit(1200, 600, 0, 100, 1), edit(6000, 600, 0, 100, 1)],
        )
        .unwrap();
        assert_eq!(
            p.segments()[0].acquisition(),
            Acquisition::Span(span(0, 2, 1))
        );
        assert_eq!(
            p.segments()[1].acquisition(),
            Acquisition::Span(span(0, 10, 1))
        );
        assert_eq!(
            p.segments()[0].project(span(0, 100, 100)).unwrap(),
            Some(span(0, 600, 600))
        );
        assert_eq!(
            p.segments()[1].project(span(0, 100, 100)).unwrap(),
            Some(span(1200, 1800, 600))
        );
    }
    #[test]
    fn clip_crossing_frames_and_double_rate_exactly() {
        let p = Plan::new(stamp(-600, 600), &[edit(600, 600, 200, 100, 2)]).unwrap();
        assert_eq!(
            p.segments()[0].project(span(100, 300, 100)).unwrap(),
            Some(span(-600, -300, 600))
        );
        assert_eq!(
            p.segments()[0].project(span(300, 500, 100)).unwrap(),
            Some(span(-300, 0, 600))
        );
        assert_eq!(p.segments()[0].project(span(400, 500, 100)).unwrap(), None);
    }
    #[test]
    fn gaps_and_dwell_are_explicit_and_half_open() {
        let gap = Edit {
            extent: Extent::Finite(FrameDuration::new(1, 1).unwrap()),
            content: Content::Gap,
        };
        let p = Plan::new(stamp(0, 1), &[gap, edit(2, 1, 3, 1, 0)]).unwrap();
        assert!(p.segments()[0].is_gap());
        assert_eq!(p.segments()[0].acquisition(), Acquisition::Gap);
        assert_eq!(
            p.segments()[1].acquisition(),
            Acquisition::FrameAt(time(3, 1))
        );
        assert_eq!(p.segments()[0].project(span(0, 10, 1)).unwrap(), None);
        assert_eq!(p.segments()[1].project(span(2, 3, 1)).unwrap(), None);
        assert_eq!(
            p.segments()[1].project(span(3, 4, 1)).unwrap(),
            Some(span(1, 3, 1))
        );
    }
    #[test]
    fn admission_rejects_reverse_zero_duration_and_bad_denominator() {
        assert!(matches!(
            Plan::new(stamp(0, 1), &[edit(1, 1, 0, 1, -1)]),
            Err(Error::UnsupportedRate)
        ));
        assert!(matches!(
            Plan::new(stamp(0, 1), &[edit(0, 1, 0, 1, 1)]),
            Err(Error::InvalidInterval)
        ));
        let e = Edit {
            extent: Extent::Finite(FrameDuration::new(1, 1).unwrap()),
            content: Content::Media {
                start: stamp(0, 1),
                rate_num: 1,
                rate_den: 0,
            },
        };
        assert!(matches!(
            Plan::new(stamp(0, 1), &[e]),
            Err(Error::UnsupportedRate)
        ));
    }
    #[test]
    fn destination_clock_never_rounds_implicitly() {
        assert_eq!(time(1, 3).on_clock(TimeBase::MILLIS), Err(Error::Inexact));
        assert_eq!(
            time(1, 3).on_clock(TimeBase::hz(90000).unwrap()).unwrap(),
            stamp(30000, 90000)
        );
        assert_eq!(
            Time::new(i128::MAX, 1).unwrap().on_clock(TimeBase::MILLIS),
            Err(Error::Overflow)
        );
    }
    #[test]
    fn each_integer_source_frame_projects_to_independent_rational_oracle() {
        for rate in 1..=7 {
            for start in -5..=5 {
                let p = Plan::new(stamp(0, 1), &[edit(3, 1, start, 1, rate)]).unwrap();
                for frame in -8..=30 {
                    let lo = frame.max(start);
                    let hi = (frame + 1).min(start + 3 * i64::from(rate));
                    let expected = if hi <= lo {
                        None
                    } else {
                        Some(
                            Interval::new(
                                Time::new(i128::from(lo - start), i128::from(rate)).unwrap(),
                                Time::new(i128::from(hi - start), i128::from(rate)).unwrap(),
                            )
                            .unwrap(),
                        )
                    };
                    assert_eq!(
                        p.segments()[0].project(span(frame, frame + 1, 1)).unwrap(),
                        expected
                    );
                }
            }
        }
    }

    #[test]
    fn fractional_rate_and_movie_clock_remain_exact() {
        let e = Edit {
            extent: Extent::Finite(FrameDuration::new(2, 1).unwrap()),
            content: Content::Media {
                start: stamp(0, 100),
                rate_num: 3,
                rate_den: 2,
            },
        };
        let p = Plan::new(stamp(600, 600), &[e]).unwrap();
        let actual = p.segments()[0].project(span(0, 100, 100)).unwrap().unwrap();
        assert_eq!(
            actual.start().on_clock(TimeBase::hz(600).unwrap()).unwrap(),
            stamp(600, 600)
        );
        assert_eq!(
            actual.end().on_clock(TimeBase::hz(600).unwrap()).unwrap(),
            stamp(1000, 600)
        );
        assert_eq!(actual.end().on_clock(TimeBase::MILLIS), Err(Error::Inexact));
    }

    #[test]
    fn open_ended_offset_projects_future_fragments_without_guessing_end() {
        let mut e = edit(1, 1, 200, 100, 1);
        e.extent = Extent::Remaining;
        let p = Plan::new(stamp(0, 600), &[e]).unwrap();
        let s = &p.segments()[0];
        assert_eq!(s.output.end, None);
        assert_eq!(s.acquisition(), Acquisition::From(time(2, 1)));
        assert_eq!(s.project(span(0, 200, 100)).unwrap(), None);
        assert_eq!(
            s.project(span(150, 250, 100)).unwrap(),
            Some(span(0, 300, 600))
        );
        assert_eq!(
            s.project(span(10000, 10100, 100)).unwrap(),
            Some(span(58800, 59400, 600))
        );
    }

    #[test]
    fn open_end_requires_terminal_media_with_progressing_time() {
        let mut e = edit(1, 1, 0, 1, 1);
        e.extent = Extent::Remaining;
        assert!(matches!(
            Plan::new(stamp(0, 1), &[e, edit(1, 1, 0, 1, 1)]),
            Err(Error::OpenEndedMustBeLast)
        ));
        let gap = Edit {
            extent: Extent::Remaining,
            content: Content::Gap,
        };
        assert!(matches!(
            Plan::new(stamp(0, 1), &[gap]),
            Err(Error::UnsupportedExtent)
        ));
        let dwell = Edit {
            extent: Extent::Remaining,
            content: Content::Media {
                start: stamp(0, 1),
                rate_num: 0,
                rate_den: 1,
            },
        };
        assert!(matches!(
            Plan::new(stamp(0, 1), &[dwell]),
            Err(Error::UnsupportedExtent)
        ));
    }
}
