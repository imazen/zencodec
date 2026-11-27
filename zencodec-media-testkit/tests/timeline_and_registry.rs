//! Contract tests for PR 1 `zencodec-media` additions:
//! - `TrackTiming` preservation through `CodecConfig::from(&TrackInfo)` and scalar-only legacy edits.
//! - `PresentationMap` two-sided interval intersection, codec-delay zero straddling, missing duration
//!   refusal, trailing gaps at EOF without sentinel frames, `TimingEvidence::packet_discard` ownership,
//!   and `config_epoch` enforcement.
//! - `StagedEncode` timing handoff, timing mutation refusal, and bounded progress accounting.
//! - `Receive::Progress` `work_units` accounting, progress-only limit termination, and post-callback
//!   pulse polling across pump helpers.
//! - Cooperative `Pulse` cancellation on `VideoDecoder::{end_input, reset}` and `VideoEncoder::end_input`.
//! - Media factory roles (`DemuxRole`, `MuxRole`) in `CodecSet`, `erase_demuxer` refusing unquoted
//!   `begin_seek`, `erase_seekable_demuxer`, and routed `MuxFactory` -> `as_bounded_mut` backpressure.

use std::num::NonZeroU32;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use enough::{Stop, StopReason};
use zencodec::decode::{ErasedError, Input};
use zencodec::how_far::{NoPulse, Report, StopOnly};
use zencodec::operation::PolicySet;
use zencodec::source::{AccessPattern, NativeBound, SourceAccess};
use zencodec::storage::StorageBudget;
use zencodec::time::{FrameDuration, TimeBase, Timestamp};
use zencodec::{CodecSet, JobResources, NamedFactory, Pulse, ResourceLimits, SharedPulse};
use zencodec_media::container::{
    BoundedMuxer, Demuxer, DynDemuxer, DynMuxer, MuxFinish, MuxHint, MuxOffer, MuxTiming, Muxer,
    PacketWrite, SeekOffer, SeekStatus, SeekTarget, SeekableDemuxer, erase_bounded_muxer,
    erase_demuxer, erase_seekable_demuxer,
};
use zencodec_media::frame::{
    Frame, FrameGeometry, FrameInterpretation, FrameTiming, OwnedPlane, OwnedPlanes, PlaneRole,
    Presentation,
};
use zencodec_media::limits::MediaLimits;
use zencodec_media::output::Forward;
use zencodec_media::packet::{Discard, Packet};
use zencodec_media::registry::{
    ContainerId, ContainerProbe, DemuxFactory, DemuxNeeds, DemuxOffer, DemuxRole, MediaError,
    MuxFactory, MuxNeeds, MuxPlan, MuxRole, ProbeLimits, TimingPolicy,
};
use zencodec_media::session::{
    CodecConfig, ProtocolViolation, PumpError, Receive, SendStatus, SessionError, UnsupportedMedia,
    VideoDecoder, VideoEncoder, VideoStreamInfo, drain_decoder, drain_encoder, erase_video_decoder,
    erase_video_encoder, send_frame_until_accepted, send_packet_until_accepted,
};
use zencodec_media::staged::{Refusal, StagedEncode};
use zencodec_media::timeline::{
    EditRate, EditSegment, PresentationMap, PresentationSink, TimelineError, TimingEvidence,
    TrackTiming, TrimTreatment,
};
use zencodec_media::track::{CodecId, CodecTag, TrackInfo, VideoTrackInfo};
use zencodec_media_testkit::toy::{
    ToyDecoder, ToyDemuxer, ToyEncoder, ToyError, ToyMuxer, toy_payload,
};

fn tb_30() -> TimeBase {
    TimeBase::new(1, 30).unwrap()
}

fn make_frame_with_epoch(pts_ticks: i64, dur_ticks: Option<u64>, epoch: u32, val: u8) -> Frame {
    let enc = zencodec::sample::SampleEncoding::new(zenpixels::SampleType::U8, 8, 0).unwrap();
    let planes = OwnedPlanes::new(
        vec![OwnedPlane::U8(
            zencodec::plane::PlaneBuf::new(vec![val; 4], 2, 2, 2).unwrap(),
        )],
        enc,
    )
    .unwrap();
    let interp = FrameInterpretation::new(
        FrameGeometry::full(2, 2),
        zencodec::plane::PlanarLayout::new(
            vec![zencodec::plane::PlaneSpec::new(
                PlaneRole::Luma,
                zencodec::plane::PlaneEncoding::Code(enc),
            )],
            zencodec::sample::ChromaSubsampling::Mono,
            zencodec::sample::ChromaSiting::Unknown,
        )
        .unwrap(),
    )
    .with_config_epoch(epoch);
    let mut timing = FrameTiming::at(Timestamp::new(pts_ticks, tb_30()));
    if let Some(d) = dur_ticks {
        timing = timing.with_duration_ticks(d);
    }
    Frame::new(
        Arc::new(planes),
        Arc::new(interp),
        timing,
        Presentation::default(),
    )
}

fn make_frame(pts_ticks: i64, dur_ticks: Option<u64>, val: u8) -> Frame {
    make_frame_with_epoch(pts_ticks, dur_ticks, 0, val)
}

#[derive(Default)]
struct SinkLog {
    gaps: Vec<(i64, u64)>,
    frames: Vec<(i64, Option<u64>)>,
}

impl PresentationSink for SinkLog {
    fn gap(&mut self, start: Timestamp, duration: FrameDuration) -> Result<(), TimelineError> {
        let ticks = zencodec::time::duration_to_ticks(duration, start.base()).unwrap();
        self.gaps.push((start.ticks(), ticks));
        Ok(())
    }

    fn presentation(&mut self, frame: Frame) -> Result<(), TimelineError> {
        self.frames.push((
            frame.timing().pts.unwrap().ticks(),
            frame.timing().duration_ticks,
        ));
        Ok(())
    }
}

