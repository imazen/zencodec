//! Track timing programs, edit lists, and checked presentation-timeline mapping.
//!
//! Preserves full carrier timing facts (`codec_delay`, `seek_preroll`, multi-segment
//! [`EditSegment`] lists, and incomplete legacy scalar edit-delay markers) across
//! [`TrackInfo`](crate::track::TrackInfo) and [`CodecConfig`](crate::session::CodecConfig),
//! while compiling an explicit [`PresentationMap`] for exact decoded playback.

use alloc::sync::Arc;
use core::fmt;
use core::num::NonZeroU32;

use zencodec::JobResources;
use zencodec::decode::BoxedError;
use zencodec::time::{FrameDuration, TimeBase, Timestamp, TimingError, duration_to_ticks};

use crate::frame::Frame;
use crate::packet::Discard;
use crate::track::TrackInfo;

/// Rational edit-list playback rate (`numerator / denominator`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EditRate {
    /// Rate numerator (e.g. `1` for normal speed, `0` for dwell/hold).
    pub numerator: i32,
    /// Positive rate denominator (e.g. `1` for normal speed).
    pub denominator: NonZeroU32,
}

impl EditRate {
    /// Normal 1× forward playback (`1/1`).
    pub const UNIT: Self = Self {
        numerator: 1,
        denominator: NonZeroU32::MIN,
    };

    /// Construct a rational playback rate.
    #[inline]
    #[must_use]
    pub const fn new(numerator: i32, denominator: NonZeroU32) -> Self {
        Self {
            numerator,
            denominator,
        }
    }

    /// Returns `true` when this rate equals `1/1` (normal forward playback).
    #[inline]
    #[must_use]
    pub const fn is_unit(self) -> bool {
        self.numerator > 0 && self.numerator as u32 == self.denominator.get()
    }
}

impl Default for EditRate {
    fn default() -> Self {
        Self::UNIT
    }
}

/// One segment of a container edit list (`elst` / timeline program).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum EditSegment {
    /// Empty / silent presentation gap of `duration`.
    Empty {
        /// Gap length on the edited presentation timeline.
        duration: FrameDuration,
    },
    /// Media playback starting at `start` (on the track's media clock) for `duration`
    /// on the edited presentation timeline at `rate`.
    Media {
        /// Starting media timestamp on the track clock.
        start: Timestamp,
        /// Duration on the edited presentation timeline.
        duration: FrameDuration,
        /// Playback rate (`EditRate::UNIT` for normal 1× forward playback).
        rate: EditRate,
    },
}

/// Complete track timing declarations retained across track metadata and codec epochs.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct TrackTiming {
    /// Decoder pre-skip / priming delay recorded by the container.
    pub codec_delay: Option<FrameDuration>,
    /// Pre-roll required before a seek target (e.g. Opus 80 ms in WebM).
    pub seek_preroll: Option<FrameDuration>,
    /// Ordered edit segments from the container (`elst`, empty when unedited).
    pub edits: Arc<[EditSegment]>,
    /// Legacy scalar edit-list offset (`TrackInfoBuilder::with_edit_delay_ticks`).
    ///
    /// When `Some(_)` while [`edits`](Self::edits) is empty, the track carries
    /// incomplete legacy edit evidence and [`PresentationMap::prepare`] refuses with
    /// [`TimelineError::IncompleteLegacyEdit`] instead of treating the track as unedited.
    pub legacy_edit_delay_ticks: Option<i64>,
}

impl Default for TrackTiming {
    fn default() -> Self {
        Self {
            codec_delay: None,
            seek_preroll: None,
            edits: Arc::from([]),
            legacy_edit_delay_ticks: None,
        }
    }
}

