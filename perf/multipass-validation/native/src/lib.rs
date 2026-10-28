#![forbid(unsafe_code)]
//! Shared `Encoder::pull_rows` over imij's public sans-IO bounded lossy session.
//!
//! The session owns its band buffer; requests are filled in place. Output
//! writes at the current end become pending bytes; earlier offsets become
//! retained patches (seek-back only). Index and checksum scratch are encoder
//! retention, kept here under an explicit cap, never in the driver.

use imij::stream_encode::{
    BoundedLossySession, BoundedLossyStep, BoundedStream, LossyUnitIndex, StreamEncodeError,
};
use imij::{Channels, ImageInfo, ImiEncoderConfig, Metadata, SampleType};
use std::{error::Error, fmt, fs::File, io, os::unix::fs::FileExt, sync::Arc};
use zencodec::encode::{
    EncodeDeclaration, EncodePlan, EncodeRows as PassRows, Encoder, OutputPhase,
    PullProgress as PassProgress, RowDemand as RowRequest, SourcePasses,
};
use zencodec::{
    ImageFormat,
    encode::{EncodeOutput, OutputTiming},
    output_patch::{OutputAccess, OutputFinish, Patch},
};
use zencodec_io::row_encode::Description;
use zenpixels::{
    AlphaMode, ChannelLayout, ChannelType, ColorPrimaries, PixelSliceMut, SignalRange,
    TransferFunction,
};

/// Adapter error. Native errors keep their original value.
#[derive(Debug)]
#[non_exhaustive]
pub enum ImiError {
    /// Native encoder error.
    Native(StreamEncodeError),
    /// A shared protocol this native adapter does not implement.
    Rejected(zencodec::UnsupportedOperation),
    /// Declaration this prototype does not map without relabeling.
    Unsupported(&'static str),
    /// Encoder state machine misuse.
    State(&'static str),
    /// The driver interrupted a request; its cause is primary.
    Interrupted,
    /// Cancelled between native steps.
    Cancelled,
    /// A rewrite of earlier output with forward-only access.
    PatchRequired,
    /// Scratch cap or I/O.
    Scratch(io::Error),
}
impl fmt::Display for ImiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Native(e) => write!(f, "IMI native encoder: {e}"),
            Self::Rejected(e) => write!(f, "{e}"),
            Self::Unsupported(s) => write!(f, "IMI adapter unsupported: {s}"),
            Self::State(s) => write!(f, "IMI adapter state: {s}"),
            Self::Interrupted => f.write_str("IMI request interrupted by driver"),
            Self::Cancelled => f.write_str("IMI encode cancelled"),
            Self::PatchRequired => f.write_str("IMI output rewrite needs seek-back access"),
            Self::Scratch(e) => write!(f, "IMI scratch: {e}"),
        }
    }
}
impl Error for ImiError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Native(e) => Some(e),
            Self::Rejected(e) => Some(e),
            Self::Scratch(e) => Some(e),
            _ => None,
        }
    }
}

/// Encoder-owned scratch for the unit index and checksum digests.
pub enum Scratch {
    /// Heap, refusing growth past this many bytes in total.
    Memory {
        /// Cap across index and checksum scratch.
        cap: u64,
    },
    /// Caller-owned files (index, optional checksum), positional writes from 0.
    Files(File, Option<File>),
}

enum Store {
    Memory(Vec<u8>),
    File(File),
}
impl Store {
    fn write_at(&mut self, at: u64, bytes: &[u8], used: &mut u64, cap: u64) -> io::Result<()> {
        match self {
            Self::Memory(v) => {
                let end = usize::try_from(at)
                    .ok()
                    .and_then(|a| a.checked_add(bytes.len()))
                    .ok_or(io::ErrorKind::OutOfMemory)?;
                if end > v.len() {
                    let grow = (end - v.len()) as u64;
                    if used.saturating_add(grow) > cap {
                        return Err(io::Error::new(io::ErrorKind::OutOfMemory, "scratch cap"));
                    }
                    v.try_reserve_exact(end - v.len())
                        .map_err(|_| io::ErrorKind::OutOfMemory)?;
                    v.resize(end, 0);
                    *used += grow;
                }
                v[at as usize..end].copy_from_slice(bytes);
                Ok(())
            }
            Self::File(f) => f.write_all_at(bytes, at),
        }
    }
    fn read_at(&mut self, at: u64, buf: &mut [u8]) -> io::Result<()> {
        match self {
            Self::Memory(v) => {
                let s = usize::try_from(at)
                    .ok()
                    .and_then(|a| v.get(a..a.checked_add(buf.len())?))
                    .ok_or(io::ErrorKind::UnexpectedEof)?;
                buf.copy_from_slice(s);
                Ok(())
            }
            Self::File(f) => f.read_exact_at(buf, at),
        }
    }
}

