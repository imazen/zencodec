//! Fixture infrastructure only: native jobs and generated shared erasure do all
//! planning/startup/encoding. No experimental encoder or replay driver is used.
use imij::{
    DecodeSpeed, ImiEncoderConfig,
    codec::{EncodeRequest, Scratch},
};
use std::{
    fs::{File, OpenOptions},
    io::{self, Cursor, Seek, Write},
    path::PathBuf,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};
use zencodec::{
    At, Cicp, ImageFormat, Metadata, MetadataPolicy, ResourceLimits,
    encode::{DynEncoderConfig, EncodeJob, Encoder, EncoderConfig},
    operation::OperationPolicy,
    storage::StorageBudget,
};
use zencodec_io::{
    destination::SeekBack,
    row_encode::{self, Description, Options, Source},
};
use zenpixels::{
    ChannelType, ColorContext, PixelDescriptor, PixelSlice, PixelSliceMut, SignalRange,
    sample::ExactDepth,
};

pub const W: u32 = 129;
pub const H: u32 = 97;
pub const BUDGET: usize = 64 << 20;

pub struct Fixture {
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    pub descriptor: PixelDescriptor,
    pub color: Arc<ColorContext>,
    pub exact: ExactDepth,
}
impl Fixture {
    pub fn new(descriptor: PixelDescriptor) -> Self {
        let bpp = descriptor.bytes_per_pixel();
        let stride = (W as usize + 2) * bpp; // Pixel-aligned padding, including U16.
        let mut bytes = vec![0x5a; stride * H as usize];
        // Same edge/gradient fixture family as native tests/codec_bounded.rs.
        // U16 expands the U8 grid exactly; padding deliberately violates it.
        for y in 0..H as usize {
            for x in 0..W as usize {
                for c in 0..3 {
                    let value = (x * (c + 1) + y * 2 + ((x / 13 + y / 7) % 3) * 40) as u8;
                    if descriptor.channel_type() == ChannelType::U16 {
                        let at = y * stride + x * bpp + c * 2;
                        bytes[at..at + 2].copy_from_slice(&(u16::from(value) * 257).to_ne_bytes());
                    } else {
                        bytes[y * stride + x * bpp + c] = value;
                    }
                }
            }
        }
        Self {
            bytes,
            width: W,
            height: H,
            stride,
            descriptor,
            color: Arc::new(ColorContext::from_cicp(Cicp::SRGB)),
            exact: ExactDepth::new(
                descriptor.channel_type(),
                SignalRange::Full,
                8,
                if descriptor.channel_type() == ChannelType::U16 {
                    16
                } else {
                    8
                },
            )
            .unwrap(),
        }
    }
    pub fn pixels(&self) -> PixelSlice<'_> {
        PixelSlice::new(
            &self.bytes,
            self.width,
            self.height,
            self.stride,
            self.descriptor,
        )
        .unwrap()
        .with_exact_depth(self.exact)
        .unwrap()
        .with_color_context(self.color.clone())
    }
    pub fn description(&self) -> Description {
        Description::new(self.width, self.height, self.descriptor)
            .with_color_context(self.color.clone())
            .with_exact_depth(self.exact)
    }
}

pub fn config(checksum: bool) -> ImiEncoderConfig {
    ImiEncoderConfig::lossy(70.0)
        .with_decode_speed(DecodeSpeed::Fast)
        .with_checksum(checksum)
}
pub fn metadata() -> Metadata {
    let mut value = Metadata::none();
    value.xmp = Some(Arc::from(&b"<x:xmpmeta xmlns:x='adobe:ns:meta/'/>"[..]));
    value
}
pub fn job(
    config: ImiEncoderConfig,
    scratch: Scratch,
    metadata: Metadata,
    limits: ResourceLimits,
) -> EncodeRequest {
    config
        .job()
        .with_scratch(scratch)
        .with_metadata_policy(metadata, MetadataPolicy::PreserveExact)
        .with_limits(limits)
}

// Public generic erasure starts from EncoderConfig, while native scratch is a
// job setting. This private test configuration binds that setting *before* the
// standard DynEncoderConfig blanket creates the real native EncodeRequest.
// It does not implement DynEncodeJob/Encoder, substitute a plan, or forward pulls.
#[derive(Clone)]
struct NativeConfig {
    config: ImiEncoderConfig,
    scratch: Scratch,
    metadata: Metadata,
    limits: ResourceLimits,
}
impl EncoderConfig for NativeConfig {
    type Error = imij::Error;
    type Job = EncodeRequest;
    fn format() -> ImageFormat {
        <ImiEncoderConfig as EncoderConfig>::format()
    }
    fn supported_descriptors() -> &'static [PixelDescriptor] {
        <ImiEncoderConfig as EncoderConfig>::supported_descriptors()
    }
    fn job(self) -> Self::Job {
        job(self.config, self.scratch, self.metadata, self.limits)
    }
}
fn erased(
    config: ImiEncoderConfig,
    scratch: Scratch,
    metadata: Metadata,
    limits: ResourceLimits,
) -> Box<dyn zencodec::encode::DynEncodeJob> {
    // Drop the temporary configuration before the driver runs: the one retained
    // native job is then the only owner of this shared metadata payload.
    NativeConfig {
        config,
        scratch,
        metadata,
        limits,
    }
    .dyn_job()
}