#[test]
fn track_timing_and_scalar_only_legacy_edit_survive_codec_config_and_refuse_exact_map() {
    let timing = TrackTiming::empty()
        .with_codec_delay(FrameDuration::new(1, 30).unwrap())
        .with_seek_preroll(FrameDuration::new(3, 30).unwrap())
        .with_edits(Arc::from([
            EditSegment::Empty {
                duration: FrameDuration::new(2, 30).unwrap(),
            },
            EditSegment::Media {
                start: Timestamp::new(5, tb_30()),
                duration: FrameDuration::new(10, tb_30().denominator()).unwrap(),
                rate: EditRate::UNIT,
            },
        ]));
    let track = TrackInfo::video(0, CodecId::Av1, tb_30(), VideoTrackInfo::new(16, 16))
        .with_timing(timing.clone())
        .with_edit_delay_ticks(-5)
        .build()
        .unwrap();
    let track_rev = TrackInfo::video(0, CodecId::Av1, tb_30(), VideoTrackInfo::new(16, 16))
        .with_edit_delay_ticks(-5)
        .with_timing(timing)
        .build()
        .unwrap();

    let cfg = CodecConfig::from(&track);
    let cfg_rev = CodecConfig::from(&track_rev);
    assert_eq!(
        cfg.timing.codec_delay,
        Some(FrameDuration::new(1, 30).unwrap())
    );
    assert_eq!(
        cfg.timing.seek_preroll,
        Some(FrameDuration::new(3, 30).unwrap())
    );
    assert_eq!(cfg.timing.edits.len(), 2);
    assert_eq!(cfg.timing, *track.timing());
    assert_eq!(track.edit_delay_ticks(), Some(-5));
    assert_eq!(track_rev.edit_delay_ticks(), Some(-5));
    assert_eq!(track_rev.timing().legacy_edit_delay_ticks, Some(-5));
    assert_eq!(cfg.timing, cfg_rev.timing);

    // Scalar-only track (`with_edit_delay_ticks` without full `edits`) must preserve
    // incomplete legacy edit evidence through `CodecConfig::from` and refuse exact mapping.
    let scalar_only = TrackInfo::video(0, CodecId::Av1, tb_30(), VideoTrackInfo::new(16, 16))
        .with_edit_delay_ticks(12)
        .build()
        .unwrap();
    assert!(!scalar_only.timing().is_empty());
    assert_eq!(scalar_only.timing().legacy_edit_delay_ticks, Some(12));
    let scalar_cfg = CodecConfig::from(&scalar_only);
    assert_eq!(scalar_cfg.timing.legacy_edit_delay_ticks, Some(12));

    let resources = JobResources::new(
        ResourceLimits::none(),
        StorageBudget::new(1024, 8),
        SharedPulse::default(),
    );
    assert!(matches!(
        PresentationMap::prepare(&scalar_only, TimingEvidence::already_applied(), &resources),
        Err(TimelineError::IncompleteLegacyEdit)
    ));
}

