//! Container presentation instructions, separate from codec configuration.
//!
//! These types retain declarations. They neither execute edits nor certify a
//! muxer's ability to preserve them. Unsupported execution (such as reverse
//! playback) can still be retained for a capable remux route.

use alloc::{sync::Arc, vec::Vec};
use core::fmt;
use zencodec::time::{FrameDuration, Timestamp};

/// Exact signed media-time advance per unit of presentation time.
/// Zero denotes a dwell; a negative value denotes reverse playback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlaybackRate {
    numerator: i32,
    denominator: u32,
}
impl PlaybackRate {
    /// Normal playback.
    pub const NORMAL: Self = Self {
        numerator: 1,
        denominator: 1,
    };
    /// Construct and reduce an exact rate. A zero denominator refuses.
    pub fn new(numerator: i32, denominator: u32) -> Result<Self, TimelineError> {
        if denominator == 0 {
            return Err(TimelineError::ZeroRateDenominator);
        }
        let (mut a, mut b) = (numerator.unsigned_abs(), denominator);
        while b != 0 {
            (a, b) = (b, a % b);
        }
        Ok(Self {
            numerator: (i64::from(numerator) / i64::from(a)) as i32,
            denominator: denominator / a,
        })
    }
    /// Signed numerator of the reduced rate.
    pub const fn numerator(self) -> i32 {
        self.numerator
    }
    /// Nonzero denominator of the reduced rate.
    pub const fn denominator(self) -> u32 {
        self.denominator
    }
}

/// How long an edit occupies the presentation timeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EditExtent {
    /// An exact duration, independent of the media clock. Zero is retained as
    /// a finite declaration, not silently changed to an open-ended edit.
    Finite(FrameDuration),
    /// Continues through the eventual end of the media, including future
    /// fragments. Only the last edit may have this extent.
    ToEnd,
}

/// Content selected by one edit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EditContent {
    /// No source media. Rendering requires an explicit fill/omit/refuse choice;
    /// this declaration does not implicitly choose black or silence.
    Gap,
    /// Source media begins at this exact timestamp and advances at `rate`.
    Media {
        /// Media timestamp, carrying its own clock.
        start: Timestamp,
        /// Exact playback rate; preservation does not imply executability.
        rate: PlaybackRate,
    },
}

/// One ordered presentation edit. Fields are private to allow additive facts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EditSegment {
    extent: EditExtent,
    content: EditContent,
}
impl EditSegment {
    /// Retain the supplied declaration without interpreting its pixels/audio.
    pub const fn new(extent: EditExtent, content: EditContent) -> Self {
        Self { extent, content }
    }
    /// Presentation duration or explicit open end.
    pub const fn extent(self) -> EditExtent {
        self.extent
    }
    /// Selected media or explicit gap.
    pub const fn content(self) -> EditContent {
        self.content
    }
}

/// Ordered edits with an explicit presentation origin.
/// An empty list means an empty presentation, not unedited media.
#[derive(Debug, PartialEq, Eq)]
pub struct EditList {
    origin: Timestamp,
    segments: Vec<EditSegment>,
}
impl EditList {
    /// Move already-owned descriptors into the list without cloning or
    /// allocating. Validates ordering of open ends, not native executability.
    pub fn new(origin: Timestamp, segments: Vec<EditSegment>) -> Result<Self, TimelineError> {
        if segments
            .iter()
            .take(segments.len().saturating_sub(1))
            .any(|segment| matches!(segment.extent, EditExtent::ToEnd))
        {
            return Err(TimelineError::NonterminalOpenEnd);
        }
        Ok(Self { origin, segments })
    }
    /// First presentation time, independent of each source timestamp's clock.
    pub const fn origin(&self) -> Timestamp {
        self.origin
    }
    /// Segments in presentation order, including repeats and gaps.
    pub fn segments(&self) -> &[EditSegment] {
        &self.segments
    }
}