impl TrackTiming {
    /// Empty timing declaration (no delay, no preroll, no edit segments, no legacy scalar).
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// Returns `true` when no delay, preroll, edit segments, or legacy edit offset exist.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.codec_delay.is_none()
            && self.seek_preroll.is_none()
            && self.edits.is_empty()
            && self.legacy_edit_delay_ticks.is_none()
    }

    /// Set decoder priming delay.
    #[must_use]
    pub fn with_codec_delay(mut self, delay: FrameDuration) -> Self {
        self.codec_delay = Some(delay);
        self
    }

    /// Set seek pre-roll duration.
    #[must_use]
    pub fn with_seek_preroll(mut self, preroll: FrameDuration) -> Self {
        self.seek_preroll = Some(preroll);
        self
    }

    /// Set the ordered edit-list program.
    #[must_use]
    pub fn with_edits(mut self, edits: impl Into<Arc<[EditSegment]>>) -> Self {
        self.edits = edits.into();
        self
    }

    /// Record a legacy scalar edit-delay marker in track ticks.
    #[must_use]
    pub fn with_legacy_edit_delay_ticks(mut self, ticks: i64) -> Self {
        self.legacy_edit_delay_ticks = Some(ticks);
        self
    }
}

/// Ownership of a carrier trim or delay interval between decoder and presentation mapper.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TrimTreatment {
    /// The decoder did not apply this trim; [`PresentationMap`] must apply it once.
    ApplyInPresentation,
    /// The decoder already applied this trim; [`PresentationMap`] must not double-apply it.
    AlreadyAppliedByDecoder,
    /// Ownership is unproven; refuse if a trim actually affects presentation.
    #[default]
    Unknown,
}

/// Declared trim ownership evidence supplied when preparing a [`PresentationMap`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct TimingEvidence {
    /// Who owns `TrackTiming::codec_delay`.
    pub delay: TrimTreatment,
    /// Who owns packet-level [`Discard`] intervals.
    pub packet_discard: TrimTreatment,
}

impl TimingEvidence {
    /// Construct explicit ownership evidence for codec delay and packet discard.
    #[inline]
    #[must_use]
    pub const fn new(delay: TrimTreatment, packet_discard: TrimTreatment) -> Self {
        Self {
            delay,
            packet_discard,
        }
    }

    /// Both codec delay and packet discard are handled by [`PresentationMap`].
    ///
    /// Note: when `packet_discard` is [`TrimTreatment::ApplyInPresentation`], callers
    /// must supply per-frame discard evidence via
    /// [`PresentationMap::map_frame_with_discard`] (calling [`PresentationMap::map_frame`]
    /// refuses with [`TimelineError::UnappliedPacketDiscard`]).
    #[inline]
    #[must_use]
    pub const fn apply_in_presentation() -> Self {
        Self {
            delay: TrimTreatment::ApplyInPresentation,
            packet_discard: TrimTreatment::ApplyInPresentation,
        }
    }

    /// Both codec delay and packet discard were already applied by the decoder.
    #[inline]
    #[must_use]
    pub const fn already_applied() -> Self {
        Self {
            delay: TrimTreatment::AlreadyAppliedByDecoder,
            packet_discard: TrimTreatment::AlreadyAppliedByDecoder,
        }
    }

    /// Override codec-delay ownership.
    #[inline]
    #[must_use]
    pub const fn with_delay(mut self, delay: TrimTreatment) -> Self {
        self.delay = delay;
        self
    }

    /// Override packet-discard ownership.
    #[inline]
    #[must_use]
    pub const fn with_packet_discard(mut self, packet_discard: TrimTreatment) -> Self {
        self.packet_discard = packet_discard;
        self
    }
}

