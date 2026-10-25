//! Two qualified bitmap configurations and extracted EXIF packets only.
//! No whole-file privacy or whole-operation allocation claim is made here.
#![allow(dead_code)]
use crate::policy::*;
use core::{convert::Infallible, error::Error, fmt};
use std::{
    io::{self, Write},
    sync::Arc,
};
use zenbitmaps::{BmpEncoderConfig, QoiEncoderConfig};
use zencodec::{
    At, CodecSet, ImageFormat, Metadata, MetadataPolicy, ResourceLimits, Unstoppable,
    encode::{EncodeJob, EncoderConfig, Fidelity},
};
use zenpixels::PixelSlice;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stage {
    Policy,
    Metadata,
    Configuration,
    Codec,
    Output,
}
#[derive(Debug)]
pub(crate) struct Report {
    pub stage: Stage,
    /// Accepted by Write, not a durability/remote-receipt assertion.
    pub accepted_bytes: u64,
    pub output_complete: bool,
    pub decisions: Decisions,
}
/// Selected effects and boundary records survive both success and later failure.
/// This still-image slice makes at most three decisions; it retains all of them.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Decisions {
    pub metadata: Option<MetadataAction>,
    pub fidelity: Option<Fidelity>,
    pub execution: Option<Execution>,
    pub records: [Option<Record>; 3],
}
pub(crate) enum Cause<E> {
    Policy(Failure<E>),
    Metadata(zencodec::metadata_rewrite::Error),
    Other(Box<dyn Error + Send + Sync>),
}
pub(crate) struct RunError<E> {
    pub report: Box<Report>,
    pub cause: Cause<E>,
}
pub(crate) type RunResult<T, E> = Result<T, At<RunError<E>>>;
pub(crate) struct Request {
    pub format: ImageFormat,
    pub erased: bool,
    pub requirements: Requirements,
    pub preset: PolicySet,
    pub limits: ResourceLimits,
}
impl Request {
    pub fn new(format: ImageFormat, profile: Profile) -> Self {
        Self {
            format,
            erased: false,
            requirements: profile.requirements,
            preset: profile.preset,
            limits: ResourceLimits::none(),
        }
    }
    pub fn via_registry(mut self) -> Self {
        self.erased = true;
        self
    }
    pub fn encode(
        self,
        pixels: PixelSlice<'_>,
        metadata: &Metadata,
        writer: &mut impl Write,
    ) -> RunResult<Report, Infallible> {
        let mut preset = self.preset;
        encode(self, pixels, metadata, &mut preset, writer)
    }
    pub fn encode_with<R: PolicyResolver + ?Sized>(
        self,
        pixels: PixelSlice<'_>,
        metadata: &Metadata,
        resolver: &mut R,
        writer: &mut impl Write,
    ) -> RunResult<Report, R::Error> {
        encode(self, pixels, metadata, resolver, writer)
    }
}
impl<E> fmt::Debug for RunError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.report.fmt(f)
    }
}
impl<E> fmt::Display for RunError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "operation failed at {:?}, {} bytes accepted",
            self.report.stage, self.report.accepted_bytes
        )
    }
}
impl<E: Error + 'static> Error for RunError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(match &self.cause {
            Cause::Policy(e) => e,
            Cause::Metadata(e) => e,
            Cause::Other(e) => e.as_ref(),
        })
    }
}
fn failure<E>(stage: Stage, cause: Cause<E>, bytes: u64) -> RunError<E> {
    RunError {
        report: Box::new(Report {
            stage,
            accepted_bytes: bytes,
            output_complete: false,
            decisions: Decisions::default(),
        }),
        cause,
    }
}
fn policy_error<E>(e: At<Failure<E>>) -> At<RunError<E>> {
    e.map_error(|e| failure(Stage::Policy, Cause::Policy(e), 0))
        .at_named("checked native operation")
}

fn codec_error<E>(error: Box<dyn Error + Send + Sync>) -> At<RunError<E>> {
    // The pinned bitmap adapters expose this envelope. Generic Error::source
    // cannot recover an At wrapper's hidden inner error or its trace frames.
    match error.downcast::<At<zencodec::CodecError>>() {
        Ok(error) => error
            .map_error(|e| failure(Stage::Codec, Cause::Other(Box::new(e)), 0))
            .at_named("erased native encoder"),
        Err(error) => At::wrap(failure(Stage::Codec, Cause::Other(error), 0)).at(),
    }
}

fn offer(action: Action, effects: Effects) -> Alternative {
    Alternative {
        action,
        ram: Evidence::Unknown,
        disk: Evidence::Upper(0),
        accounting: Accounting::Operation,
        effects,
        needs_replay: false,
        preserves_published: false,
    }
}