/// What is known about a track's presentation program.
/// Immutable lists can be shared by the demuxer, renderer and remuxer without
/// deep cloning. Keep the original owner while delayed frames or replay can
/// still refer to its interpretation; a newer configuration cannot replace it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum TrackTimeline {
    /// No qualified declaration is available. This is not evidence of identity.
    #[default]
    Unknown,
    /// The container has been inspected and declares no edit program.
    /// Codec priming, packet discard and seek pre-roll remain separate facts.
    Unedited,
    /// Complete ordered program. A renderer/muxer must bind support or refuse;
    /// inspecting this value is not a capability or memory-budget guarantee.
    Edited(Arc<EditList>),
}

/// Structurally invalid presentation declaration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TimelineError {
    /// A rational playback rate cannot have a zero denominator.
    ZeroRateDenominator,
    /// An open-ended edit cannot precede another edit with an implied origin.
    NonterminalOpenEnd,
}
impl fmt::Display for TimelineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::ZeroRateDenominator => "playback rate has a zero denominator",
            Self::NonterminalOpenEnd => "open-ended edit is not the final segment",
        })
    }
}
impl core::error::Error for TimelineError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::track::{CodecId, TrackError, TrackInfo, VideoTrackInfo};
    use alloc::vec;
    use zencodec::time::TimeBase;

    fn origin() -> Timestamp {
        Timestamp::new(0, TimeBase::MILLIS)
    }
    fn media(extent: EditExtent) -> EditSegment {
        EditSegment::new(
            extent,
            EditContent::Media {
                start: origin(),
                rate: PlaybackRate::NORMAL,
            },
        )
    }
    #[test]
    fn exact_rates_keep_zero_reverse_and_extremes_without_rounding() {
        assert_eq!(PlaybackRate::new(2, 4), PlaybackRate::new(1, 2));
        assert_eq!(PlaybackRate::new(0, u32::MAX).unwrap().denominator(), 1);
        assert_eq!(
            PlaybackRate::new(i32::MIN, 1 << 31),
            PlaybackRate::new(-1, 1)
        );
        assert_eq!(
            PlaybackRate::new(i32::MIN, 1).unwrap().numerator(),
            i32::MIN
        );
        assert_eq!(
            PlaybackRate::new(1, 0),
            Err(TimelineError::ZeroRateDenominator)
        );
    }
    #[test]
    fn unknown_unedited_empty_finite_zero_and_open_are_distinct() {
        let empty = TrackTimeline::Edited(Arc::new(EditList::new(origin(), vec![]).unwrap()));
        assert_ne!(TrackTimeline::Unknown, TrackTimeline::Unedited);
        assert_ne!(empty, TrackTimeline::Unedited);
        let finite = EditList::new(
            origin(),
            vec![media(EditExtent::Finite(FrameDuration::new(0, 1).unwrap()))],
        )
        .unwrap();
        let open = EditList::new(origin(), vec![media(EditExtent::ToEnd)]).unwrap();
        assert_ne!(finite, open);
        assert_eq!(
            EditList::new(
                origin(),
                vec![media(EditExtent::ToEnd), media(EditExtent::ToEnd)]
            ),
            Err(TimelineError::NonterminalOpenEnd)
        );
    }
    #[test]
    fn track_clones_share_program_and_legacy_conflicts_do_not_depend_on_order() {
        let program = Arc::new(EditList::new(origin(), vec![media(EditExtent::ToEnd)]).unwrap());
        let make =
            || TrackInfo::video(0, CodecId::Av1, TimeBase::MILLIS, VideoTrackInfo::new(2, 2));
        assert_eq!(make().build().unwrap().timeline(), &TrackTimeline::Unknown);
        let track = make()
            .with_timeline(TrackTimeline::Edited(program.clone()))
            .build()
            .unwrap();
        let copy = track.clone();
        let TrackTimeline::Edited(actual) = copy.timeline() else {
            panic!("lost program")
        };
        assert!(Arc::ptr_eq(&program, actual));
        #[allow(deprecated)]
        let conflicts = [
            make()
                .with_edit_delay_ticks(0)
                .with_timeline(TrackTimeline::Unedited),
            make()
                .with_timeline(TrackTimeline::Unedited)
                .with_edit_delay_ticks(0),
        ];
        for builder in conflicts {
            assert!(matches!(
                builder.build(),
                Err(TrackError::ConflictingTimeline)
            ));
        }
    }
}