/// Failure when validating or executing a [`PresentationMap`].
#[derive(Debug)]
#[non_exhaustive]
pub enum TimelineError {
    /// Edit segment uses a non-unit playback rate not supported on the presentation path.
    UnsupportedEditRate(EditRate),
    /// Edit segment starts at a negative media timestamp.
    NegativeMediaStart(Timestamp),
    /// Edit segments revisit or reorder media intervals without admitted replay capacity.
    NonMonotonicEditProgram,
    /// Track carries only a legacy scalar `edit_delay_ticks` without a full edit-segment map.
    IncompleteLegacyEdit,
    /// A trim/delay affects presentation, but [`TimingEvidence`] reported [`TrimTreatment::Unknown`].
    UnknownTrimOwnership,
    /// Presentation owns packet discard (`TimingEvidence::packet_discard == ApplyInPresentation`),
    /// but `map_frame` was called without per-frame discard evidence (`map_frame_with_discard`).
    UnappliedPacketDiscard,
    /// Frame `config_epoch` does not match the prepared track's epoch.
    EpochMismatch {
        /// Epoch bound when preparing this map.
        expected: u32,
        /// Epoch carried on the frame's interpretation.
        actual: u32,
    },
    /// Frame or edit timestamp uses a different [`TimeBase`] than the track clock.
    ClockMismatch {
        /// Expected track time base.
        expected: TimeBase,
        /// Observed time base.
        actual: TimeBase,
    },
    /// Frame lacks a required presentation timestamp for edited or delayed playback.
    MissingTimestamp,
    /// Exact interval clipping or trim application requires an explicit `duration_ticks`,
    /// which the frame did not carry.
    MissingDuration,
    /// Input frame interval overlaps or precedes an earlier mapped frame on the media clock.
    OverlappingFrame {
        /// End (or start, when un-timed) of the previous mapped frame on the media clock.
        prev_end: i64,
        /// Start timestamp of the incoming frame on the media clock.
        start: i64,
    },
    /// Exact rational timing conversion or tick arithmetic overflowed.
    Timing(TimingError),
    /// Operation was cancelled via the bound [`JobResources`] pulse.
    Cancelled(enough::StopReason),
    /// Downstream [`PresentationSink`] failed.
    Sink(BoxedError),
}

impl fmt::Display for TimelineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedEditRate(r) => write!(
                f,
                "unsupported edit playback rate {}/{}",
                r.numerator, r.denominator
            ),
            Self::NegativeMediaStart(ts) => write!(f, "negative media edit start: {ts:?}"),
            Self::NonMonotonicEditProgram => {
                f.write_str("edit program revisits or reorders media without admitted replay")
            }
            Self::IncompleteLegacyEdit => f.write_str(
                "track carries legacy scalar edit_delay_ticks without a complete edit program",
            ),
            Self::UnknownTrimOwnership => {
                f.write_str("codec delay or discard affects presentation with unknown ownership")
            }
            Self::UnappliedPacketDiscard => f.write_str(
                "presentation owns packet discard; pass per-frame discard via map_frame_with_discard",
            ),
            Self::EpochMismatch { expected, actual } => write!(
                f,
                "frame config_epoch mismatch: expected {expected}, got {actual}"
            ),
            Self::ClockMismatch { expected, actual } => {
                write!(f, "clock mismatch: expected {expected:?}, got {actual:?}")
            }
            Self::MissingTimestamp => {
                f.write_str("frame lacks presentation timestamp required by timeline map")
            }
            Self::MissingDuration => f.write_str(
                "frame lacks explicit duration_ticks required for exact interval trimming/clipping",
            ),
            Self::OverlappingFrame { prev_end, start } => write!(
                f,
                "overlapping or out-of-order frame interval: start {start} < previous end {prev_end}"
            ),
            Self::Timing(e) => write!(f, "timing arithmetic error: {e}"),
            Self::Cancelled(r) => write!(f, "timeline mapping cancelled: {r:?}"),
            Self::Sink(e) => write!(f, "presentation sink failed: {e}"),
        }
    }
}

impl core::error::Error for TimelineError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Timing(e) => Some(e),
            Self::Sink(e) => Some(&**e),
            _ => None,
        }
    }
}

/// Receiver of presentation-ordered frames and explicit edit-list gaps from [`PresentationMap`].
pub trait PresentationSink {
    /// Receive one mapped frame on the edited presentation timeline.
    fn presentation(&mut self, frame: Frame) -> Result<(), TimelineError>;

    /// Receive an explicit empty/gap edit segment on the edited presentation timeline.
    fn gap(&mut self, start: Timestamp, duration: FrameDuration) -> Result<(), TimelineError>;
}