#[test]
fn presentation_map_interval_intersection_trailing_gaps_discard_and_epoch_checks() {
    let resources = JobResources::new(
        ResourceLimits::none(),
        StorageBudget::new(1024, 8),
        SharedPulse::default(),
    );

    // 1. Unknown delay ownership refused when codec_delay > 0.
    let delayed_track = TrackInfo::video(0, CodecId::Av1, tb_30(), VideoTrackInfo::new(2, 2))
        .with_codec_delay(FrameDuration::new(3, 30).unwrap())
        .build()
        .unwrap();
    assert!(matches!(
        PresentationMap::prepare(&delayed_track, TimingEvidence::default(), &resources),
        Err(TimelineError::UnknownTrimOwnership)
    ));

    // 2. Codec-delay zero straddling: frame [1, 5) shifted by delay 3 -> [-2, 2) -> surviving [0, 2)!
    let delay_only_evidence = TimingEvidence::new(
        TrimTreatment::ApplyInPresentation,
        TrimTreatment::AlreadyAppliedByDecoder,
    );
    let mut map =
        PresentationMap::prepare(&delayed_track, delay_only_evidence, &resources).unwrap();
    let mut log = SinkLog::default();
    map.map_frame(make_frame(0, Some(1), 0), &mut log).unwrap(); // [-3, -2) -> dropped
    map.map_frame(make_frame(1, Some(4), 1), &mut log).unwrap(); // [-2, 2) -> [0, 2)
    map.map_frame(make_frame(5, Some(2), 2), &mut log).unwrap(); // [2, 4) -> [2, 2)
    map.finish(&mut log).unwrap();
    assert_eq!(log.frames, vec![(0, Some(2)), (2, Some(2))]);

    // 2b. Overlapping frames across split Media edits [0, 5) + [5, 10) must refuse explicitly
    //     rather than silently dropping valid PTS-ordered input.
    let split_edit_track = TrackInfo::video(0, CodecId::Av1, tb_30(), VideoTrackInfo::new(2, 2))
        .with_timing(TrackTiming::empty().with_edits(Arc::from([
            EditSegment::Media {
                start: Timestamp::new(0, tb_30()),
                duration: FrameDuration::new(5, 30).unwrap(),
                rate: EditRate::UNIT,
            },
            EditSegment::Media {
                start: Timestamp::new(5, tb_30()),
                duration: FrameDuration::new(5, 30).unwrap(),
                rate: EditRate::UNIT,
            },
        ])))
        .build()
        .unwrap();
    let mut omap = PresentationMap::prepare(
        &split_edit_track,
        TimingEvidence::already_applied(),
        &resources,
    )
    .unwrap();
    let mut olog = SinkLog::default();
    omap.map_frame(make_frame(0, Some(8), 1), &mut olog)
        .unwrap();
    assert!(matches!(
        omap.map_frame(make_frame(4, Some(1), 2), &mut olog),
        Err(TimelineError::OverlappingFrame {
            prev_end: 8,
            start: 4
        })
    ));

    // Missing duration when straddling tick 0 refuses explicitly.
    let mut map_nodur =
        PresentationMap::prepare(&delayed_track, delay_only_evidence, &resources).unwrap();
    let mut nd_log = SinkLog::default();
    assert!(matches!(
        map_nodur.map_frame(make_frame(1, None, 1), &mut nd_log),
        Err(TimelineError::MissingDuration)
    ));

    // 3. Edit-list left-side straddling: frame PTS 3 dur 4 ([3, 7)) vs Media [5, 9) -> [pres 0, dur 2).
    let straddle_track = TrackInfo::video(0, CodecId::Av1, tb_30(), VideoTrackInfo::new(2, 2))
        .with_timing(TrackTiming::empty().with_edits(Arc::from([
            EditSegment::Media {
                start: Timestamp::new(5, tb_30()),
                duration: FrameDuration::new(4, 30).unwrap(),
                rate: EditRate::UNIT,
            },
            EditSegment::Empty {
                duration: FrameDuration::new(3, 30).unwrap(),
            },
        ])))
        .build()
        .unwrap();
    let mut smap = PresentationMap::prepare(
        &straddle_track,
        TimingEvidence::already_applied(),
        &resources,
    )
    .unwrap();
    let mut slog = SinkLog::default();
    smap.map_frame(make_frame(3, Some(4), 1), &mut slog)
        .unwrap(); // [3, 7) ∩ [5, 9) = [5, 7) -> (0, 2)
    smap.map_frame(make_frame(7, Some(2), 2), &mut slog)
        .unwrap(); // [7, 9) -> (2, 2)
    // Finish WITHOUT any sentinel frame at 9: MUST still emit trailing Empty(3) at pres 4!
    smap.finish(&mut slog).unwrap();
    assert_eq!(slog.frames, vec![(0, Some(2)), (2, Some(2))]);
    assert_eq!(slog.gaps, vec![(4, 3)]);

    // 4. Explicit trailing gap after Media[0, 2) with last frame [1, 2) and NO sentinel frame.
    let gap_eof_track = TrackInfo::video(0, CodecId::Av1, tb_30(), VideoTrackInfo::new(2, 2))
        .with_timing(TrackTiming::empty().with_edits(Arc::from([
            EditSegment::Media {
                start: Timestamp::new(0, tb_30()),
                duration: FrameDuration::new(2, 30).unwrap(),
                rate: EditRate::UNIT,
            },
            EditSegment::Empty {
                duration: FrameDuration::new(5, 30).unwrap(),
            },
        ])))
        .build()
        .unwrap();
    let mut gmap = PresentationMap::prepare(
        &gap_eof_track,
        TimingEvidence::already_applied(),
        &resources,
    )
    .unwrap();
    let mut glog = SinkLog::default();
    gmap.map_frame(make_frame(0, Some(1), 0), &mut glog)
        .unwrap();
    gmap.map_frame(make_frame(1, Some(1), 1), &mut glog)
        .unwrap();
    gmap.finish(&mut glog).unwrap();
    assert_eq!(glog.frames, vec![(0, Some(1)), (1, Some(1))]);
    assert_eq!(glog.gaps, vec![(2, 5)]);

    // Missing duration on edited interval refuses explicitly.
    let mut gmap_nodur = PresentationMap::prepare(
        &gap_eof_track,
        TimingEvidence::already_applied(),
        &resources,
    )
    .unwrap();
    let mut glog2 = SinkLog::default();
    assert!(matches!(
        gmap_nodur.map_frame(make_frame(0, None, 0), &mut glog2),
        Err(TimelineError::MissingDuration)
    ));

    // 5. Packet discard ownership:
    //    - ApplyInPresentation refuses bare map_frame (UnappliedPacketDiscard) and applies Discard
    //      once via map_frame_with_discard.
    //    - AlreadyAppliedByDecoder ignores Discard (never double-applies).
    //    - Unknown refuses non-zero Discard (UnknownTrimOwnership).
    let plain_track = TrackInfo::video(0, CodecId::Av1, tb_30(), VideoTrackInfo::new(2, 2))
        .with_config_epoch(2)
        .build()
        .unwrap();
    let mut dmap = PresentationMap::prepare(
        &plain_track,
        TimingEvidence::apply_in_presentation(),
        &resources,
    )
    .unwrap();
    let mut dlog = SinkLog::default();
    assert!(matches!(
        dmap.map_frame(make_frame_with_epoch(10, Some(6), 2, 1), &mut dlog),
        Err(TimelineError::UnappliedPacketDiscard)
    ));
    // Wrong config_epoch refused!
    assert!(matches!(
        dmap.map_frame_with_discard(make_frame_with_epoch(10, Some(6), 1, 1), None, &mut dlog),
        Err(TimelineError::EpochMismatch {
            expected: 2,
            actual: 1
        })
    ));
    let discard = Discard::start(FrameDuration::new(1, 30).unwrap())
        .with_end(FrameDuration::new(2, 30).unwrap());
    dmap.map_frame_with_discard(
        make_frame_with_epoch(10, Some(6), 2, 1),
        Some(discard),
        &mut dlog,
    )
    .unwrap();
    assert_eq!(dlog.frames, vec![(11, Some(3))]);

    let mut amap =
        PresentationMap::prepare(&plain_track, TimingEvidence::already_applied(), &resources)
            .unwrap();
    let mut alog = SinkLog::default();
    amap.map_frame_with_discard(
        make_frame_with_epoch(10, Some(6), 2, 1),
        Some(discard),
        &mut alog,
    )
    .unwrap();
    assert_eq!(alog.frames, vec![(10, Some(6))]);

    let mut umap =
        PresentationMap::prepare(&plain_track, TimingEvidence::default(), &resources).unwrap();
    let mut ulog = SinkLog::default();
    assert!(matches!(
        umap.map_frame_with_discard(
            make_frame_with_epoch(10, Some(6), 2, 1),
            Some(discard),
            &mut ulog
        ),
        Err(TimelineError::UnknownTrimOwnership)
    ));

    // 6. Non-unit EditRate & NonMonotonicEditProgram refused upfront.
    let bad_rate_track = TrackInfo::video(0, CodecId::Av1, tb_30(), VideoTrackInfo::new(2, 2))
        .with_timing(
            TrackTiming::empty().with_edits(Arc::from([EditSegment::Media {
                start: Timestamp::new(0, tb_30()),
                duration: FrameDuration::new(5, 30).unwrap(),
                rate: EditRate::new(2, NonZeroU32::new(1).unwrap()),
            }])),
        )
        .build()
        .unwrap();
    assert!(matches!(
        PresentationMap::prepare(&bad_rate_track, TimingEvidence::default(), &resources),
        Err(TimelineError::UnsupportedEditRate(_))
    ));
}