#[derive(Clone, Copy, Debug)]
pub enum Dispatch {
    Typed,
    Erased,
}
#[derive(Clone, Copy, Debug)]
pub enum Access {
    Forward,
    SeekBack,
}
pub const DISPATCHES: [Dispatch; 2] = [Dispatch::Typed, Dispatch::Erased];
pub const ACCESSES: [Access; 2] = [Access::Forward, Access::SeekBack];

pub struct Run {
    pub config: ImiEncoderConfig,
    pub scratch: Scratch,
    pub metadata: Metadata,
    pub limits: ResourceLimits,
    pub options: Options,
    pub writer: Writer,
}
impl Run {
    pub fn execute<S: Source, R: OperationPolicy>(
        self,
        dispatch: Dispatch,
        access: Access,
        source: S,
        policy: R,
        pulse: &dyn zencodec::Pulse,
    ) -> Result<row_encode::Completed, At<row_encode::Failure>> {
        match (dispatch, access) {
            (Dispatch::Typed, Access::Forward) => row_encode::encode_job(
                job(self.config, self.scratch, self.metadata, self.limits),
                source,
                self.writer,
                policy,
                self.options,
                pulse,
            ),
            (Dispatch::Typed, Access::SeekBack) => row_encode::encode_job_to_destination(
                job(self.config, self.scratch, self.metadata, self.limits),
                source,
                SeekBack::new(self.writer).unwrap(),
                policy,
                self.options,
                pulse,
            ),
            (Dispatch::Erased, Access::Forward) => row_encode::encode_dyn_job(
                erased(self.config, self.scratch, self.metadata, self.limits),
                source,
                self.writer,
                policy,
                self.options,
                pulse,
            ),
            (Dispatch::Erased, Access::SeekBack) => row_encode::encode_dyn_job_to_destination(
                erased(self.config, self.scratch, self.metadata, self.limits),
                source,
                SeekBack::new(self.writer).unwrap(),
                policy,
                self.options,
                pulse,
            ),
        }
    }
}