/// Prepared, admitted presentation-timeline mapper executing unit-rate multi-segment
/// edit lists, codec-delay trimming, and optional packet-discard trimming with exact
/// two-sided interval intersection and epoch validation.
#[derive(Debug)]
pub struct PresentationMap {
    time_base: TimeBase,
    expected_config_epoch: u32,
    codec_delay_ticks: i128,
    packet_discard: TrimTreatment,
    edits: Arc<[EditSegment]>,
    seg_index: usize,
    pres_cursor_ticks: i64,
    leading_gaps_emitted: bool,
    current_seg_touched: bool,
    last_media_start: Option<i128>,
    last_media_end: Option<i128>,
    resources: JobResources,
}

impl PresentationMap {
    /// Validate `track`'s timing program against `evidence` and `resources`, returning
    /// an admitted [`PresentationMap`].
    ///
    /// Enforces:
    /// - Legacy scalar `edit_delay_ticks` without full `edits` refuses with
    ///   [`TimelineError::IncompleteLegacyEdit`].
    /// - Unknown delay ownership refuses when `codec_delay` is non-zero.
    /// - Non-unit edit rates (`!= 1/1`) refuse with [`TimelineError::UnsupportedEditRate`].
    /// - Edit media starts must share the track clock and be monotonically non-decreasing
    ///   for linear streaming without admitted replay.
    pub fn prepare(
        track: &TrackInfo,
        evidence: TimingEvidence,
        resources: &JobResources,
    ) -> Result<Self, TimelineError> {
        resources.check_stop().map_err(TimelineError::Cancelled)?;
        let time_base = track.time_base();
        let timing = track.timing();

        if timing.edits.is_empty()
            && (timing.legacy_edit_delay_ticks.is_some() || track.edit_delay_ticks().is_some())
        {
            return Err(TimelineError::IncompleteLegacyEdit);
        }

        let codec_delay_ticks = match timing.codec_delay {
            Some(d) if d.numerator() > 0 => match evidence.delay {
                TrimTreatment::Unknown => return Err(TimelineError::UnknownTrimOwnership),
                TrimTreatment::AlreadyAppliedByDecoder => 0i128,
                TrimTreatment::ApplyInPresentation => {
                    let t = duration_to_ticks(d, time_base).map_err(TimelineError::Timing)?;
                    i128::from(t)
                }
            },
            _ => 0i128,
        };

        // Validate edit segments upfront before exposing any frame.
        let mut prev_media_end: Option<i128> = None;
        for &seg in timing.edits.iter() {
            match seg {
                EditSegment::Empty { duration } => {
                    let _ =
                        duration_to_ticks(duration, time_base).map_err(TimelineError::Timing)?;
                }
                EditSegment::Media {
                    start,
                    duration,
                    rate,
                } => {
                    if !rate.is_unit() {
                        return Err(TimelineError::UnsupportedEditRate(rate));
                    }
                    if start.base() != time_base {
                        return Err(TimelineError::ClockMismatch {
                            expected: time_base,
                            actual: start.base(),
                        });
                    }
                    if start.ticks() < 0 {
                        return Err(TimelineError::NegativeMediaStart(start));
                    }
                    let dur_ticks =
                        duration_to_ticks(duration, time_base).map_err(TimelineError::Timing)?;
                    let seg_start = i128::from(start.ticks());
                    let seg_end = seg_start + i128::from(dur_ticks);
                    if i64::try_from(seg_end).is_err() {
                        return Err(TimelineError::Timing(TimingError::Overflow));
                    }
                    if let Some(prev_end) = prev_media_end
                        && seg_start < prev_end
                    {
                        return Err(TimelineError::NonMonotonicEditProgram);
                    }
                    prev_media_end = Some(seg_end);
                }
            }
        }

        Ok(Self {
            time_base,
            expected_config_epoch: track.config_epoch(),
            codec_delay_ticks,
            packet_discard: evidence.packet_discard,
            edits: Arc::clone(&timing.edits),
            seg_index: 0,
            pres_cursor_ticks: 0,
            leading_gaps_emitted: false,
            current_seg_touched: false,
            last_media_start: None,
            last_media_end: None,
            resources: resources.clone(),
        })
    }