#[test]
fn staged_encode_transfers_timing_and_rejects_timing_mutation_and_progress_spin() {
    struct TimingCaptureMuxer {
        inner: ToyMuxer,
        captured_timing: Arc<std::sync::Mutex<Option<TrackTiming>>>,
    }
    impl Muxer for TimingCaptureMuxer {
        type Error = ToyError;
        fn begin(
            &mut self,
            tracks: &[TrackInfo],
            hint: MuxHint,
        ) -> Result<MuxTiming, SessionError<ToyError>> {
            *self.captured_timing.lock().unwrap() = Some((**tracks[0].timing()).clone());
            self.inner.begin(tracks, hint)
        }
        fn begin_exact_timing(
            &mut self,
            tracks: &[TrackInfo],
            hint: MuxHint,
        ) -> Result<MuxTiming, SessionError<ToyError>> {
            *self.captured_timing.lock().unwrap() = Some((**tracks[0].timing()).clone());
            self.inner.begin(tracks, hint)
        }
        fn write_packet(
            &mut self,
            packet: Packet,
            pulse: Option<&dyn Pulse>,
        ) -> Result<(), SessionError<ToyError>> {
            self.inner.write_packet(packet, pulse)
        }
        fn pending_output(&self) -> &[u8] {
            self.inner.pending_output()
        }
        fn consume_output(&mut self, n: usize) {
            self.inner.consume_output(n);
        }
        fn take_output(&mut self, into: &mut Vec<u8>) {
            self.inner.take_output(into);
        }
        fn finish(self, pulse: Option<&dyn Pulse>) -> Result<MuxFinish, SessionError<ToyError>> {
            self.inner.finish(pulse)
        }
    }

    struct MutatingTimingEncoder {
        inner: ToyEncoder,
        cfg: CodecConfig,
        mutate_on_send: bool,
        spin_progress_forever: bool,
    }
    impl VideoEncoder for MutatingTimingEncoder {
        type Error = ToyError;
        fn send(
            &mut self,
            frame: &Frame,
            pulse: Option<&dyn Pulse>,
        ) -> Result<SendStatus, SessionError<ToyError>> {
            if self.mutate_on_send {
                self.cfg.timing = Arc::new(
                    TrackTiming::empty().with_codec_delay(FrameDuration::new(9, 30).unwrap()),
                );
            }
            self.inner.send(frame, pulse)
        }
        fn receive(
            &mut self,
            pulse: Option<&dyn Pulse>,
        ) -> Result<Receive<Packet>, SessionError<ToyError>> {
            if self.spin_progress_forever {
                return Ok(Receive::Progress {
                    work_units: NonZeroU32::new(1).unwrap(),
                });
            }
            self.inner.receive(pulse)
        }
        fn end_input(&mut self, pulse: Option<&dyn Pulse>) -> Result<(), SessionError<ToyError>> {
            self.inner.end_input(pulse)
        }
        fn output_config(&self) -> Option<&CodecConfig> {
            Some(&self.cfg)
        }
    }

    let initial_timing = TrackTiming::empty()
        .with_codec_delay(FrameDuration::new(2, 30).unwrap())
        .with_seek_preroll(FrameDuration::new(4, 30).unwrap());
    let base_enc = ToyEncoder::new(2, 2, tb_30(), 0, 1);
    let mut cfg = base_enc.output_config().unwrap().clone();
    cfg.timing = Arc::new(initial_timing.clone());

    // 1. StagedEncode transfers config.timing onto mux TrackInfo and detects timing mutation.
    let captured = Arc::new(std::sync::Mutex::new(None));
    let mut staged = StagedEncode::new(
        MutatingTimingEncoder {
            inner: base_enc,
            cfg: cfg.clone(),
            mutate_on_send: true,
            spin_progress_forever: false,
        },
        TimingCaptureMuxer {
            inner: ToyMuxer::new(false),
            captured_timing: Arc::clone(&captured),
        },
        Forward(Vec::new()),
        PolicySet::new(),
        StorageBudget::new(64 * 1024, 16),
        None,
    )
    .unwrap();
    assert_eq!(
        captured.lock().unwrap().as_ref(),
        Some(&initial_timing),
        "StagedEncode must transfer config.timing to the mux TrackInfo"
    );
    let err = staged.push(make_frame(0, Some(1), 5), None).unwrap_err();
    assert_eq!(
        zencodec::find_cause::<Refusal>(err.error()),
        Some(&Refusal::ConfigurationChanged)
    );

    // 2. Progress-only encoder terminates with Refusal::ProgressLimitExceeded instead of spinning.
    let mut spinning = StagedEncode::new(
        MutatingTimingEncoder {
            inner: ToyEncoder::new(2, 2, tb_30(), 0, 1),
            cfg,
            mutate_on_send: false,
            spin_progress_forever: true,
        },
        TimingCaptureMuxer {
            inner: ToyMuxer::new(false),
            captured_timing: Arc::new(std::sync::Mutex::new(None)),
        },
        Forward(Vec::new()),
        PolicySet::new(),
        StorageBudget::new(64 * 1024, 16),
        None,
    )
    .unwrap();
    let err = spinning.push(make_frame(0, Some(1), 5), None).unwrap_err();
    assert_eq!(
        zencodec::find_cause::<Refusal>(err.error()),
        Some(&Refusal::ProgressLimitExceeded)
    );
}

