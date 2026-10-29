//! Native witness for output transitions. Fixed AV1 settings per segment;
//! finish/drop native encoders before exposing admitted compressed output.
use enough::Stop;
use std::{
    error::Error,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use zenavif::media::{Av1EncodeSettings, Av1VideoEncoder};
use zencodec::how_far::NoPulse;
use zencodec::{
    At, CodecError,
    encode::Fidelity,
    frame::{ChromaSiting, Frame, FrameGeometry, FrameInterpretation, FrameTiming, Subsampling},
    operation::{
        DestinationFacts, FrameConversionOffer, FrameFacts, FrameRoute, FrameRouteOffer,
        FrameRouteState, OperationPolicy, OperationReport, PolicySet, resolve_frame_route,
    },
    storage::StorageBudget,
    time::TimeBase,
};
use zencodec_media::{
    packet::Packet,
    session::{CodecConfig, Receive, SendStatus, VideoEncoder},
};
use zenpixels::Cicp;

type Cause = Box<dyn Error + Send + Sync>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Key {
    width: u32,
    height: u32,
    bits: u8,
    sub: Subsampling,
    siting: ChromaSiting,
    color: Cicp,
    clock: TimeBase,
}
impl Key {
    fn from_input(input: &FrameInterpretation, timing: FrameTiming) -> Result<Self, Cause> {
        let geometry = input.geometry;
        if geometry != FrameGeometry::full(geometry.coded_width, geometry.coded_height)
            || input.encoding.bit_shift() != 0
        {
            return Err(
                std::io::Error::other("native route requires full canonical coded planes").into(),
            );
        }
        Ok(Self {
            width: geometry.coded_width,
            height: geometry.coded_height,
            bits: input.encoding.code_bits(),
            sub: input.subsampling,
            siting: input.siting,
            color: input.color.resolved.ok_or_else(|| {
                std::io::Error::other("unknown color requires an explicit interpretation route")
            })?,
            clock: timing
                .pts
                .ok_or_else(|| std::io::Error::other("native route requires PTS"))?
                .base(),
        })
    }
    fn settings(self) -> Av1EncodeSettings {
        Av1EncodeSettings::new(self.width, self.height)
            .with_bit_depth(self.bits)
            .with_subsampling(self.sub)
            .with_siting(self.siting)
            .with_color(self.color)
            .with_quantizer(0)
            .with_speed(10)
            .with_threads(1)
            .with_key_frame_interval(4)
    }
}

struct Native {
    encoder: Av1VideoEncoder,
    live: Arc<AtomicUsize>,
}
impl Native {
    fn new(key: Key, live: Arc<AtomicUsize>) -> Result<Self, Cause> {
        let encoder = Av1VideoEncoder::new(key.settings(), key.clock)?;
        live.fetch_add(1, Ordering::SeqCst);
        Ok(Self { encoder, live })
    }
}
impl Drop for Native {
    fn drop(&mut self) {
        self.live.fetch_sub(1, Ordering::SeqCst);
    }
}

pub struct Segment {
    pub config: CodecConfig,
    pub settings: Av1EncodeSettings,
    pub packets: Vec<Packet>,
    pub inputs: Vec<FrameTiming>,
}

struct State {
    key: Option<Key>,
    native: Option<Native>,
    packets: Vec<Packet>,
    inputs: Vec<FrameTiming>,
    segments: Vec<Segment>,
    previous: Option<FrameFacts>,
}

pub struct Candidate<R> {
    state: Option<State>,
    resolver: R,
    preset: PolicySet,
    segmented: bool,
    committed: bool,
    storage: StorageBudget,
    live: Arc<AtomicUsize>,
    report: OperationReport,
}
impl<R: OperationPolicy> Candidate<R> {
    pub fn new(
        resolver: R,
        segmented: bool,
        committed: bool,
        storage: StorageBudget,
        live: Arc<AtomicUsize>,
    ) -> Self {
        Self {
            state: Some(State {
                key: None,
                native: None,
                packets: Vec::new(),
                inputs: Vec::new(),
                segments: Vec::new(),
                previous: None,
            }),
            resolver,
            preset: PolicySet::new(),
            segmented,
            committed,
            storage,
            live,
            report: OperationReport::default(),
        }
    }
    pub fn with_policy_set(mut self, preset: PolicySet) -> Self {
        self.preset = preset;
        self
    }
    pub fn report(&self) -> &OperationReport {
        &self.report
    }
    pub fn is_failed(&self) -> bool {
        self.state.is_none()
    }
    /// Exercise the irreversible header boundary with the actual live encoder
    /// configuration. The publisher can have partial effects on error or unwind;
    /// neither case leaves an operation that can accept another frame.
    pub fn publish_header(
        &mut self,
        publish: impl FnOnce(&CodecConfig) -> Result<(), Cause>,
        stop: Option<&dyn zencodec::Pulse>,
    ) -> Result<(), Cause> {
        let local_stop = stop.unwrap_or(&zencodec::NoPulse);
        let state = self
            .state
            .take()
            .ok_or_else(|| std::io::Error::other("terminal frame route"))?;
        check(local_stop)?;
        if self.committed {
            return Err(std::io::Error::other("header already committed").into());
        }
        let config = state
            .native
            .as_ref()
            .and_then(|native| native.encoder.output_config())
            .ok_or_else(|| std::io::Error::other("no prepared output configuration"))?;
        self.committed = true;
        publish(config)?;
        check(local_stop)?;
        self.state = Some(state);
        Ok(())
    }
    pub fn push(&mut self, frame: &Frame, stop: Option<&dyn zencodec::Pulse>) -> Result<(), Cause> {
        self.push_route(
            frame,
            (frame.interpretation(), frame.timing()),
            None,
            || Ok(frame.clone()),
            stop,
        )
    }
    pub(super) fn push_route(
        &mut self,
        frame: &Frame,
        target: (&FrameInterpretation, FrameTiming),
        conversion: Option<FrameConversionOffer>,
        execute: impl FnOnce() -> Result<Frame, Cause>,
        stop: Option<&dyn zencodec::Pulse>,
    ) -> Result<(), Cause> {
        self.push_route_with_destination(
            frame,
            target,
            conversion,
            |_| Ok(((), None)),
            execute,
            stop,
        )
    }
    /// Retain the actual destination before policy, conversion or encoder input.
    /// This preparation must not publish bytes; publication is a later operation.
    pub(super) fn push_route_with_destination<D>(
        &mut self,
        frame: &Frame,
        target: (&FrameInterpretation, FrameTiming),
        conversion: Option<FrameConversionOffer>,
        prepare_destination: impl FnOnce(&CodecConfig) -> Result<(D, Option<DestinationFacts>), Cause>,
        execute: impl FnOnce() -> Result<Frame, Cause>,
        stop: Option<&dyn zencodec::Pulse>,
    ) -> Result<D, Cause> {
        let local_stop = stop.unwrap_or(&zencodec::NoPulse);
        let mut state = self
            .state
            .take()
            .ok_or_else(|| std::io::Error::other("terminal frame route"))?;
        check(local_stop)?;
        let (target, timing) = target;
        if conversion.is_some_and(|c| c.encoder_input != FrameFacts::new(target, timing)) {
            return Err(std::io::Error::other("conversion offer target mismatch").into());
        }
        let key = Key::from_input(target, timing)?;
        let changed = state.key != Some(key);
        let permitted = !changed || state.key.is_none() || self.segmented;
        // A real encoder/configuration is retained during policy. This may
        // allocate native workspace; no workspace bound is claimed by this route.
        let prepared = if changed && permitted {
            Some(Native::new(key, self.live.clone())?)
        } else {
            None
        };
        let destination = if permitted {
            let native = prepared
                .as_ref()
                .or(state.native.as_ref())
                .ok_or_else(|| std::io::Error::other("missing prepared encoder"))?;
            let config = native
                .encoder
                .output_config()
                .ok_or_else(|| std::io::Error::other("missing prepared output config"))?;
            Some(prepare_destination(config)?)
        } else {
            None
        };
        check(local_stop)?;
        let mut offer = FrameRouteOffer::new(if changed {
            FrameRoute::BeginSegment
        } else {
            FrameRoute::Continue
        });
        offer.fidelity = Some(Fidelity::Lossless);
        offer.conversion = conversion;
        offer.destination = destination.as_ref().and_then(|(_, facts)| *facts);
        offer.admitted_encoded_bytes = self.storage.reserved_bytes();
        let mut output = FrameRouteState::default();
        output.previous = state.previous;
        output.completed_segments = state.segments.len().try_into()?;
        output.output_committed = self.committed;
        let offers = if permitted {
            std::slice::from_ref(&offer)
        } else {
            &[]
        };
        let choice = resolve_frame_route(
            frame.interpretation(),
            frame.timing(),
            output,
            offers,
            &mut self.resolver,
            self.preset,
            stop,
        );
        match choice {
            Ok((0, record)) => self.report.record(record),
            Ok(_) => unreachable!("one retained prepared route"),
            Err(error) => {
                self.report.record(error.error().record());
                return Err(error.into());
            }
        }
        // The selected converter is consumed exactly once, after policy and
        // before any old encoder is drained or replacement installed.
        let converted = execute()?;
        check(local_stop)?;
        if converted.interpretation().as_ref() != target || converted.timing() != timing {
            return Err(std::io::Error::other("prepared conversion target changed").into());
        }
        if changed {
            finish_segment(&mut state, &self.storage, stop)?;
            state.native = prepared;
            state.key = Some(key);
        }
        let encoder = &mut state.native.as_mut().unwrap().encoder;
        loop {
            match encoder.send(&converted, stop)? {
                SendStatus::Accepted => break,
                SendStatus::ReceivePending => {
                    let Receive::Ready(packet) = encoder.receive(stop)? else {
                        return Err(
                            std::io::Error::other("native receive-pending promise broken").into(),
                        );
                    };
                    stage(&mut state.packets, packet, &self.storage)?;
                }
                _ => return Err(std::io::Error::other("unqualified send result").into()),
            }
        }
        loop {
            match encoder.receive(stop)? {
                Receive::Ready(packet) => stage(&mut state.packets, packet, &self.storage)?,
                Receive::NeedInput => break,
                _ => {
                    return Err(
                        std::io::Error::other("native encoder ended before end_input").into(),
                    );
                }
            }
        }
        state.inputs.try_reserve(1)?;
        state.inputs.push(converted.timing());
        state.previous = Some(FrameFacts::new(frame.interpretation(), frame.timing()));
        check(local_stop)?;
        self.state = Some(state);
        Ok(destination.expect("selected route retains destination").0)
    }
    pub fn finish(mut self, stop: Option<&dyn zencodec::Pulse>) -> Result<Vec<Segment>, Cause> {
        let local_stop = stop.unwrap_or(&zencodec::NoPulse);
        let mut state = self
            .state
            .take()
            .ok_or_else(|| std::io::Error::other("terminal frame route"))?;
        finish_segment(&mut state, &self.storage, stop)?;
        check(local_stop)?;
        // No live native encoder reaches the writer. It may wait while keeping
        // only admitted compressed payloads; caller-owned sources are separate.
        assert_eq!(self.live.load(Ordering::SeqCst), 0);
        Ok(state.segments)
    }
}

fn check(stop: &dyn zencodec::Pulse) -> Result<(), Cause> {
    stop.check()
        .map_err(|reason| At::wrap(CodecError::new(None, reason.into())).into())
}

fn stage(packets: &mut Vec<Packet>, packet: Packet, storage: &StorageBudget) -> Result<(), Cause> {
    if !packet.additions().is_empty() || packet.discard().is_some() {
        return Err(std::io::Error::other("unqualified encoded side-data preservation").into());
    }
    let mut bytes = storage.bytes(packet.len())?;
    bytes.as_mut_slice().copy_from_slice(packet.bytes());
    let mut out = Packet::from_accounted(bytes.freeze())?
        .with_track(packet.track())
        .with_config_epoch(packet.config_epoch())
        .with_random_access(packet.random_access());
    if let Some(value) = packet.pts() {
        out = out.with_pts(value);
    }
    if let Some(value) = packet.dts() {
        out = out.with_dts(value);
    }
    if let Some(value) = packet.duration_ticks() {
        out = out.with_duration_ticks(value);
    }
    if let Some(value) = packet.ordinal() {
        out = out.with_ordinal(value);
    }
    if let Some(value) = packet.input_offset() {
        out = out.with_input_offset(value);
    }
    packets.try_reserve(1)?;
    packets.push(out);
    Ok(())
}

fn finish_segment(
    state: &mut State,
    storage: &StorageBudget,
    stop: Option<&dyn zencodec::Pulse>,
) -> Result<(), Cause> {
    let local_stop = stop.unwrap_or(&zencodec::NoPulse);
    let Some(mut native) = state.native.take() else {
        return Ok(());
    };
    check(local_stop)?;
    let mut config = native.encoder.output_config().unwrap().clone();
    config.epoch = state.segments.len().try_into()?;
    native.encoder.end_input()?;
    loop {
        match native.encoder.receive(stop)? {
            Receive::Ready(packet) => stage(
                &mut state.packets,
                packet.with_config_epoch(config.epoch),
                storage,
            )?,
            Receive::EndOfStream => break,
            _ => return Err(std::io::Error::other("native encoder failed to drain").into()),
        }
    }
    drop(native);
    // Also stamp packets that became available before end_input.
    for packet in &mut state.packets {
        *packet = packet.clone().with_config_epoch(config.epoch);
    }
    state.segments.try_reserve(1)?;
    state.segments.push(Segment {
        config,
        settings: state.key.unwrap().settings(),
        packets: std::mem::take(&mut state.packets),
        inputs: std::mem::take(&mut state.inputs),
    });
    Ok(())
}

#[path = "epoch_route_tests.rs"]
mod tests;