    /// Configuration epoch bound to this prepared timeline map.
    #[inline]
    #[must_use]
    pub const fn expected_config_epoch(&self) -> u32 {
        self.expected_config_epoch
    }

    fn advance_media_segment(&mut self, dur_ticks: u64) -> Result<(), TimelineError> {
        let dt =
            i64::try_from(dur_ticks).map_err(|_| TimelineError::Timing(TimingError::Overflow))?;
        self.pres_cursor_ticks = self
            .pres_cursor_ticks
            .checked_add(dt)
            .ok_or(TimelineError::Timing(TimingError::Overflow))?;
        self.seg_index += 1;
        self.current_seg_touched = false;
        Ok(())
    }

    fn emit_pending_empty_segments(
        &mut self,
        dst: &mut dyn PresentationSink,
    ) -> Result<(), TimelineError> {
        while let Some(&EditSegment::Empty { duration }) = self.edits.get(self.seg_index) {
            let start = Timestamp::new(self.pres_cursor_ticks, self.time_base);
            dst.gap(start, duration)?;
            let ticks =
                duration_to_ticks(duration, self.time_base).map_err(TimelineError::Timing)?;
            let dt =
                i64::try_from(ticks).map_err(|_| TimelineError::Timing(TimingError::Overflow))?;
            self.pres_cursor_ticks = self
                .pres_cursor_ticks
                .checked_add(dt)
                .ok_or(TimelineError::Timing(TimingError::Overflow))?;
            self.seg_index += 1;
            self.current_seg_touched = false;
        }
        Ok(())
    }

    fn has_later_media_before(&self, f_end: i128) -> bool {
        for seg in self.edits.get(self.seg_index + 1..).unwrap_or(&[]) {
            if let EditSegment::Media { start, .. } = *seg {
                return i128::from(start.ticks()) < f_end;
            }
        }
        false
    }

    /// Map one decoded presentation-order `frame` onto the edited presentation timeline
    /// and emit it (plus any preceding empty gaps) to `dst`.
    ///
    /// Refuses with [`TimelineError::UnappliedPacketDiscard`] when
    /// `TimingEvidence::packet_discard` is [`TrimTreatment::ApplyInPresentation`]; call
    /// [`map_frame_with_discard`](Self::map_frame_with_discard) to supply per-frame
    /// packet discard evidence explicitly.
    pub fn map_frame(
        &mut self,
        frame: Frame,
        dst: &mut dyn PresentationSink,
    ) -> Result<(), TimelineError> {
        if self.packet_discard == TrimTreatment::ApplyInPresentation {
            return Err(TimelineError::UnappliedPacketDiscard);
        }
        self.map_frame_with_discard(frame, None, dst)
    }