struct MeterControl {
    tripped: Arc<AtomicBool>,
    units: Arc<AtomicU64>,
}
impl Stop for MeterControl {
    fn check(&self) -> Result<(), StopReason> {
        if self.tripped.load(Ordering::SeqCst) {
            Err(StopReason::Cancelled)
        } else {
            Ok(())
        }
    }
}
impl Report for MeterControl {
    fn advance(&self, n: u64) {
        self.units.fetch_add(n, Ordering::SeqCst);
    }
}
impl Pulse for MeterControl {
    fn split(
        &self,
        e: zencodec::how_far::Execution,
        parts: &[zencodec::how_far::PhaseSpec<'_>],
    ) -> Result<Vec<zencodec::how_far::Child<'_>>, zencodec::how_far::PlanError> {
        zencodec::NoPulse.split(e, parts)
    }
}

struct ProgressDecoder {
    inner: ToyDecoder,
    progress_left: u32,
    infinite_progress: bool,
    receive_calls: Arc<AtomicUsize>,
}

impl VideoDecoder for ProgressDecoder {
    type Error = ToyError;
    fn configure(
        &mut self,
        c: &CodecConfig,
        p: Option<&dyn Pulse>,
    ) -> Result<(), SessionError<ToyError>> {
        self.inner.configure(c, p)
    }
    fn send(
        &mut self,
        pkt: &mut Packet,
        p: Option<&dyn Pulse>,
    ) -> Result<SendStatus, SessionError<ToyError>> {
        self.inner.send(pkt, p)
    }
    fn receive(&mut self, p: Option<&dyn Pulse>) -> Result<Receive<Frame>, SessionError<ToyError>> {
        self.receive_calls.fetch_add(1, Ordering::SeqCst);
        if self.infinite_progress {
            return Ok(Receive::Progress {
                work_units: NonZeroU32::new(1).unwrap(),
            });
        }
        if self.progress_left > 0 {
            self.progress_left -= 1;
            return Ok(Receive::Progress {
                work_units: NonZeroU32::new(3).unwrap(),
            });
        }
        self.inner.receive(p)
    }
    fn end_input(&mut self, p: Option<&dyn Pulse>) -> Result<(), SessionError<ToyError>> {
        self.inner.end_input(p)
    }
    fn reset(&mut self, p: Option<&dyn Pulse>) -> Result<(), SessionError<ToyError>> {
        self.inner.reset(p)
    }
    fn stream_info(&self) -> Option<&VideoStreamInfo> {
        self.inner.stream_info()
    }
}

#[test]
fn receive_progress_accounts_work_units_caps_spin_and_polls_after_callback() {
    let cfg = CodecConfig::new(CodecId::Other(CodecTag::new("toy")), tb_30())
        .with_video(VideoTrackInfo::new(2, 2));

    // 1. Finite progress forwards exact work_units to Pulse::advance.
    let units = Arc::new(AtomicU64::new(0));
    let tripped = Arc::new(AtomicBool::new(false));
    let meter = MeterControl {
        tripped: Arc::clone(&tripped),
        units: Arc::clone(&units),
    };
    let mut dec = ProgressDecoder {
        inner: ToyDecoder::new(2),
        progress_left: 2,
        infinite_progress: false,
        receive_calls: Arc::new(AtomicUsize::new(0)),
    };
    dec.configure(&cfg, Some(&meter)).unwrap();
    let mut pkt = Packet::from_vec(toy_payload(&[&[1, 2, 3, 4]], false))
        .unwrap()
        .with_pts(Timestamp::new(0, tb_30()));
    let mut decoded = Vec::new();
    send_packet_until_accepted(&mut dec, &mut pkt, Some(&meter), |f| {
        decoded.push(f);
        Ok::<(), ()>(())
    })
    .unwrap();
    drain_decoder(&mut dec, Some(&meter), |f| {
        decoded.push(f);
        Ok::<(), ()>(())
    })
    .unwrap();
    assert_eq!(decoded.len(), 1);
    assert_eq!(units.load(Ordering::SeqCst), 6); // 2 * 3 work_units

    // 2. Progress-only backend terminates promptly with ProtocolViolation::ProgressLimitExceeded.
    let mut spin_dec = ProgressDecoder {
        inner: ToyDecoder::new(2),
        progress_left: 0,
        infinite_progress: true,
        receive_calls: Arc::new(AtomicUsize::new(0)),
    };
    spin_dec.configure(&cfg, Some(&NoPulse)).unwrap();
    let mut pkt2 = Packet::from_vec(toy_payload(&[&[1, 2, 3, 4]], false))
        .unwrap()
        .with_pts(Timestamp::new(0, tb_30()));
    let err = send_packet_until_accepted(&mut spin_dec, &mut pkt2, Some(&NoPulse), |_| {
        Ok::<(), ()>(())
    })
    .unwrap_err();
    assert!(matches!(
        err,
        PumpError::Upstream(SessionError::Protocol(
            ProtocolViolation::ProgressLimitExceeded
        ))
    ));

    // 3. Output callback that trips the pulse stops IMMEDIATELY after the callback without
    //    polling receive() again.
    let cb_tripped = Arc::new(AtomicBool::new(false));
    let cb_pulse = MeterControl {
        tripped: Arc::clone(&cb_tripped),
        units: Arc::new(AtomicU64::new(0)),
    };
    let calls = Arc::new(AtomicUsize::new(0));
    let mut dec3 = ProgressDecoder {
        inner: ToyDecoder::new(4),
        progress_left: 0,
        infinite_progress: false,
        receive_calls: Arc::clone(&calls),
    };
    dec3.configure(&cfg, Some(&cb_pulse)).unwrap();
    let mut multi = Packet::from_vec(toy_payload(&[&[1, 2, 3, 4], &[5, 6, 7, 8]], false))
        .unwrap()
        .with_pts(Timestamp::new(0, tb_30()));
    // Send two packets so drain yields multiple frames.
    send_packet_until_accepted(&mut dec3, &mut multi, Some(&cb_pulse), |_| Ok::<(), ()>(()))
        .unwrap();
    calls.store(0, Ordering::SeqCst);
    let err = drain_decoder(&mut dec3, Some(&cb_pulse), |_f| {
        cb_tripped.store(true, Ordering::SeqCst);
        Ok::<(), ()>(())
    })
    .unwrap_err();
    assert!(matches!(
        err,
        PumpError::Upstream(SessionError::Cancelled(StopReason::Cancelled))
    ));
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "must stop immediately after callback trips pulse without calling receive again"
    );

    // 4. Encoder pump works with finite progress as well.
    let mut enc = ToyEncoder::new(2, 2, tb_30(), 1, 1);
    let mut encoded = Vec::new();
    send_frame_until_accepted(&mut enc, &make_frame(0, Some(1), 7), Some(&NoPulse), |p| {
        encoded.push(p);
        Ok::<(), ()>(())
    })
    .unwrap();
    drain_encoder(&mut enc, Some(&NoPulse), |p| {
        encoded.push(p);
        Ok::<(), ()>(())
    })
    .unwrap();
    assert_eq!(encoded.len(), 1);
}

