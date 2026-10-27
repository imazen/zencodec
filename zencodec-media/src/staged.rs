//! Finite video encoding with no writer waits while the encoder is alive.
//!
//! The operation owns the encoder and actual prepared container/destination.
//! Every frame is resolved through policy before conversion and native input.
//! Encoded payloads are copied into admitted storage. Finish drains and drops
//! the encoder before publishing any bytes. This intentionally stages the full
//! compressed sequence; it is not a live streaming/backpressure mechanism.
//! Caller-held pictures, codec/CMS workspace and bookkeeping are outside the
//! payload budget. Unsupported timing/layout fails before the first frame.
//! ```
//! use zencodec::{frame::Frame, operation::PolicySet, storage::StorageBudget};
//! use zencodec_media::{container::Muxer, output::Forward, session::VideoEncoder,
//!     staged::StagedEncode};
//! fn encode<E: VideoEncoder, M: Muxer, W: std::io::Write>(
//!     encoder: E, mux: M, writer: W, frames: impl IntoIterator<Item = Frame>,
//! ) -> Result<(), Box<dyn core::error::Error>> {
//!     let stop = enough::Unstoppable;
//!     let mut job = StagedEncode::new(encoder, mux, Forward(writer),
//!         PolicySet::new(), StorageBudget::new(64 * 1024 * 1024, 4096), &stop)?;
//!     for frame in frames { job.push(frame, &stop)?; }
//!     let completed = job.finish(&stop)?;
//!     let _ = (completed.stats, completed.report);
//!     Ok(())
//! }
//! ```
use crate::{
    container::Muxer,
    output::{Destination, PreparedOutput, Stats},
    packet::Packet,
    session::{CodecConfig, Receive, SendStatus, VideoEncoder},
    track::TrackInfo,
};
use alloc::{boxed::Box, vec::Vec};
use core::{error::Error as StdError, fmt};
use enough::Stop;
use zencodec::{
    At,
    frame::{Frame, FrameInterpretation},
    operation::{
        FrameConversionOffer, FrameFacts, FrameRoute, FrameRouteOffer, FrameRouteState,
        OperationReport, PolicyResolver, PolicySet, resolve_frame_route,
    },
    storage::StorageBudget,
};

/// Concrete source, native codec, conversion, policy or destination failure.
pub type Cause = Box<dyn StdError + Send + Sync>;

/// Boundary at which the owned operation stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Stage {
    /// Prepare the container from the actual encoder configuration.
    Prepare,
    /// Resolve the held frame and conversion through application policy.
    Policy,
    /// Execute and validate the selected conversion.
    Convert,
    /// Send, receive or stage native encoded packets.
    Encode,
    /// Drain and release the encoder.
    Finish,
    /// Publish the container; partial effects remain in the cause.
    Publish,
    /// An operation already failed or unwound.
    Terminal,
}
/// Failure with bounded policy history and a retained concrete cause.
/// Default formatting omits arbitrary backend and application payloads.
pub struct Failure {
    /// Failed boundary.
    pub stage: Stage,
    /// Decisions up to this failure.
    pub report: Box<OperationReport>,
    cause: Cause,
}
impl fmt::Debug for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StagedEncodeFailure")
            .field("stage", &self.stage)
            .field("report", &self.report)
            .finish_non_exhaustive()
    }
}
impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "staged encode failed during {:?}", self.stage)
    }
}
impl StdError for Failure {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        Some(self.cause.as_ref())
    }
}
/// Typed refusal of the fixed-configuration staging contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Refusal {
    /// Encoder cannot declare its container configuration before input.
    ConfigurationUnavailable,
    /// Configuration does not describe video.
    VideoConfigurationRequired,
    /// Conversion's bounded facts disagree with its full target.
    OfferMismatch,
    /// Conversion returned different interpretation or timing.
    ConversionMismatch,
    /// Encoder promised pending output but did not return a packet.
    ReceivePendingWithoutPacket,
    /// Encoder ended before the caller finished input.
    EarlyEnd,
    /// Encoder changed the configuration bound to the prepared container.
    ConfigurationChanged,
    /// Encoder requested input after end_input.
    InputAfterEnd,
    /// Encoder returned an empty or taken packet.
    EmptyPacket,
    /// Operation previously failed or unwound.
    Terminal,
}
impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "staged encode refusal: {self:?}")
    }
}
impl StdError for Refusal {}
fn invalid(reason: Refusal) -> Cause {
    Box::new(reason)
}
fn check(stop: &dyn Stop) -> Result<(), Cause> {
    stop.check()
        .map_err(|reason| At::wrap(zencodec::CodecError::new(None, reason.into())).into())
}
fn same_config(a: &CodecConfig, b: &CodecConfig) -> bool {
    a.codec == b.codec
        && a.private == b.private
        && a.epoch == b.epoch
        && a.time_base == b.time_base
        && a.video == b.video
}
struct State<E, M, D> {
    encoder: E,
    output: PreparedOutput<M, D>,
    config: CodecConfig,
    packets: Vec<Packet>,
    previous: Option<FrameFacts>,
}
/// Completed output and its bounded application decision history.
#[derive(Debug)]
pub struct Completed {
    /// Confirmed destination effects.
    pub stats: Stats,
    /// Decisions made before accepting frames.
    pub report: OperationReport,
}