    /// Map one decoded presentation-order `frame` together with its originating packet's
    /// optional [`Discard`] trim onto the edited presentation timeline.
    pub fn map_frame_with_discard(
        &mut self,
        frame: Frame,
        discard: Option<Discard>,
        dst: &mut dyn PresentationSink,
    ) -> Result<(), TimelineError> {
        self.resources
            .check_stop()
            .map_err(TimelineError::Cancelled)?;

        let actual_epoch = frame.interpretation().config_epoch;
        if actual_epoch != self.expected_config_epoch {
            return Err(TimelineError::EpochMismatch {
                expected: self.expected_config_epoch,
                actual: actual_epoch,
            });
        }

        let (trim_start, trim_end) = resolve_discard(discard, self.packet_discard, self.time_base)?;

        if !self.leading_gaps_emitted {
            self.leading_gaps_emitted = true;
            self.emit_pending_empty_segments(dst)?;
        }

        // Fast pass-through when there are no edit segments, no presentation delay,
        // and no presentation-owned discard to apply.
        if self.edits.is_empty() && self.codec_delay_ticks == 0 && trim_start == 0 && trim_end == 0
        {
            if let Some(pts) = frame.timing().pts
                && pts.base() != self.time_base
            {
                return Err(TimelineError::ClockMismatch {
                    expected: self.time_base,
                    actual: pts.base(),
                });
            }
            return dst.presentation(frame);
        }

        let raw_pts = frame.timing().pts.ok_or(TimelineError::MissingTimestamp)?;
        if raw_pts.base() != self.time_base {
            return Err(TimelineError::ClockMismatch {
                expected: self.time_base,
                actual: raw_pts.base(),
            });
        }

        // Step 1: apply presentation-owned packet discard (requires explicit duration).
        let (media_start, media_dur): (i128, Option<u64>) = if trim_start > 0 || trim_end > 0 {
            let frame_dur = frame
                .timing()
                .duration_ticks
                .ok_or(TimelineError::MissingDuration)?;
            let start = i128::from(raw_pts.ticks()) + trim_start;
            let end = i128::from(raw_pts.ticks()) + i128::from(frame_dur) - trim_end;
            if end <= start {
                return Ok(());
            }
            let remaining = u64::try_from(end - start)
                .map_err(|_| TimelineError::Timing(TimingError::Overflow))?;
            (start, Some(remaining))
        } else {
            (i128::from(raw_pts.ticks()), frame.timing().duration_ticks)
        };

        // Step 2: apply codec delay trim once when owned by presentation.
        let f_start = media_start - self.codec_delay_ticks;
        let f_end_opt = media_dur.map(|d| f_start + i128::from(d));

        // Reject overlapping or out-of-order frame intervals rather than silently dropping
        // valid frames after a spanning frame advances the edit cursor.
        if let Some(prev_end) = self.last_media_end
            && f_start < prev_end
        {
            return Err(TimelineError::OverlappingFrame {
                prev_end: i64::try_from(prev_end).unwrap_or(i64::MAX),
                start: i64::try_from(f_start).unwrap_or(i64::MIN),
            });
        } else if let Some(prev_start) = self.last_media_start
            && f_start < prev_start
        {
            return Err(TimelineError::OverlappingFrame {
                prev_end: i64::try_from(prev_start).unwrap_or(i64::MAX),
                start: i64::try_from(f_start).unwrap_or(i64::MIN),
            });
        }
        self.last_media_start = Some(f_start);
        if let Some(end) = f_end_opt {
            self.last_media_end = Some(end);
        }

        // Unedited stream with delay and/or packet discard: intersect with [0, +inf).
        if self.edits.is_empty() {
            let (out_start, out_dur) = if f_start < 0 {
                let f_end = f_end_opt.ok_or(TimelineError::MissingDuration)?;
                if f_end <= 0 {
                    // Entire frame falls before presentation t = 0.
                    return Ok(());
                }
                let clipped = u64::try_from(f_end)
                    .map_err(|_| TimelineError::Timing(TimingError::Overflow))?;
                (0i128, Some(clipped))
            } else {
                (f_start, media_dur)
            };
            let out_pts = i64::try_from(out_start)
                .map_err(|_| TimelineError::Timing(TimingError::Overflow))?;
            let mut shifted_timing = frame.timing();
            shifted_timing.pts = Some(Timestamp::new(out_pts, self.time_base));
            if let Some(d) = out_dur {
                shifted_timing = shifted_timing.with_duration_ticks(d);
            }
            return dst.presentation(frame.represent(shifted_timing, frame.presentation()));
        }

        // Edited stream: two-sided interval intersection across linear edit segments.
        loop {
            self.emit_pending_empty_segments(dst)?;
            let Some(&EditSegment::Media {
                start, duration, ..
            }) = self.edits.get(self.seg_index)
            else {
                // Frame falls past the end of the edit program -> trimmed.
                return Ok(());
            };
            let dur_ticks =
                duration_to_ticks(duration, self.time_base).map_err(TimelineError::Timing)?;
            let seg_start = i128::from(start.ticks());
            let seg_end = seg_start + i128::from(dur_ticks);

            if f_start >= seg_end {
                // Frame starts at or after this segment's end -> finalize segment & advance.
                self.advance_media_segment(dur_ticks)?;
                continue;
            }

            // Frame starts before seg_end: exact two-sided intersection requires duration.
            let f_end = f_end_opt.ok_or(TimelineError::MissingDuration)?;

            if f_start < seg_start && f_end <= seg_start {
                // Entire frame lies strictly before the current edit segment -> trimmed.
                return Ok(());
            }

            // Special case: zero-duration frame right inside [seg_start, seg_end).
            let clip_start = f_start.max(seg_start);
            let clip_end = f_end.min(seg_end);
            if clip_end < clip_start || (f_start < seg_start && clip_end == clip_start) {
                return Ok(());
            }

            let offset_in_seg = clip_start - seg_start;
            let out_pts = i64::try_from(i128::from(self.pres_cursor_ticks) + offset_in_seg)
                .map_err(|_| TimelineError::Timing(TimingError::Overflow))?;
            let clip_dur = u64::try_from(clip_end - clip_start)
                .map_err(|_| TimelineError::Timing(TimingError::Overflow))?;
            let mut out_timing = frame.timing();
            out_timing.pts = Some(Timestamp::new(out_pts, self.time_base));
            out_timing = out_timing.with_duration_ticks(clip_dur);
            self.current_seg_touched = true;
            dst.presentation(frame.represent(out_timing, frame.presentation()))?;

            if f_end > seg_end && self.has_later_media_before(f_end) {
                // Frame strictly spans past this segment into a subsequent Media segment.
                self.advance_media_segment(dur_ticks)?;
                continue;
            }
            return Ok(());
        }
    }