/// Retention covers one extracted EXIF packet. NO_GPS means the standard GPS
/// IFD was removed; arbitrary opaque/private data is not certified anonymous.
pub(crate) fn exif<R: PolicyResolver + ?Sized>(
    engine: &mut Engine,
    bytes: Option<&[u8]>,
    carrier: bool,
    requirements: Requirements,
    resolver: &mut R,
) -> RunResult<Option<Arc<[u8]>>, R::Error> {
    select_exif(engine, bytes, carrier, requirements, resolver).map(|resolved| resolved.packet)
}

struct ResolvedExif {
    packet: Option<Arc<[u8]>>,
    selection: Selected,
}

fn select_exif<R: PolicyResolver + ?Sized>(
    engine: &mut Engine,
    bytes: Option<&[u8]>,
    carrier: bool,
    requirements: Requirements,
    resolver: &mut R,
) -> RunResult<ResolvedExif, R::Error> {
    // Hard operation memory requirements cannot use this unqualified native
    // processing path. Refuse before parsing or constructing output packets.
    if requirements.ram.is_some() {
        return engine
            .resolve(
                Facts::Metadata(MetadataFacts {
                    packet_bytes: bytes.map_or(0, |b| b.len() as u64),
                    parse_complete: false,
                    has_location: None,
                    has_rights: None,
                }),
                &[],
                requirements,
                Context::default(),
                resolver,
                &Unstoppable,
            )
            .map(|selection| ResolvedExif {
                packet: None,
                selection,
            })
            .map_err(policy_error);
    }
    let request = zencodec::metadata_rewrite::Request::new(b"")
        .with_policy(MetadataPolicy::Web)
        .with_metadata_limit(1024 * 1024);
    let processor = zenmeta::rewrite::Context::new(&request).unwrap();
    let filtered = bytes.map(|b| processor.exif(b));
    let validated = matches!(filtered, None | Some(Ok(_)));
    let parsed = if validated {
        bytes.and_then(zencodec::Exif::parse)
    } else {
        None
    };
    let has_rights = if bytes.is_none() {
        Some(false)
    } else {
        parsed
            .as_ref()
            .map(|e| e.copyright().is_some() || e.artist().is_some())
    };
    let has_location = if bytes.is_none() {
        Some(false)
    } else {
        parsed.as_ref().map(|e| e.has_gps())
    };
    let mut offers = Vec::new();
    let mut packets = Vec::new();
    if carrier || bytes.is_none() {
        let mut effects = Effects::RIGHTS.union(Effects::EXACT_METADATA);
        if has_location == Some(false) {
            effects = effects.union(Effects::NO_GPS);
        }
        offers.push(offer(Action::Metadata(MetadataAction::KeepExact), effects));
        packets.push(bytes.map(Arc::from));
    }
    if carrier && let Some(Ok(filtered)) = &filtered {
        offers.push(offer(
            Action::Metadata(MetadataAction::FilterPublic),
            Effects::RIGHTS.union(Effects::NO_GPS),
        ));
        packets.push(Some(Arc::from(filtered.as_slice())));
    }
    let mut effects = Effects::NO_GPS;
    if has_rights == Some(false) {
        effects = effects.union(Effects::RIGHTS);
    }
    if bytes.is_none() {
        effects = effects.union(Effects::EXACT_METADATA);
    }
    offers.push(offer(
        Action::Metadata(MetadataAction::DropDescriptive),
        effects,
    ));
    packets.push(None);
    let selected = engine
        .resolve(
            Facts::Metadata(MetadataFacts {
                packet_bytes: bytes.map_or(0, |b| b.len() as u64),
                parse_complete: validated,
                has_location,
                has_rights,
            }),
            &offers,
            requirements,
            Context::default(),
            resolver,
            &Unstoppable,
        )
        .map_err(policy_error)?;
    Ok(ResolvedExif {
        packet: packets.swap_remove(selected.index),
        selection: selected,
    })
}

pub(crate) struct CountWriter<'a, W> {
    pub writer: &'a mut W,
    pub accepted: u64,
}
impl<W: Write> Write for CountWriter<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let n = self.writer.write(bytes)?;
        if n > bytes.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "writer overreported accepted bytes",
            ));
        }
        self.accepted = self
            .accepted
            .checked_add(n as u64)
            .ok_or_else(|| io::Error::other("output count overflow"))?;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