/// Counters for retained-memory evidence (bytes the adapter itself owns).
#[derive(Clone, Copy, Debug, Default)]
pub struct ImiStats {
    /// Heap scratch high-water (index + checksum).
    pub scratch_bytes: u64,
    /// Largest pending output held between drains.
    pub max_pending: usize,
    /// Patch bytes retained for finish.
    pub patch_bytes: u64,
    /// Distinct source passes the native session requested.
    pub passes: u32,
    /// Native file length reported at completion.
    pub native_len: u64,
}

struct Active {
    session: BoundedLossySession,
    desc: Description,
    row_bytes: usize,
    access: OutputAccess,
    /// Output bytes appended so far (consumed + pending).
    produced: u64,
    pending: Vec<u8>,
    taken: usize,
    patches: Vec<Patch>,
    index: Store,
    checksum: Option<Store>,
    last_label: Option<u8>,
    done: bool,
    /// Every byte stays pending-invisible until finish (single-band images,
    /// whose header precedes their only band's records).
    withhold: bool,
}

/// IMI's coding pass: always the last, and the only one that writes output.
const CODING_PASS: u8 = 2;

/// IMI bounded lossy configuration with the user's exact quality/effort/tier.
pub struct ImiBounded {
    config: ImiEncoderConfig,
    metadata: Metadata,
    scratch: Option<Scratch>,
    scratch_cap: u64,
    max_index_bytes: u64,
    active: Option<Active>,
    stats: ImiStats,
}

impl ImiBounded {
    /// `max_index_bytes` is the native index scratch budget.
    pub fn new(
        config: ImiEncoderConfig,
        metadata: Metadata,
        scratch: Scratch,
        max_index_bytes: u64,
    ) -> Self {
        let scratch_cap = match &scratch {
            Scratch::Memory { cap } => *cap,
            Scratch::Files(..) => u64::MAX,
        };
        Self {
            config,
            metadata,
            scratch: Some(scratch),
            scratch_cap,
            max_index_bytes,
            active: None,
            stats: ImiStats::default(),
        }
    }
    /// Evidence counters so far.
    pub fn stats(&self) -> ImiStats {
        self.stats
    }
}

/// Native description for a declaration, refusing anything this prototype
/// cannot carry without relabeling (only sRGB-signaled 8/16-bit samples).
pub fn image_info(desc: &Description) -> Result<ImageInfo, ImiError> {
    let d = desc.descriptor;
    let channels = match d.layout() {
        ChannelLayout::Gray => Channels::Gray,
        ChannelLayout::GrayAlpha => Channels::GrayAlpha,
        ChannelLayout::Rgb => Channels::Rgb,
        ChannelLayout::Rgba => Channels::Rgba,
        _ => return Err(ImiError::Unsupported("channel layout")),
    };
    let sample = match d.channel_type() {
        ChannelType::U8 => SampleType::U8,
        ChannelType::U16 => SampleType::U16,
        _ => return Err(ImiError::Unsupported("sample type")),
    };
    if d.transfer() != TransferFunction::Srgb {
        return Err(ImiError::Unsupported("transfer other than sRGB"));
    }
    if d.primaries != ColorPrimaries::Bt709 {
        return Err(ImiError::Unsupported("primaries other than sRGB"));
    }
    if d.signal_range != SignalRange::Full {
        return Err(ImiError::Unsupported("signal range other than full"));
    }
    match (channels, d.alpha()) {
        (Channels::GrayAlpha | Channels::Rgba, Some(AlphaMode::Straight)) => {}
        (Channels::Gray | Channels::Rgb, None) => {}
        _ => return Err(ImiError::Unsupported("alpha mode")),
    }
    if let Some(ctx) = &desc.color_context {
        let srgb = ctx.icc.is_none()
            && ctx.cicp.is_some_and(|c| {
                c.color_primaries == 1
                    && c.transfer_characteristics == 13
                    && c.matrix_coefficients == 0
                    && c.full_range
            });
        if !srgb {
            return Err(ImiError::Unsupported("color context other than sRGB CICP"));
        }
    }
    let info = ImageInfo::new(desc.width, desc.height, channels, sample);
    if info.cicp != imij::Cicp::SRGB {
        return Err(ImiError::Unsupported("native default color is not sRGB"));
    }
    Ok(info)
}