struct SwitchStop(Arc<AtomicBool>);
impl Stop for SwitchStop {
    fn check(&self) -> Result<(), StopReason> {
        if self.0.load(Ordering::SeqCst) {
            Err(StopReason::TimedOut)
        } else {
            Ok(())
        }
    }
}

#[test]
fn end_input_and_reset_observe_pulse_cancellation_through_erased_shims() {
    let tripped = Arc::new(AtomicBool::new(true));
    let stop = StopOnly::new(SwitchStop(Arc::clone(&tripped)));

    let mut dec = erase_video_decoder(ToyDecoder::new(2));
    assert!(matches!(
        dec.end_input(Some(&stop)),
        Err(SessionError::Cancelled(StopReason::TimedOut))
    ));
    assert!(matches!(
        dec.reset(Some(&stop)),
        Err(SessionError::Cancelled(StopReason::TimedOut))
    ));
    tripped.store(false, Ordering::SeqCst);
    dec.reset(Some(&stop)).unwrap();

    tripped.store(true, Ordering::SeqCst);
    let mut enc = erase_video_encoder(ToyEncoder::new(2, 2, tb_30(), 0, 1));
    assert!(matches!(
        enc.end_input(Some(&stop)),
        Err(SessionError::Cancelled(StopReason::TimedOut))
    ));

    // Cancellation tripped during successful `end_input` is observed BEFORE calling `receive`!
    struct TripOnEndInput {
        inner: ToyDecoder,
        tripped: Arc<AtomicBool>,
        receive_calls: Arc<AtomicUsize>,
    }
    impl VideoDecoder for TripOnEndInput {
        type Error = ToyError;
        fn configure(
            &mut self,
            c: &CodecConfig,
            p: Option<&dyn Pulse>,
        ) -> Result<(), SessionError<ToyError>> {
            self.inner.configure(c, p)
        }
        fn send(
            &mut self,
            pkt: &mut Packet,
            p: Option<&dyn Pulse>,
        ) -> Result<SendStatus, SessionError<ToyError>> {
            self.inner.send(pkt, p)
        }
        fn receive(
            &mut self,
            p: Option<&dyn Pulse>,
        ) -> Result<Receive<Frame>, SessionError<ToyError>> {
            self.receive_calls.fetch_add(1, Ordering::SeqCst);
            self.inner.receive(p)
        }
        fn end_input(&mut self, p: Option<&dyn Pulse>) -> Result<(), SessionError<ToyError>> {
            self.inner.end_input(p)?;
            self.tripped.store(true, Ordering::SeqCst);
            Ok(())
        }
        fn reset(&mut self, p: Option<&dyn Pulse>) -> Result<(), SessionError<ToyError>> {
            self.inner.reset(p)
        }
        fn stream_info(&self) -> Option<&VideoStreamInfo> {
            self.inner.stream_info()
        }
    }

    let drain_tripped = Arc::new(AtomicBool::new(false));
    let drain_calls = Arc::new(AtomicUsize::new(0));
    let drain_stop = StopOnly::new(SwitchStop(Arc::clone(&drain_tripped)));
    let mut d = TripOnEndInput {
        inner: ToyDecoder::new(2),
        tripped: Arc::clone(&drain_tripped),
        receive_calls: Arc::clone(&drain_calls),
    };
    let cfg = CodecConfig::new(CodecId::Other(CodecTag::new("toy")), tb_30())
        .with_video(VideoTrackInfo::new(2, 2));
    d.configure(&cfg, Some(&drain_stop)).unwrap();
    let mut pkt = Packet::from_vec(toy_payload(&[&[1, 2, 3, 4]], false))
        .unwrap()
        .with_pts(Timestamp::new(0, tb_30()));
    send_packet_until_accepted(&mut d, &mut pkt, Some(&drain_stop), |_| Ok::<(), ()>(())).unwrap();
    drain_calls.store(0, Ordering::SeqCst);
    let err = drain_decoder(&mut d, Some(&drain_stop), |_| Ok::<(), ()>(())).unwrap_err();
    assert!(matches!(
        err,
        PumpError::Upstream(SessionError::Cancelled(StopReason::TimedOut))
    ));
    assert_eq!(
        drain_calls.load(Ordering::SeqCst),
        0,
        "drain_decoder must observe cancellation raised during end_input before polling receive"
    );
}