/// Same callback and request for concrete and CodecSet paths. Both fixtures have
/// no EXIF carrier; only an explicitly admissible metadata omission may proceed.
pub(crate) fn encode<R: PolicyResolver + ?Sized>(
    request: Request,
    pixels: PixelSlice<'_>,
    metadata: &Metadata,
    resolver: &mut R,
    writer: &mut impl Write,
) -> RunResult<Report, R::Error> {
    let Request {
        format,
        erased,
        requirements,
        preset,
        limits,
    } = request;
    let mut engine = Engine::new(preset, 3);
    let mut decisions = Decisions::default();
    let result = (|| {
        if requirements.effects.0 & !31 != 0 {
            return Err(At::wrap(failure(
                Stage::Configuration,
                Cause::Metadata(zencodec::metadata_rewrite::Error::Unsupported(
                    "requirement outside still-image operation",
                )),
                0,
            ))
            .at());
        }
        // Scope is explicit: these controls accept EXIF only. No color/orientation
        // conversion, XMP editor or HDR/gain-map handling is silently supplied.
        if metadata.xmp.is_some()
            || metadata.icc_profile.is_some()
            || metadata.cicp.is_some()
            || metadata.content_light_level.is_some()
            || metadata.mastering_display.is_some()
            || metadata.diffuse_white.is_some()
            || metadata.orientation != zencodec::Orientation::Identity
        {
            return Err(At::wrap(failure(
                Stage::Metadata,
                Cause::Metadata(zencodec::metadata_rewrite::Error::Unsupported(
                    "unqualified metadata or orientation request",
                )),
                0,
            ))
            .at());
        }
        let resolved = select_exif(
            &mut engine,
            metadata.exif.as_deref(),
            false,
            requirements,
            resolver,
        )?;
        let Action::Metadata(action) = resolved.selection.alternative.action else {
            unreachable!("engine enforces question family")
        };
        decisions.metadata = Some(action);
        assert!(resolved.packet.is_none());
        let fidelity = [offer(
            Action::Fidelity(Fidelity::Lossless),
            Effects::LOSSLESS,
        )];
        let selected = engine
            .resolve(
                Facts::Fidelity(FidelityFacts {
                    alpha: pixels.descriptor().has_alpha(),
                    bits_per_sample: (pixels.descriptor().channel_type().byte_size() * 8) as u8,
                }),
                &fidelity,
                requirements,
                Context::default(),
                resolver,
                &Unstoppable,
            )
            .map_err(policy_error)?;
        let Action::Fidelity(action) = selected.alternative.action else {
            unreachable!("engine enforces question family")
        };
        decisions.fidelity = Some(action);
        let execution = [offer(
            Action::Execution(Execution::Buffered),
            Effects::default(),
        )];
        let selected = engine
            .resolve(
                Facts::Execution(ExecutionFacts {
                    resident_input: pixels.as_strided_bytes().len() as u64,
                    source_length: None,
                    network: false,
                }),
                &execution,
                requirements,
                Context::default(),
                resolver,
                &Unstoppable,
            )
            .map_err(policy_error)?;
        let Action::Execution(action) = selected.alternative.action else {
            unreachable!("engine enforces question family")
        };
        decisions.execution = Some(action);
        let output = if erased {
            let set = CodecSet::new()
                .with_encoder(QoiEncoderConfig::new())
                .with_encoder(BmpEncoderConfig::new());
            let config = set.encoder_for(format).ok_or_else(|| {
                At::wrap(failure(
                    Stage::Configuration,
                    Cause::Other(Box::new(zencodec::UnsupportedOperation::EncodePolicy)),
                    0,
                ))
                .at()
            })?;
            // Use the existing checked target/job constructor; encode_job_with is
            // currently best-effort and cannot substitute for this validation.
            let mut job = config.try_fidelity_job(Fidelity::Lossless).map_err(|e| {
                At::wrap(failure(Stage::Configuration, Cause::Other(Box::new(e)), 0)).at()
            })?;
            job.set_limits(limits);
            job.encode(pixels).map_err(codec_error)?
        } else {
            let output = match format {
                ImageFormat::Qoi => QoiEncoderConfig::new()
                    .try_with_fidelity(Fidelity::Lossless)
                    .unwrap()
                    .job()
                    .with_limits(limits)
                    .encode(pixels),
                ImageFormat::Bmp => BmpEncoderConfig::new()
                    .try_with_fidelity(Fidelity::Lossless)
                    .unwrap()
                    .job()
                    .with_limits(limits)
                    .encode(pixels),
                _ => {
                    return Err(At::wrap(failure(
                        Stage::Configuration,
                        Cause::Other(Box::new(zencodec::UnsupportedOperation::EncodePolicy)),
                        0,
                    ))
                    .at());
                }
            };
            output.map_err(|e| {
                e.map_error(|e| failure(Stage::Codec, Cause::Other(Box::new(e)), 0))
                    .at_named("native encoder")
            })?
        };
        let mut counted = CountWriter {
            writer,
            accepted: 0,
        };
        counted.write_all(output.data()).map_err(|e| {
            At::wrap(failure(
                Stage::Output,
                Cause::Other(Box::new(e)),
                counted.accepted,
            ))
            .at()
        })?;
        // Completion means the encoded bytes were accepted by Write. Flushing,
        // durable publication and remote acknowledgement are adapter contracts.
        Ok(Report {
            stage: Stage::Output,
            accepted_bytes: counted.accepted,
            output_complete: true,
            decisions,
        })
    })();
    for (slot, record) in decisions.records.iter_mut().zip(engine.records()) {
        *slot = Some(record);
    }
    result
        .map(|mut report| {
            report.decisions = decisions;
            report
        })
        .map_err(|error: At<RunError<R::Error>>| {
            error.map_error(|mut error| {
                error.report.decisions = decisions;
                error
            })
        })
}