/// An owned, fixed-configuration video-to-container operation.
///
/// Failure or unwind makes it terminal and drops encoder, staged packets and
/// private destination preparation. Neither push nor drop writes output. Finish
/// consumes the operation and releases native state before the first write.
/// No implicit segmentation, timing rewrite or fallback is performed.
pub struct StagedEncode<E, M, D, R> {
    state: Option<State<E, M, D>>,
    resolver: R,
    preset: PolicySet,
    storage: StorageBudget,
    report: OperationReport,
}
impl<E: VideoEncoder, M: Muxer, D: Destination, R: PolicyResolver> StagedEncode<E, M, D, R> {
    /// Retain the encoder and prepare exact-timing output from its actual config.
    /// This single-video-track operation requires configuration before input.
    pub fn new(
        encoder: E,
        mux: M,
        destination: D,
        resolver: R,
        storage: StorageBudget,
        stop: &dyn Stop,
    ) -> Result<Self, At<Failure>> {
        let prepare = || -> Result<_, Cause> {
            check(stop)?;
            let config = encoder
                .output_config()
                .ok_or_else(|| invalid(Refusal::ConfigurationUnavailable))?
                .clone();
            let video = config
                .video
                .ok_or_else(|| invalid(Refusal::VideoConfigurationRequired))?;
            let mut track = TrackInfo::video(0, config.codec.clone(), config.time_base, video)
                .with_config_epoch(config.epoch);
            if let Some(private) = &config.private {
                track = track.with_codec_private(private.clone());
            }
            let output = PreparedOutput::new(mux, destination, &[track.build()?], stop)?;
            Ok(State {
                encoder,
                output,
                config,
                packets: Vec::new(),
                previous: None,
            })
        };
        let state = prepare().map_err(|cause| {
            At::wrap(Failure {
                stage: Stage::Prepare,
                report: Box::default(),
                cause,
            })
        })?;
        Ok(Self {
            state: Some(state),
            resolver,
            preset: PolicySet::new(),
            storage,
            report: OperationReport::default(),
        })
    }
    /// Preset consulted when the resolver returns `UsePreset`.
    pub fn with_policy_set(mut self, preset: PolicySet) -> Self {
        self.preset = preset;
        self
    }
    /// Bounded decisions, including after a terminal failure.
    pub fn report(&self) -> &OperationReport {
        &self.report
    }
    /// Whether failure or unwind has made this operation unusable.
    pub fn is_failed(&self) -> bool {
        self.state.is_none()
    }
    fn failure(&self, stage: Stage, cause: Cause) -> At<Failure> {
        At::wrap(Failure {
            stage,
            report: Box::new(self.report.clone()),
            cause,
        })
    }
    /// Send the exact held picture without conversion or timing changes.
    /// Encoder fidelity and native workspace remain unqualified.
    pub fn push(&mut self, input: Frame, stop: &dyn Stop) -> Result<(), At<Failure>> {
        let target = input.interpretation().clone();
        let mut offer = FrameConversionOffer::new(FrameFacts::new(&target, input.timing()));
        offer.preserves_samples = true;
        offer.preserves_extended_range = true;
        self.push_converted(input, &target, offer, Ok, stop)
    }
    /// Retain this input and selected converter through policy, then execute once.
    /// `target` is the complete required interpretation; timing comes from the
    /// offer. Output must match both exactly before reaching the encoder. The
    /// closure owns its prepared conversion/reservations; it must not publish.
    pub fn push_converted(
        &mut self,
        input: Frame,
        target: &FrameInterpretation,
        conversion: FrameConversionOffer,
        execute: impl FnOnce(Frame) -> Result<Frame, Cause>,
        stop: &dyn Stop,
    ) -> Result<(), At<Failure>> {
        let mut state = self
            .state
            .take()
            .ok_or_else(|| self.failure(Stage::Terminal, invalid(Refusal::Terminal)))?;
        let mut stage = Stage::Policy;
        let run = || -> Result<(), Cause> {
            check(stop)?;
            if conversion.encoder_input != FrameFacts::new(target, conversion.encoder_input.timing)
            {
                return Err(invalid(Refusal::OfferMismatch));
            }
            let mut offer = FrameRouteOffer::new(if state.previous.is_none() {
                FrameRoute::BeginSegment
            } else {
                FrameRoute::Continue
            });
            offer.conversion = Some(conversion);
            offer.destination = Some(state.output.facts());
            offer.admitted_encoded_bytes = self.storage.reserved_bytes();
            let mut progress = FrameRouteState::default();
            progress.previous = state.previous;
            match resolve_frame_route(
                input.interpretation(),
                input.timing(),
                progress,
                &[offer],
                &mut self.resolver,
                self.preset,
                stop,
            ) {
                Ok((_, record)) => self.report.record(record),
                Err(error) => {
                    self.report.record(error.error().record());
                    return Err(error.into());
                }
            }
            stage = Stage::Convert;
            let previous = FrameFacts::new(input.interpretation(), input.timing());
            let converted = execute(input)?;
            check(stop)?;
            if converted.interpretation().as_ref() != target
                || converted.timing() != conversion.encoder_input.timing
            {
                return Err(invalid(Refusal::ConversionMismatch));
            }
            stage = Stage::Encode;
            loop {
                check(stop)?;
                match state.encoder.send(&converted, stop)? {
                    SendStatus::Accepted => break,
                    SendStatus::ReceivePending => match state.encoder.receive(stop)? {
                        Receive::Ready(packet) => {
                            stage_packet(&mut state.packets, packet, &self.storage, &state.output)?
                        }
                        _ => return Err(invalid(Refusal::ReceivePendingWithoutPacket)),
                    },
                }
            }
            loop {
                check(stop)?;
                match state.encoder.receive(stop)? {
                    Receive::Ready(packet) => {
                        stage_packet(&mut state.packets, packet, &self.storage, &state.output)?
                    }
                    Receive::NeedInput => break,
                    Receive::EndOfStream => return Err(invalid(Refusal::EarlyEnd)),
                }
            }
            if !state
                .encoder
                .output_config()
                .is_some_and(|config| same_config(config, &state.config))
            {
                return Err(invalid(Refusal::ConfigurationChanged));
            }
            state.previous = Some(previous);
            check(stop)
        };
        run().map_err(|cause| self.failure(stage, cause))?;
        self.state = Some(state);
        Ok(())
    }
    /// Drain, release all owned encoder state, then publish the retained output.
    /// Writer failure may leave partial bytes; its concrete output failure and
    /// confirmed append count remain in the error chain. No retry is performed.
    pub fn finish(mut self, stop: &dyn Stop) -> Result<Completed, At<Failure>> {
        let mut state = self
            .state
            .take()
            .ok_or_else(|| self.failure(Stage::Terminal, invalid(Refusal::Terminal)))?;
        let mut drain = || -> Result<(), Cause> {
            check(stop)?;
            state.encoder.end_input()?;
            loop {
                check(stop)?;
                match state.encoder.receive(stop)? {
                    Receive::Ready(packet) => {
                        stage_packet(&mut state.packets, packet, &self.storage, &state.output)?
                    }
                    Receive::EndOfStream => break,
                    Receive::NeedInput => {
                        return Err(invalid(Refusal::InputAfterEnd));
                    }
                }
            }
            if !state
                .encoder
                .output_config()
                .is_some_and(|config| same_config(config, &state.config))
            {
                return Err(invalid(Refusal::ConfigurationChanged));
            }
            check(stop)
        };
        drain().map_err(|cause| self.failure(Stage::Finish, cause))?;
        let State {
            encoder,
            output,
            packets,
            ..
        } = state;
        drop(encoder);
        let publish = || -> Result<Stats, Cause> {
            let mut output = output.publish(stop)?;
            for packet in packets {
                output.send(packet, stop)?;
            }
            Ok(output.finish(stop)?)
        };
        let stats = publish().map_err(|cause| self.failure(Stage::Publish, cause))?;
        Ok(Completed {
            stats,
            report: self.report,
        })
    }
}
fn stage_packet<M: Muxer, D: Destination>(
    packets: &mut Vec<Packet>,
    packet: Packet,
    storage: &StorageBudget,
    output: &PreparedOutput<M, D>,
) -> Result<(), Cause> {
    output.validate_packet(&packet)?;
    if packet.is_empty() {
        return Err(invalid(Refusal::EmptyPacket));
    }
    packets.try_reserve(1)?;
    packets.push(packet.accounted(storage)?);
    Ok(())
}