struct SeekableToy(ToyDemuxer);
impl Demuxer for SeekableToy {
    type Error = ToyError;
    fn step(
        &mut self,
        input: Input<'_>,
        pulse: Option<&dyn Pulse>,
    ) -> Result<zencodec_media::container::DemuxProgress, SessionError<ToyError>> {
        self.0.step(input, pulse)
    }
    fn requested(&self) -> &[zencodec::decode::InputRequest] {
        self.0.requested()
    }
    fn tracks(&self) -> &[TrackInfo] {
        self.0.tracks()
    }
    fn seek(&mut self, target: &SeekTarget) -> Result<SeekStatus, SessionError<ToyError>> {
        self.0.seek(target)
    }
}
impl SeekableDemuxer for SeekableToy {
    fn seek_offer(&self, _target: &SeekTarget) -> Result<SeekOffer, SessionError<ToyError>> {
        Ok(
            SeekOffer::new(AccessPattern::RandomAccess, NativeBound::UpperBound(1024))
                .with_requires_index(true),
        )
    }
    fn begin_seek(
        &mut self,
        target: &SeekTarget,
        pulse: Option<&dyn Pulse>,
    ) -> Result<SeekStatus, SessionError<ToyError>> {
        if let Some(p) = pulse {
            p.check()?;
        }
        self.0.seek(target)
    }
}

struct ToyDemuxFactory;
impl DemuxFactory for ToyDemuxFactory {
    fn probe(&self, input: Input<'_>, _caps: &ProbeLimits) -> Result<ContainerProbe, MediaError> {
        if input.bytes.len() < 4 {
            return Ok(ContainerProbe::Need { min_bytes: 4 });
        }
        if &input.bytes[..4] == b"TIVF" {
            Ok(ContainerProbe::Confirmed {
                container: ContainerId::Custom(zencodec_media::registry::ContainerTag::new("tivf")),
                confidence: 95,
            })
        } else {
            Ok(ContainerProbe::NoMatch)
        }
    }
    fn quote(
        &self,
        _need: &DemuxNeeds,
        source: &SourceAccess,
        _run: &JobResources,
    ) -> Result<DemuxOffer, MediaError> {
        let mut offer = DemuxOffer::new(
            ContainerId::Custom(zencodec_media::registry::ContainerTag::new("tivf")),
            AccessPattern::Sequential,
        )
        .with_workspace(NativeBound::UpperBound(4096));
        if source.satisfies(AccessPattern::RandomAccess) {
            offer = offer.with_seek_offer(SeekOffer::new(
                AccessPattern::RandomAccess,
                NativeBound::UpperBound(1024),
            ));
        }
        Ok(offer)
    }
    fn open(
        &self,
        _offer: &DemuxOffer,
        _run: &JobResources,
    ) -> Result<Box<dyn DynDemuxer + Send>, MediaError> {
        Ok(erase_seekable_demuxer(SeekableToy(ToyDemuxer::new(
            MediaLimits::default(),
        ))))
    }
}

struct CreditToyMuxer {
    inner: ToyMuxer,
    offer: MuxOffer,
    cap: usize,
}
impl DynMuxer for CreditToyMuxer {
    fn begin(
        &mut self,
        tracks: &[TrackInfo],
        hint: MuxHint,
    ) -> Result<MuxTiming, SessionError<ErasedError>> {
        self.inner
            .begin(tracks, hint)
            .map_err(|e| e.map_codec(|c| ErasedError::new(Box::new(c))))
    }
    fn write_packet(
        &mut self,
        packet: Packet,
        pulse: Option<&dyn Pulse>,
    ) -> Result<(), SessionError<ErasedError>> {
        self.inner
            .write_packet(packet, pulse)
            .map_err(|e| e.map_codec(|c| ErasedError::new(Box::new(c))))
    }
    fn pending_output(&self) -> &[u8] {
        self.inner.pending_output()
    }
    fn consume_output(&mut self, n: usize) {
        self.inner.consume_output(n);
    }
    fn take_output(&mut self, into: &mut Vec<u8>) {
        self.inner.take_output(into);
    }
    fn finish(
        self: Box<Self>,
        pulse: Option<&dyn Pulse>,
    ) -> Result<MuxFinish, SessionError<ErasedError>> {
        self.inner
            .finish(pulse)
            .map_err(|e| e.map_codec(|c| ErasedError::new(Box::new(c))))
    }
}
impl BoundedMuxer for CreditToyMuxer {
    fn admitted_offer(&self) -> &MuxOffer {
        &self.offer
    }
    fn try_write_packet(
        &mut self,
        packet: &mut Option<Packet>,
        pulse: Option<&dyn Pulse>,
    ) -> Result<PacketWrite, SessionError<ErasedError>> {
        let Some(pkt_ref) = packet.as_ref() else {
            return Err(SessionError::Protocol(ProtocolViolation::EmptyPacket));
        };
        // TIVF packet overhead is 13 bytes + payload.len().
        let next_bytes = 13usize.saturating_add(pkt_ref.len());
        let pending = self.inner.pending_output().len();
        if pending > 0 && pending.saturating_add(next_bytes) > self.cap {
            return Ok(PacketWrite::DrainOutput);
        }
        let pkt = packet.take().expect("checked Some above");
        self.write_packet(pkt, pulse)?;
        Ok(PacketWrite::Accepted)
    }
}