impl Encoder for ImiBounded {
    type Error = ImiError;

    fn reject(op: zencodec::UnsupportedOperation) -> ImiError {
        ImiError::Rejected(op)
    }

    fn encode(self, _: zenpixels::PixelSlice<'_>) -> Result<EncodeOutput, ImiError> {
        Err(ImiError::Unsupported("use the declared pull-row protocol"))
    }

    fn begin_declared(
        &mut self,
        declaration: &EncodeDeclaration<'_>,
    ) -> Result<EncodePlan, ImiError> {
        let access = declaration.access;
        let mut desc = Description::new(
            declaration.width,
            declaration.height,
            declaration.descriptor,
        );
        desc.color_context = declaration.color.map(|color| Arc::new(color.clone()));
        desc.exact_depth = declaration.exact_depth;
        if self.active.is_some() {
            return Err(ImiError::State("begin_declared called twice"));
        }
        let info = image_info(&desc)?;
        let row_bytes = info.row_bytes();
        // Seek-back output keeps the unit table at the head; forward output
        // gets a trailing table. Same pixels either way.
        let index = match access {
            OutputAccess::SeekBack => LossyUnitIndex::Reserved(4),
            _ => LossyUnitIndex::Trailing,
        };
        let session = BoundedLossySession::new(
            info,
            &self.metadata,
            &self.config,
            index,
            self.max_index_bytes,
        )
        .map_err(ImiError::Native)?;
        // From the native plan, before any source row: the pass range and the
        // band each request fills.
        let planned = session.passes();
        if planned.min == 0 || planned.max < planned.min || planned.band_rows == 0 {
            return Err(ImiError::State("native pass plan"));
        }
        // Validated: take scratch only now so a failed begin leaves self unchanged.
        let (index_store, checksum) = match self.scratch.take().expect("unchanged on failure") {
            Scratch::Memory { .. } => (Store::Memory(Vec::new()), Some(Store::Memory(Vec::new()))),
            Scratch::Files(i, c) => (Store::File(i), c.map(Store::File)),
        };
        self.active = Some(Active {
            session,
            desc: desc.clone(),
            row_bytes,
            access,
            produced: 0,
            pending: Vec::new(),
            taken: 0,
            patches: Vec::new(),
            index: index_store,
            checksum,
            last_label: None,
            done: false,
            withhold: planned.band_rows >= desc.height,
        });
        // Output begins in the coding pass, the last. A pull returns right
        // after filling a band and the session codes that band on the next
        // pull, so the coding pass holds one band of rows. A single band can
        // be held back entirely, which streaming cannot state.
        let mut timing = if planned.band_rows < desc.height {
            OutputTiming::streaming(planned.band_rows)
        } else {
            OutputTiming::AT_FINISH
        };
        if access == OutputAccess::SeekBack {
            timing = timing.with_patches();
        }
        Ok(EncodePlan::replay(
            SourcePasses::new(planned.min, Some(planned.max)).expect("validated native range"),
            timing,
            OutputPhase::FinalPass,
        ))
    }