pub fn ordinary(f: &Fixture, checksum: bool) -> Vec<u8> {
    job(
        config(checksum),
        Scratch::Memory,
        metadata(),
        ResourceLimits::none(),
    )
    .encoder()
    .unwrap()
    .encode(f.pixels())
    .unwrap()
    .into_vec()
}
pub fn readback(bytes: &[u8]) -> Vec<u8> {
    let (whole_info, whole) = imij::decode(bytes).unwrap();
    assert_eq!((whole_info.width, whole_info.height), (W, H));
    let mut streamed = Vec::new();
    let mut next = 0;
    let (reader_info, reader_meta) = imij::decode_reader(
        Cursor::new(bytes),
        &imij::Limits::DEFAULT,
        |info, y, row| {
            assert_eq!(y, next);
            assert_eq!(info, &whole_info);
            streamed.extend_from_slice(row);
            next += 1;
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(next, whole_info.height);
    assert_eq!(reader_info, whole_info);
    assert_eq!(streamed, whole);
    assert_eq!(reader_meta.xmp.as_deref(), metadata().xmp.as_deref());
    whole
}

pub struct ScratchDir(pub PathBuf);
impl ScratchDir {
    pub fn new() -> Self {
        static ID: AtomicU64 = AtomicU64::new(0);
        let base = PathBuf::from(
            std::env::var_os("MULTIPASS_SCRATCH")
                .expect("set MULTIPASS_SCRATCH to admitted disk scratch"),
        );
        let path = base.join(format!(
            "public-driver-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    pub fn native(&self) -> Scratch {
        Scratch::Dir(self.0.clone())
    }
    pub fn empty(&self) {
        assert_eq!(std::fs::read_dir(&self.0).unwrap().count(), 0);
    }
}
impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.0);
    } // Retain unexpected files on failure.
}
pub struct RawFile {
    pub path: PathBuf,
    file: Option<File>,
}
impl RawFile {
    pub fn new(dir: &ScratchDir) -> Self {
        let path = dir.0.join("raw-replay");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        Self {
            path,
            file: Some(file),
        }
    }
    pub fn take(&mut self) -> File {
        self.file.take().unwrap()
    }
}
impl Drop for RawFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[derive(Default)]
pub struct Counts {
    pub rows: AtomicU64,
    pub traversals: AtomicU64,
    pub calls: AtomicU64,
    pub dropped: AtomicBool,
    pub cancelled: AtomicBool,
}
#[derive(Clone, Copy)]
pub enum Fault {
    None,
    Source,
    Cancel,
    Depth,
    ReplayChange,
}
pub struct Rows<'a> {
    pub fixture: &'a Fixture,
    pub positional: bool,
    pub counts: Arc<Counts>,
    pub fault: Fault,
    next: u32,
}
impl<'a> Rows<'a> {
    pub fn new(fixture: &'a Fixture, positional: bool, counts: Arc<Counts>) -> Self {
        Self {
            fixture,
            positional,
            counts,
            fault: Fault::None,
            next: 0,
        }
    }
}
impl Drop for Rows<'_> {
    fn drop(&mut self) {
        self.counts.dropped.store(true, Ordering::SeqCst);
    }
}
impl Source for Rows<'_> {
    type Error = io::Error;
    fn description(&self) -> Description {
        self.fixture.description()
    }
    fn supports_positional_replay(&self) -> bool {
        self.positional
    }
    fn read_rows(
        &mut self,
        first: u32,
        mut rows: PixelSliceMut<'_>,
        _: &dyn zencodec::Pulse,
    ) -> io::Result<()> {
        self.counts.calls.fetch_add(1, Ordering::SeqCst);
        if first == 0 {
            if self.next != 0 {
                assert!(self.positional);
                assert_eq!(self.next, self.fixture.height);
            }
            self.next = 0;
            self.counts.traversals.fetch_add(1, Ordering::SeqCst);
        }
        assert_eq!(first, self.next);
        if matches!(self.fault, Fault::Source) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "retained-native-source-control",
            ));
        }
        let pixels = self.fixture.pixels();
        for y in 0..rows.rows() {
            rows.row_mut(y).copy_from_slice(pixels.row(first + y));
        }
        if matches!(self.fault, Fault::Depth) {
            rows.row_mut(0)[..2].copy_from_slice(&1u16.to_ne_bytes());
        }
        if matches!(self.fault, Fault::ReplayChange)
            && self.counts.traversals.load(Ordering::SeqCst) > 1
        {
            // Change a sample while preserving the declared 8->16 exact grid,
            // so this isolates replay hashing rather than depth verification.
            let old = u16::from_ne_bytes(rows.row_mut(0)[..2].try_into().unwrap());
            let changed = if old == 0 { 257u16 } else { 0u16 };
            rows.row_mut(0)[..2].copy_from_slice(&changed.to_ne_bytes());
        }
        self.next += rows.rows();
        self.counts
            .rows
            .fetch_add(u64::from(rows.rows()), Ordering::SeqCst);
        if matches!(self.fault, Fault::Cancel) {
            self.counts.cancelled.store(true, Ordering::SeqCst);
        }
        Ok(())
    }
}
pub struct Cancel(pub Arc<Counts>);
impl enough::Stop for Cancel {
    fn check(&self) -> Result<(), enough::StopReason> {
        if self.0.cancelled.load(Ordering::SeqCst) {
            Err(enough::StopReason::TimedOut)
        } else {
            Ok(())
        }
    }
}

#[derive(Default)]
pub struct Output {
    pub bytes: Cursor<Vec<u8>>,
    pub writes: u32,
    pub flushes: u32,
    pub seeks: u32,
}
pub struct Writer {
    pub output: Arc<Mutex<Output>>,
    pub source: Option<Arc<Counts>>,
    pub metadata: Weak<[u8]>,
    pub native_scratch: PathBuf,
}
impl Write for Writer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if let Some(source) = &self.source {
            assert!(
                source.dropped.load(Ordering::SeqCst),
                "source held during destination wait"
            );
        }
        assert!(
            self.metadata.upgrade().is_none(),
            "job still owns original metadata during publication"
        );
        assert_eq!(
            std::fs::read_dir(&self.native_scratch)?.count(),
            0,
            "native scratch held during publication"
        );
        let mut out = self.output.lock().unwrap();
        out.writes += 1;
        out.bytes.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.output.lock().unwrap().flushes += 1;
        Ok(())
    }
}
impl Seek for Writer {
    fn seek(&mut self, at: io::SeekFrom) -> io::Result<u64> {
        let mut out = self.output.lock().unwrap();
        out.seeks += 1;
        out.bytes.seek(at)
    }
}
pub fn run_setup(
    checksum: bool,
    native: &ScratchDir,
    counts: Option<Arc<Counts>>,
) -> (Run, Arc<Mutex<Output>>, StorageBudget) {
    let metadata = metadata();
    let weak = Arc::downgrade(metadata.xmp.as_ref().unwrap());
    let output = Arc::new(Mutex::new(Output::default()));
    let storage = StorageBudget::new(BUDGET, 1024);
    (
        Run {
            config: config(checksum),
            scratch: native.native(),
            metadata,
            limits: ResourceLimits::none(),
            options: Options::new(storage.clone(), BUDGET as u64),
            writer: Writer {
                output: output.clone(),
                source: counts,
                metadata: weak,
                native_scratch: native.0.clone(),
            },
        },
        output,
        storage,
    )
}
