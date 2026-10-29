//! Apply the shared application policy before publishing native video frames.
//!
//! Each `receive` holds the actual decoded frame while the callback inspects
//! its immutable interpretation and presentation timing. Only successful policy
//! resolution can return it to a caller or downstream converter. Native decode
//! has already allocated/reconstructed the frame: this is not a pre-allocation
//! budget or a complete-container metadata privacy boundary.
use crate::{
    frame::Frame,
    packet::Packet,
    session::{CodecConfig, Receive, SendStatus, SessionError, VideoDecoder, VideoStreamInfo},
};
use alloc::boxed::Box;
use core::{error::Error as StdError, fmt};

use zencodec::operation::{OperationPolicy, OperationReport, PolicySet, resolve_frame};

/// Which part of the controlled session failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Stage {
    /// The native decoder failed.
    Decode,
    /// The shared policy refused, failed or was cancelled before publication.
    Policy,
}

/// Native or application error with the decisions preceding it. Default
/// formatting excludes native/application error text and metadata payloads.
pub struct Error {
    stage: Stage,
    report: Box<OperationReport>,
    cause: Box<dyn StdError + Send + Sync>,
}
impl Error {
    fn new(
        stage: Stage,
        report: &OperationReport,
        cause: impl StdError + Send + Sync + 'static,
    ) -> Self {
        Self {
            stage,
            report: Box::new(report.clone()),
            cause: Box::new(cause),
        }
    }
    /// The stage that failed.
    pub const fn stage(&self) -> Stage {
        self.stage
    }
    /// Bounded policy history, including the failed decision when available.
    pub fn report(&self) -> &OperationReport {
        &self.report
    }
}
impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VideoPolicyError")
            .field("stage", &self.stage)
            .field("report", &self.report)
            .finish_non_exhaustive()
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "controlled video decode failed during {:?}", self.stage)
    }
}
impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        Some(self.cause.as_ref())
    }
}

/// A native decoder with one shared application resolver before every output
/// presentation. No native frame is published on refusal, cancellation or an
/// invalid/stale choice. The original frame is returned unchanged on success.
///
/// The resolver runs for every presentation, even within the same epoch: policy
/// may inspect timing, and equal epochs do not prove equal interpretation.
/// Reports keep the most recent decisions in a fixed bound.
///
/// All failures and caught unwinds are terminal and drop the native decoder,
/// including its queued frames/references. `reset` is available only while the
/// operation remains healthy; after failure construct a fresh operation. Frames
/// previously returned remain valid through their own native owners.
pub struct PolicyDecoder<D, R> {
    decoder: Option<D>,
    resolver: R,
    preset: PolicySet,
    report: OperationReport,
}
impl<D, R> PolicyDecoder<D, R> {
    /// Wrap a native decoder and a built-in policy, application object or closure.
    /// Construction performs no decoding or policy decision.
    pub fn new(decoder: D, resolver: R) -> Self {
        Self {
            decoder: Some(decoder),
            resolver,
            preset: PolicySet::new(),
            report: OperationReport::default(),
        }
    }
    /// The built-in policy used when this resolver returns `UsePreset`.
    /// During migration, these requirements and the policy argument
    /// both apply. A bound policy supplies preferences in place of these.
    #[deprecated(note = "Pass PolicySet or BoundPolicy as the operation policy argument")]
    pub fn with_policy_set(mut self, preset: PolicySet) -> Self {
        self.preset = preset;
        self
    }
    /// Bounded decision history; also readable after terminal failure.
    pub fn report(&self) -> &OperationReport {
        &self.report
    }
    /// Whether this operation can no longer execute native work.
    pub const fn is_failed(&self) -> bool {
        self.decoder.is_none()
    }
}
impl<D: VideoDecoder, R: OperationPolicy> PolicyDecoder<D, R> {
    fn run<T>(
        &mut self,
        f: impl FnOnce(
            &mut D,
            &mut R,
            PolicySet,
            &mut OperationReport,
        ) -> Result<T, SessionError<Error>>,
    ) -> Result<T, SessionError<Error>> {
        // Taking ownership before any application/backend call makes unwinding
        // terminal too. The native backend drops with its queues on failure.
        let mut decoder = self.decoder.take().ok_or(SessionError::Finished)?;
        let output = f(
            &mut decoder,
            &mut self.resolver,
            self.preset,
            &mut self.report,
        )?;
        self.decoder = Some(decoder);
        Ok(output)
    }
}
fn native<E: StdError + Send + Sync + 'static>(
    error: SessionError<E>,
    report: &OperationReport,
) -> SessionError<Error> {
    error.map_codec(|error| Error::new(Stage::Decode, report, error))
}
impl<D: VideoDecoder, R: OperationPolicy> VideoDecoder for PolicyDecoder<D, R> {
    type Error = Error;
    fn configure(
        &mut self,
        config: &CodecConfig,
        pulse: Option<&dyn zencodec::Pulse>,
    ) -> Result<(), SessionError<Error>> {
        let local_pulse = pulse.unwrap_or(&zencodec::NoPulse);
        self.run(|decoder, _, _, report| {
            local_pulse.check()?;
            decoder
                .configure(config, pulse)
                .map_err(|e| native(e, report))
        })
    }
    fn send(
        &mut self,
        packet: &mut Packet,
        pulse: Option<&dyn zencodec::Pulse>,
    ) -> Result<SendStatus, SessionError<Error>> {
        let local_pulse = pulse.unwrap_or(&zencodec::NoPulse);
        self.run(|decoder, _, _, report| {
            local_pulse.check()?;
            decoder.send(packet, pulse).map_err(|e| native(e, report))
        })
    }
    fn receive(
        &mut self,
        pulse: Option<&dyn zencodec::Pulse>,
    ) -> Result<Receive<Frame>, SessionError<Error>> {
        let local_pulse = pulse.unwrap_or(&zencodec::NoPulse);
        self.run(|decoder, resolver, preset, report| {
            local_pulse.check()?;
            let output = decoder.receive(pulse).map_err(|e| native(e, report))?;
            if let Receive::Ready(frame) = &output {
                match resolve_frame(
                    frame.interpretation(),
                    frame.timing(),
                    resolver,
                    preset,
                    pulse,
                ) {
                    Ok(record) => report.record(record),
                    Err(error) => {
                        report.record(error.error().record());
                        return Err(SessionError::Codec(Error::new(
                            Stage::Policy,
                            report,
                            error,
                        )));
                    }
                }
            }
            Ok(output)
        })
    }
    fn end_input(&mut self) -> Result<(), SessionError<Error>> {
        self.run(|decoder, _, _, report| decoder.end_input().map_err(|e| native(e, report)))
    }
    fn reset(&mut self) -> Result<(), SessionError<Error>> {
        self.run(|decoder, _, _, report| decoder.reset().map_err(|e| native(e, report)))
    }
    fn stream_info(&self) -> Option<&VideoStreamInfo> {
        self.decoder.as_ref().and_then(VideoDecoder::stream_info)
    }
}