struct ToyBoundedMuxFactory {
    cap: usize,
}
impl MuxFactory for ToyBoundedMuxFactory {
    fn quote(&self, need: &MuxNeeds, _run: &JobResources) -> Result<MuxPlan, MediaError> {
        let offer = MuxOffer::new(
            MuxTiming::streaming(),
            NativeBound::UpperBound(self.cap as u64),
            NativeBound::UpperBound(128),
            NativeBound::Exact(0),
        );
        let mut plan = MuxPlan::new(
            ContainerId::Custom(zencodec_media::registry::ContainerTag::new("tivf")),
            Arc::clone(&need.tracks),
            need.destination,
            need.timing,
            offer,
        );
        plan.supports_bounded_writes = true;
        Ok(plan)
    }
    fn open(
        &self,
        plan: &MuxPlan,
        _run: &JobResources,
    ) -> Result<Box<dyn DynMuxer + Send>, MediaError> {
        Ok(erase_bounded_muxer(CreditToyMuxer {
            inner: ToyMuxer::new(false),
            offer: plan.offer,
            cap: self.cap,
        }))
    }
}

#[test]
fn media_roles_unquoted_seek_refusal_and_routed_bounded_muxer_partial_drain() {
    let cid = ContainerId::Custom(zencodec_media::registry::ContainerTag::new("tivf"));
    let set = CodecSet::new()
        .with_role::<DemuxRole>(
            [cid.clone()],
            NamedFactory::<DemuxRole>::try_new(
                "org.imazen.tivf.toy",
                "0.1.0",
                Arc::new(ToyDemuxFactory),
            )
            .unwrap(),
        )
        .unwrap()
        .with_role::<MuxRole>(
            [cid.clone()],
            NamedFactory::<MuxRole>::try_new(
                "org.imazen.tivf.bounded_mux",
                "0.1.0",
                Arc::new(ToyBoundedMuxFactory { cap: 30 }),
            )
            .unwrap(),
        )
        .unwrap();

    let factories: Vec<_> = set.factories::<DemuxRole>(&cid).collect();
    assert_eq!(factories.len(), 1);
    let fac = &factories[0].factory;
    assert_eq!(
        fac.probe(Input::new(0, b"TI"), &ProbeLimits::default())
            .unwrap(),
        ContainerProbe::Need { min_bytes: 4 }
    );
    assert_eq!(
        fac.probe(Input::new(0, b"TIVFxxxx"), &ProbeLimits::default())
            .unwrap(),
        ContainerProbe::Confirmed {
            container: cid.clone(),
            confidence: 95
        }
    );

    let run = JobResources::new(
        ResourceLimits::none(),
        StorageBudget::new(4096, 8),
        SharedPulse::default(),
    );
    let offer = fac
        .quote(&DemuxNeeds::default(), &SourceAccess::borrowed(100), &run)
        .unwrap();
    assert!(offer.seek_offer.is_some());
    let dem = fac.open(&offer, &run).unwrap();
    let target = SeekTarget::new(
        0,
        Timestamp::new(0, tb_30()),
        zencodec_media::container::SeekMode::Nearest,
        zencodec_media::container::SeekScope::Any,
    );
    let so = dem.seek_offer(&target).unwrap();
    assert_eq!(so.required_access, AccessPattern::RandomAccess);
    assert!(so.requires_index);

    // Non-seekable erased demuxer MUST refuse BOTH seek_offer AND begin_seek even though
    // ToyDemuxer implements legacy Demuxer::seek.
    let mut plain = erase_demuxer(ToyDemuxer::new(MediaLimits::default()));
    assert!(matches!(
        plain.seek_offer(&target),
        Err(MediaError::Unsupported(UnsupportedMedia::Seek))
    ));
    assert!(matches!(
        plain.begin_seek(&target, Some(&NoPulse)),
        Err(MediaError::Unsupported(UnsupportedMedia::Seek))
    ));

    // Routed MuxFactory lookup -> quote -> open -> as_bounded_mut -> partial drain -> retry.
    let track = TrackInfo::video(
        0,
        CodecId::Other(CodecTag::new("toy")),
        tb_30(),
        VideoTrackInfo::new(2, 2),
    )
    .build()
    .unwrap();
    let mux_fac = &set.factories::<MuxRole>(&cid).next().unwrap().factory;
    let plan = mux_fac
        .quote(
            &MuxNeeds::new([track.clone()], MuxHint::forward_only())
                .with_timing(TimingPolicy::BestEffort),
            &run,
        )
        .unwrap();
    assert!(plan.supports_bounded_writes);

    let mut dyn_mux = mux_fac.open(&plan, &run).unwrap();
    dyn_mux.begin(&[track], MuxHint::forward_only()).unwrap();

    // Header is 20 bytes < cap (30), and next packet is 13 + 6 = 19 bytes -> 20 + 19 = 39 > 30!
    // Must return DrainOutput even though pending (20) < cap (30)!
    let mut pkt = Some(
        Packet::from_vec(toy_payload(&[&[1, 2, 3, 4]], false))
            .unwrap()
            .with_pts(Timestamp::new(0, tb_30())),
    );
    let bounded = dyn_mux
        .as_bounded_mut()
        .expect("routed muxer must preserve BoundedMuxer access");
    assert_eq!(
        bounded.admitted_offer().maximum_pending,
        NativeBound::UpperBound(30)
    );
    assert_eq!(bounded.pending_output().len(), 20);
    assert_eq!(
        bounded.try_write_packet(&mut pkt, Some(&NoPulse)).unwrap(),
        PacketWrite::DrainOutput
    );
    assert!(pkt.is_some(), "DrainOutput must leave packet untouched");

    // Partial drain of 10 bytes -> pending = 10; 10 + 19 = 29 <= 30 -> retry succeeds!
    bounded.consume_output(10);
    assert_eq!(bounded.pending_output().len(), 10);
    assert_eq!(
        bounded.try_write_packet(&mut pkt, Some(&NoPulse)).unwrap(),
        PacketWrite::Accepted
    );
    assert!(pkt.is_none());
    assert_eq!(bounded.pending_output().len(), 29);

    let finish = dyn_mux.finish(Some(&NoPulse)).unwrap();
    assert_eq!(finish.tail.len(), 29);
}

#[test]
fn track_timing_default_is_empty() {
    let t = TrackTiming::default();
    assert!(t.is_empty());
}