    /// Finish mapping, finalizing any covered active [`EditSegment::Media`] segment
    /// and emitting all trailing [`EditSegment::Empty`] gaps without requiring a
    /// post-boundary sentinel frame.
    pub fn finish(&mut self, dst: &mut dyn PresentationSink) -> Result<(), TimelineError> {
        self.resources
            .check_stop()
            .map_err(TimelineError::Cancelled)?;
        if !self.leading_gaps_emitted {
            self.leading_gaps_emitted = true;
            self.emit_pending_empty_segments(dst)?;
        }
        if self.current_seg_touched
            && let Some(&EditSegment::Media { duration, .. }) = self.edits.get(self.seg_index)
        {
            let dur_ticks =
                duration_to_ticks(duration, self.time_base).map_err(TimelineError::Timing)?;
            self.advance_media_segment(dur_ticks)?;
        }
        self.emit_pending_empty_segments(dst)?;
        Ok(())
    }
}

fn resolve_discard(
    discard: Option<Discard>,
    treatment: TrimTreatment,
    time_base: TimeBase,
) -> Result<(i128, i128), TimelineError> {
    let Some(d) = discard else {
        return Ok((0, 0));
    };
    let start_nonzero = d.start.is_some_and(|x| x.numerator() > 0);
    let end_nonzero = d.end.is_some_and(|x| x.numerator() > 0);
    if !start_nonzero && !end_nonzero {
        return Ok((0, 0));
    }
    match treatment {
        TrimTreatment::Unknown => Err(TimelineError::UnknownTrimOwnership),
        TrimTreatment::AlreadyAppliedByDecoder => Ok((0, 0)),
        TrimTreatment::ApplyInPresentation => {
            let s = match d.start {
                Some(x) if x.numerator() > 0 => {
                    i128::from(duration_to_ticks(x, time_base).map_err(TimelineError::Timing)?)
                }
                _ => 0,
            };
            let e = match d.end {
                Some(x) if x.numerator() > 0 => {
                    i128::from(duration_to_ticks(x, time_base).map_err(TimelineError::Timing)?)
                }
                _ => 0,
            };
            Ok((s, e))
        }
    }
}