    fn pull_rows(
        &mut self,
        rows: &mut dyn PassRows,
        stop: &dyn zencodec::Pulse,
    ) -> Result<PassProgress, ImiError> {
        let cap = self.scratch_cap;
        let stats = &mut self.stats;
        let a = self
            .active
            .as_mut()
            .ok_or(ImiError::State("pull_rows before begin_declared"))?;
        if a.done {
            return Ok(PassProgress::Done);
        }
        let Active {
            session,
            desc,
            row_bytes,
            access,
            produced,
            pending,
            taken,
            patches,
            index,
            checksum,
            last_label,
            done,
            ..
        } = a;
        loop {
            if stop.check().is_err() {
                session.fail(StreamEncodeError::Codec(imij::Error::Cancelled));
                return Err(ImiError::Cancelled);
            }
            match session.step().map_err(ImiError::Native)? {
                BoundedLossyStep::Rows {
                    pass,
                    y0,
                    rows: buf,
                } => {
                    if *last_label != Some(pass) {
                        *last_label = Some(pass);
                        stats.passes += 1;
                    }
                    let n = (buf.len() / *row_bytes) as u32;
                    let mut view =
                        PixelSliceMut::new(buf, desc.width, n, *row_bytes, desc.descriptor)
                            .map_err(|_| ImiError::State("native band geometry"))?;
                    if let Some(ctx) = &desc.color_context {
                        view = view.with_color_context(Arc::clone(ctx));
                    }
                    let request = RowRequest::new(stats.passes - 1, y0, view)
                        .with_final_pass(pass == CODING_PASS)
                        .with_label(pass);
                    if rows.fill(request).is_err() {
                        session.fail(StreamEncodeError::Codec(imij::Error::Read));
                        return Err(ImiError::Interrupted);
                    }
                    return Ok(PassProgress::Filled);
                }
                BoundedLossyStep::Write {
                    stream: BoundedStream::Output,
                    offset,
                    bytes,
                } => {
                    let len = bytes.len() as u64;
                    let end = offset.checked_add(len).ok_or(ImiError::State("offset"))?;
                    if offset > *produced {
                        return Err(ImiError::State("gap in native output"));
                    }
                    let overlap = end.min(*produced) - offset;
                    if overlap > 0 {
                        // Rewrite in place while still pending, else a patch.
                        let pending_start = *produced - (pending.len() - *taken) as u64;
                        if offset >= pending_start {
                            let at = *taken + (offset - pending_start) as usize;
                            pending[at..at + overlap as usize]
                                .copy_from_slice(&bytes[..overlap as usize]);
                        } else if *access == OutputAccess::SeekBack {
                            patches.push(Patch {
                                offset,
                                bytes: bytes[..overlap as usize].to_vec(),
                            });
                            stats.patch_bytes += overlap;
                        } else {
                            return Err(ImiError::PatchRequired);
                        }
                    }
                    let rest = &bytes[overlap as usize..];
                    pending
                        .try_reserve(rest.len())
                        .map_err(|_| ImiError::Scratch(io::ErrorKind::OutOfMemory.into()))?;
                    pending.extend_from_slice(rest);
                    *produced += rest.len() as u64;
                    stats.max_pending = stats.max_pending.max(pending.len() - *taken);
                }
                BoundedLossyStep::Write {
                    stream,
                    offset,
                    bytes,
                } => {
                    let store = match stream {
                        BoundedStream::IndexScratch => &mut *index,
                        _ => checksum
                            .as_mut()
                            .ok_or(ImiError::State("no checksum scratch"))?,
                    };
                    store
                        .write_at(offset, bytes, &mut stats.scratch_bytes, cap)
                        .map_err(|e| {
                            session.fail(StreamEncodeError::Write(io::Error::from(e.kind())));
                            ImiError::Scratch(e)
                        })?;
                }
                BoundedLossyStep::Read {
                    stream,
                    offset,
                    buf,
                } => {
                    let store = match stream {
                        BoundedStream::IndexScratch => &mut *index,
                        BoundedStream::ChecksumScratch => checksum
                            .as_mut()
                            .ok_or(ImiError::State("no checksum scratch"))?,
                        _ => return Err(ImiError::State("native read of output")),
                    };
                    store.read_at(offset, buf).map_err(ImiError::Scratch)?;
                }
                BoundedLossyStep::Done { len } => {
                    stats.native_len = len;
                    if len != *produced {
                        return Err(ImiError::State("native length mismatch"));
                    }
                    *done = true;
                    return Ok(PassProgress::Done);
                }
                _ => return Err(ImiError::State("unknown native step")),
            }
        }
    }

    fn pending_output(&self) -> &[u8] {
        match &self.active {
            Some(a) if !a.withhold => &a.pending[a.taken..],
            _ => &[],
        }
    }

    fn consume_output(&mut self, n: usize) {
        if let Some(a) = &mut self.active {
            a.taken = (a.taken + n).min(a.pending.len());
            if a.taken == a.pending.len() {
                a.pending.clear();
                a.taken = 0;
            }
        }
    }

    fn finish_output(self) -> Result<OutputFinish<EncodeOutput>, ImiError> {
        let mut a = self.active.ok_or(ImiError::State("finish before begin"))?;
        if !a.done {
            return Err(ImiError::State("finish before every pass completed"));
        }
        let tail = a.pending.split_off(a.taken);
        Ok(OutputFinish::new(
            EncodeOutput::new(tail, ImageFormat::Unknown),
            a.patches,
        ))
    }
}
