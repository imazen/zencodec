//! Native gates: the prototype driver over IMI's real bounded multi-pass
//! encoder. Byte parity with the native public entry point (same settings,
//! same index placement) and exact decoded-sample parity with the ordinary
//! whole-image encoder. Every failure boundary keeps its original cause.
use enough::{Stop, StopReason, };
use zencodec::how_far::NoPulse;
use imij::stream_encode::{BoundedLossyConfig, LossyUnitIndex, encode_lossy_bounded};
use imij::{DecodeSpeed, ImageRef, ImiEncoderConfig, Metadata};
use multipass_native::{ImiBounded, ImiError, Scratch, image_info};
use multipass_validation::{
    Input, PassSource, Preset, ReadFrom, Refusal, Replay, ReplayDecision, ReplayFacts, ReplayOffer,
    ReplayPolicy, ReplayRoute, SourceIdentity, Stage, encode_passes,
    sources::{Borrowed, FileRows, ReaderRows},
};
use std::{
    cell::Cell,
    error::Error,
    fs::{File, OpenOptions},
    io::{self, Cursor, Write},
    os::unix::fs::FileExt,
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use zencodec::{
    At, find_cause,
    operation::{OutputSchedule, PolicySet, StagingKind},
    storage::StorageBudget,
};
use zencodec_io::{
    destination::{Forward, SeekBack},
    row_encode::Description,
};
use zenpixels::{ColorContext, PixelDescriptor, PixelSlice, PixelSliceMut};

const MAX_INDEX: u64 = 1 << 20;
const BIG: usize = 256 << 20;

// ------------------------------------------------------------------ fixtures

fn fill(w: usize, h: usize, n: usize, stride: usize, pad: u8) -> Vec<u8> {
    let mut out = vec![pad; stride * h];
    let mut s = 0x2545_F491_4F6C_DD1Du64;
    for y in 0..h {
        for x in 0..w {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            for c in 0..n {
                let v = ((x * 5 + y * 3 + c * 70) ^ (x * y / 7)) % 256;
                let edge = (x / 40 + y / 29).is_multiple_of(5);
                let noise = (s >> (8 * c)) as u8 & 0x1f;
                out[y * stride + x * n + c] = if edge {
                    250 - noise
                } else {
                    (v as u8).wrapping_add(noise)
                };
            }
        }
    }
    out
}

fn packed(src: &[u8], stride: usize, rb: usize) -> Vec<u8> {
    src.chunks(stride).flat_map(|r| r[..rb].to_vec()).collect()
}

struct Case {
    name: &'static str,
    w: u32,
    h: u32,
    desc: PixelDescriptor,
    cfg: fn() -> ImiEncoderConfig,
    checksum: bool,
}

fn cases() -> Vec<Case> {
    vec![
        Case {
            name: "fast-rgb8",
            w: 333,
            h: 211,
            desc: PixelDescriptor::RGB8_SRGB,
            cfg: || ImiEncoderConfig::lossy(70.0).with_decode_speed(DecodeSpeed::Fast),
            checksum: false,
        },
        Case {
            name: "compact-rgb8",
            w: 300,
            h: 260,
            desc: PixelDescriptor::RGB8_SRGB,
            cfg: || ImiEncoderConfig::lossy(70.0).with_decode_speed(DecodeSpeed::Compact),
            checksum: false,
        },
        Case {
            name: "default-rgba8-checksum",
            w: 257,
            h: 190,
            desc: PixelDescriptor::RGBA8_SRGB,
            cfg: || ImiEncoderConfig::lossy(40.0).with_checksum(true),
            checksum: true,
        },
        Case {
            name: "fast-rgb16",
            w: 200,
            h: 150,
            desc: PixelDescriptor::RGB16_SRGB,
            cfg: || ImiEncoderConfig::lossy(70.0).with_decode_speed(DecodeSpeed::Fast),
            checksum: false,
        },
    ]
}

impl Case {
    fn description(&self) -> Description {
        Description::new(self.w, self.h, self.desc)
    }
    fn rb(&self) -> usize {
        self.w as usize * self.desc.bytes_per_pixel()
    }
    fn raster(&self, extra: usize) -> (Vec<u8>, usize) {
        let stride = self.rb() + extra;
        let n = self.desc.bytes_per_pixel();
        (
            fill(self.w as usize * n, self.h as usize, 1, stride, 0xEE),
            stride,
        )
    }
    fn encoder(&self) -> ImiBounded {
        ImiBounded::new(
            (self.cfg)(),
            Metadata::default(),
            Scratch::Memory { cap: 64 << 20 },
            MAX_INDEX,
        )
    }
    fn native(&self, raster: &[u8], stride: usize, index: LossyUnitIndex) -> Vec<u8> {
        let info = image_info(&self.description()).unwrap();
        let mut src = Cursor::new(raster);
        let sums = self.checksum.then(|| Cursor::new(Vec::new()));
        let (out, _, _) = encode_lossy_bounded(
            &mut src,
            Cursor::new(Vec::new()),
            Cursor::new(Vec::new()),
            sums,
            info,
            &Metadata::default(),
            &(self.cfg)(),
            BoundedLossyConfig::new(stride as u64, index, MAX_INDEX),
            &how_far::NoPulse,
        )
        .unwrap();
        out.into_inner()
    }
    fn ordinary_pixels(&self, raster: &[u8], stride: usize) -> Vec<u8> {
        let info = image_info(&self.description()).unwrap();
        let file = (self.cfg)()
            .encode_samples(ImageRef::new(raster, stride, &info), &Metadata::default())
            .unwrap();
        decode(&file)
    }
}

fn decode(file: &[u8]) -> Vec<u8> {
    imij::decode(file).unwrap().1
}

fn options() -> multipass_validation::Options {
    multipass_validation::Options::new(StorageBudget::new(BIG, 1 << 16), u64::MAX)
}

fn scratch_path(name: &str) -> PathBuf {
    let dir = std::env::var_os("MULTIPASS_SCRATCH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_TARGET_TMPDIR")));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(format!("{name}-{}", std::process::id()))
}

fn rw_file(name: &str) -> (File, PathBuf) {
    let path = scratch_path(name);
    let f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)
        .unwrap();
    (f, path)
}

/// Each pass covers every row exactly once; returns the pass count.
fn assert_full_passes(trace: &multipass_validation::Trace, h: u32) -> u32 {
    assert!(
        trace.passes >= 2,
        "a multi-pass route read {} passes",
        trace.passes
    );
    for p in 0..trace.passes {
        let mut next = 0;
        for r in trace.reads.iter().filter(|r| r.pass == p) {
            assert_eq!(r.first, next, "pass {p} sequential coverage");
            next += r.rows;
        }
        assert_eq!(next, h, "pass {p} covers every row once");
    }
    trace.passes
}

fn run_borrowed(
    c: &Case,
    raster: &[u8],
    stride: usize,
    seek: bool,
) -> (Vec<u8>, multipass_validation::Completed) {
    let pixels = PixelSlice::new(raster, c.w, c.h, stride, c.desc).unwrap();
    let mut out = Cursor::new(Vec::new());
    let done = if seek {
        encode_passes(
            c.encoder(),
            Input::raster(pixels),
            SeekBack::new(&mut out).unwrap(),
            PolicySet::new(),
            Preset,
            options(),
            &NoPulse,
        )
    } else {
        encode_passes(
            c.encoder(),
            Input::raster(pixels),
            Forward(&mut out),
            PolicySet::new(),
            Preset,
            options(),
            &NoPulse,
        )
    }
    .unwrap();
    (out.into_inner(), done)
}

// --------------------------------------------------------------- parity gates

#[test]
fn borrowed_replay_matches_native_bytes_and_ordinary_decoded_samples() {
    for c in cases() {
        let (raster, stride) = c.raster(0);
        let ordinary = c.ordinary_pixels(&raster, stride);
        for seek in [true, false] {
            let (bytes, done) = run_borrowed(&c, &raster, stride, seek);
            let index = if seek {
                LossyUnitIndex::Reserved(4)
            } else {
                LossyUnitIndex::Trailing
            };
            assert_eq!(
                bytes,
                c.native(&raster, stride, index),
                "{} seek={seek}: native bytes",
                c.name
            );
            assert_eq!(
                decode(&bytes),
                ordinary,
                "{} seek={seek}: ordinary samples",
                c.name
            );
            assert_eq!(done.route, ReplayRoute::Source);
            let passes = assert_full_passes(&done.trace, c.h);
            assert!(done.trace.reads.iter().all(|r| r.from == ReadFrom::Source));
            assert_eq!(
                done.trace.source_bytes,
                u64::from(passes) * c.h as u64 * c.rb() as u64
            );
            assert_eq!(done.trace.staging_admitted, 0);
            assert_eq!(done.trace.digest_bytes, 0, "a borrow needs no digests");
            assert_eq!(
                done.patched_bytes > 0,
                seek,
                "{}: patches only with seek-back",
                c.name
            );
            eprintln!(
                "{} seek={seek}: {} passes, {} bytes, labels {:?}",
                c.name,
                passes,
                bytes.len(),
                labels(&done.trace)
            );
        }
    }
}

fn labels(t: &multipass_validation::Trace) -> Vec<u8> {
    let mut v: Vec<u8> = Vec::new();
    for r in &t.reads {
        if v.len() <= r.pass as usize {
            v.push(r.label);
        }
    }
    v
}

#[test]
fn padded_rows_never_reach_the_encoder() {
    let c = &cases()[0];
    let (tight, stride) = c.raster(0);
    let (a, _) = run_borrowed(c, &tight, stride, true);
    // PixelSlice strides are pixel-aligned; the file test covers unaligned strides.
    for (extra, pad) in [(15usize, 0x00u8), (15, 0xFF), (66, 0x5A)] {
        let n = c.rb() + extra;
        let mut padded = vec![pad; n * c.h as usize];
        for y in 0..c.h as usize {
            padded[y * n..][..c.rb()].copy_from_slice(&tight[y * stride..][..c.rb()]);
        }
        let (b, _) = run_borrowed(c, &padded, n, true);
        assert_eq!(a, b, "padding {extra}x{pad:#x} changed output");
    }
}

#[test]
fn positional_file_replay_is_digest_verified_and_matches() {
    for c in cases().into_iter().take(2) {
        let (tight, stride) = c.raster(0);
        let expect = c.native(&tight, stride, LossyUnitIndex::Reserved(4));
        let extra = 29;
        let n = c.rb() + extra;
        let (file, _path) = rw_file(c.name);
        let origin = 101u64;
        file.write_all_at(&[9u8; 101], 0).unwrap();
        for y in 0..c.h as usize {
            let mut row = tight[y * stride..][..c.rb()].to_vec();
            row.extend(std::iter::repeat_n(0xA5u8, extra));
            file.write_all_at(&row, origin + (y * n) as u64).unwrap();
        }
        let mut out = Cursor::new(Vec::new());
        let done = encode_passes(
            c.encoder(),
            Input::source(FileRows::new(file, origin, n as u64, c.description())),
            SeekBack::new(&mut out).unwrap(),
            PolicySet::new(),
            Preset,
            options(),
            &NoPulse,
        )
        .unwrap();
        assert_eq!(out.into_inner(), expect, "{}", c.name);
        assert_eq!(done.route, ReplayRoute::Source);
        assert_full_passes(&done.trace, c.h);
        assert_eq!(done.trace.digest_bytes, c.h as u64 * 8);
    }
}

#[test]
fn forward_source_replays_from_admitted_memory_or_disk_staging_only() {
    let c = &cases()[1];
    let (tight, stride) = c.raster(0);
    let expect = c.native(&tight, stride, LossyUnitIndex::Reserved(4));
    let pass_bytes = (c.rb() * c.h as usize) as u64;
    let n = c.rb() + 7;
    let mut stream = Vec::new();
    for y in 0..c.h as usize {
        stream.extend_from_slice(&tight[y * stride..][..c.rb()]);
        stream.extend_from_slice(&[3u8; 7]);
    }
    for kind in [StagingKind::Memory, StagingKind::Disk] {
        let budget = StorageBudget::new(BIG, 1 << 16);
        let (file, _path) = rw_file("stage");
        let mut asked: Option<(ReplayFacts, Vec<ReplayOffer>)> = None;
        let mut out = Cursor::new(Vec::new());
        let done = encode_passes(
            c.encoder(),
            Input::source(ReaderRows::new(Cursor::new(&stream), n, c.description())),
            SeekBack::new(&mut out).unwrap(),
            PolicySet::new(),
            |f: &ReplayFacts,
             o: &[ReplayOffer]|
             -> Result<ReplayDecision, Box<dyn Error + Send + Sync>> {
                asked = Some((*f, o.to_vec()));
                Ok(ReplayDecision::Choose(
                    o.iter()
                        .position(|x| x.route == ReplayRoute::Stage(kind))
                        .unwrap(),
                ))
            },
            multipass_validation::Options::new(budget.clone(), u64::MAX)
                .with_stage_file(file, pass_bytes),
            &NoPulse,
        )
        .unwrap();
        assert_eq!(out.into_inner(), expect, "{kind:?}");
        let (facts, offers) = asked.expect("forward source asks the replay question");
        assert_eq!(facts.replay, Replay::Forward);
        assert_eq!(facts.pass_bytes, pass_bytes);
        assert!(
            offers.iter().all(|o| o.route != ReplayRoute::Source),
            "forward never offers rereads"
        );
        assert_eq!(done.route, ReplayRoute::Stage(kind));
        let passes = assert_full_passes(&done.trace, c.h);
        assert_eq!(
            done.trace.source_bytes, pass_bytes,
            "the source is read exactly once"
        );
        assert_eq!(done.trace.staging_reads, u64::from(passes - 1) * pass_bytes);
        assert!(
            done.trace
                .reads
                .iter()
                .all(|r| (r.pass == 0) == (r.from == ReadFrom::Source))
        );
        assert_eq!(done.trace.staging_admitted, pass_bytes);
        assert_eq!(
            done.trace.digest_bytes,
            if kind == StagingKind::Disk {
                c.h as u64 * 8
            } else {
                0
            }
        );
        assert_eq!(
            budget.reserved_bytes(),
            0,
            "staging released after completion"
        );
    }
}

// ------------------------------------------------------------ refusal gates

struct Counting<S> {
    inner: S,
    reads: Rc<Cell<u32>>,
}
impl<S: PassSource> PassSource for Counting<S> {
    type Error = S::Error;
    fn description(&self) -> Description {
        self.inner.description()
    }
    fn replay(&self) -> Replay {
        self.inner.replay()
    }
    fn identity(&mut self) -> Result<Option<SourceIdentity>, Self::Error> {
        self.inner.identity()
    }
    fn read_rows(
        &mut self,
        first: u32,
        rows: PixelSliceMut<'_>,
        stop: &dyn zencodec::Pulse,
    ) -> Result<(), Self::Error> {
        self.reads.set(self.reads.get() + 1);
        self.inner.read_rows(first, rows, stop)
    }
}

fn refusal(e: &At<multipass_validation::Failure>) -> Option<Refusal> {
    find_cause::<Refusal>(e).copied()
}

#[test]
fn forward_without_admitted_staging_refuses_before_reading() {
    let c = &cases()[0];
    let (tight, stride) = c.raster(0);
    let reads = Rc::new(Cell::new(0));
    let mut out = Vec::new();
    // Budget too small for a raw pass, and no disk file.
    let err = encode_passes(
        c.encoder(),
        Input::source(Counting {
            inner: ReaderRows::new(Cursor::new(&tight), stride, c.description()),
            reads: reads.clone(),
        }),
        Forward(&mut out),
        PolicySet::new(),
        |_: &ReplayFacts,
         _: &[ReplayOffer]|
         -> Result<ReplayDecision, Box<dyn Error + Send + Sync>> {
            panic!("nothing to decide")
        },
        multipass_validation::Options::new(StorageBudget::new(4096, 16), u64::MAX),
        &NoPulse,
    )
    .unwrap_err();
    assert_eq!(refusal(&err), Some(Refusal::NotReplayable));
    assert_eq!(err.error().stage, Stage::Policy);
    assert_eq!(reads.get(), 0);
    assert!(out.is_empty());
}

#[derive(Debug)]
struct Tenant;
impl std::fmt::Display for Tenant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("tenant forbids raw staging")
    }
}
impl Error for Tenant {}

#[test]
fn replay_policy_decline_and_error_keep_cause_without_reads() {
    let c = &cases()[0];
    let (tight, stride) = c.raster(0);
    for fail in [false, true] {
        let reads = Rc::new(Cell::new(0));
        let mut out = Vec::new();
        let err = encode_passes(
            c.encoder(),
            Input::source(Counting {
                inner: ReaderRows::new(Cursor::new(&tight), stride, c.description()),
                reads: reads.clone(),
            }),
            Forward(&mut out),
            PolicySet::new(),
            |_: &ReplayFacts,
             _: &[ReplayOffer]|
             -> Result<ReplayDecision, Box<dyn Error + Send + Sync>> {
                if fail {
                    Err(Box::new(Tenant))
                } else {
                    Ok(ReplayDecision::Decline)
                }
            },
            options(),
            &NoPulse,
        )
        .unwrap_err();
        assert_eq!(err.error().stage, Stage::Policy);
        if fail {
            assert!(
                find_cause::<Tenant>(&err).is_some(),
                "application cause retained"
            );
        } else {
            assert_eq!(refusal(&err), Some(Refusal::Declined));
        }
        assert_eq!(reads.get(), 0);
        assert!(out.is_empty());
    }
}

#[test]
fn source_pass_budget_stages_or_refuses_before_reading() {
    let c = &cases()[0];
    let (tight, stride) = c.raster(0);
    let pixels = PixelSlice::new(&tight, c.w, c.h, stride, c.desc).unwrap();
    // One source pass allowed: a borrowed raster is staged once in memory.
    let mut out = Cursor::new(Vec::new());
    let done = encode_passes(
        c.encoder(),
        Input::raster(pixels),
        SeekBack::new(&mut out).unwrap(),
        PolicySet::new(),
        Preset,
        options().with_replay_policy(ReplayPolicy {
            max_source_passes: Some(1),
            ..Default::default()
        }),
        &NoPulse,
    )
    .unwrap();
    assert_eq!(done.route, ReplayRoute::Stage(StagingKind::Memory));
    assert_eq!(
        out.into_inner(),
        c.native(&tight, stride, LossyUnitIndex::Reserved(4))
    );
    // ...and refused when memory staging is forbidden too.
    let pixels = PixelSlice::new(&tight, c.w, c.h, stride, c.desc).unwrap();
    let mut out = Vec::new();
    let err = encode_passes(
        c.encoder(),
        Input::raster(pixels),
        Forward(&mut out),
        PolicySet::new(),
        Preset,
        options().with_replay_policy(ReplayPolicy {
            max_source_passes: Some(1),
            forbid_memory_staging: true,
            ..Default::default()
        }),
        &NoPulse,
    )
    .unwrap_err();
    assert_eq!(refusal(&err), Some(Refusal::PassBudget { pass: 0 }));
    assert!(out.is_empty());
}

#[test]
fn source_pass_budget_refuses_at_the_pass_that_exceeds_it() {
    // Find a configuration needing more than two passes, then cap at two.
    let c = &cases()[1];
    let (tight, stride) = c.raster(0);
    let (_, done) = run_borrowed(c, &tight, stride, true);
    let passes = done.trace.passes;
    assert!(passes > 2, "compact tier reads {passes} passes");
    let pixels = PixelSlice::new(&tight, c.w, c.h, stride, c.desc).unwrap();
    let mut out = Cursor::new(Vec::new());
    let err = encode_passes(
        c.encoder(),
        Input::raster(pixels),
        SeekBack::new(&mut out).unwrap(),
        PolicySet::new(),
        Preset,
        options().with_replay_policy(ReplayPolicy {
            max_source_passes: Some(2),
            forbid_memory_staging: true,
            ..Default::default()
        }),
        &NoPulse,
    )
    .unwrap_err();
    assert_eq!(refusal(&err), Some(Refusal::PassBudget { pass: 2 }));
    assert_eq!(err.error().emitted, 0);
    assert_eq!(err.error().trace.passes, 2);
}

/// A positional source whose content changes when pass `at` starts. With
/// `hide_identity` it reports a constant identity, so only digests can see it.
struct Mutating {
    inner: FileRows,
    file: File,
    at: u32,
    rows_seen: u32,
    height: u32,
    hide_identity: bool,
    change_context: bool,
    desc: Description,
}
impl PassSource for Mutating {
    type Error = io::Error;
    fn description(&self) -> Description {
        self.desc.clone()
    }
    fn replay(&self) -> Replay {
        Replay::Positional
    }
    fn identity(&mut self) -> io::Result<Option<SourceIdentity>> {
        if self.hide_identity {
            Ok(Some(SourceIdentity([1, 2, 3, 4])))
        } else {
            self.inner.identity()
        }
    }
    fn read_rows(
        &mut self,
        first: u32,
        rows: PixelSliceMut<'_>,
        stop: &dyn zencodec::Pulse,
    ) -> io::Result<()> {
        let n = rows.rows();
        self.inner.read_rows(first, rows, stop)?;
        self.rows_seen += n;
        if self.rows_seen == self.at * self.height {
            if self.change_context {
                self.desc =
                    self.desc
                        .clone()
                        .with_color_context(Arc::new(ColorContext::from_cicp(
                            zencodec::Cicp::DISPLAY_P3,
                        )));
            } else {
                // Rewrite one sample in the middle of the image.
                let off = (self.height as u64 / 2) * 900 + 17;
                let mut b = [0u8];
                self.file.read_exact_at(&mut b, off)?;
                self.file.write_all_at(&[b[0] ^ 0x40], off)?;
                self.file.sync_data()?;
            }
        }
        Ok(())
    }
}

#[test]
fn changed_positional_content_identity_or_interpretation_refuses_before_output() {
    let c = &cases()[0];
    assert_eq!(c.rb(), 999);
    let (tight, stride) = c.raster(0);
    let stride_file = c.rb() as u64;
    for (hide_identity, change_context) in [(true, false), (false, false), (true, true)] {
        let (file, _path) = rw_file("mutating");
        file.write_all_at(&packed(&tight, stride, c.rb()), 0)
            .unwrap();
        // The mutation offset uses 900-byte rows; it still lands inside the raster.
        let src = Mutating {
            inner: FileRows::new(file.try_clone().unwrap(), 0, stride_file, c.description()),
            file,
            at: 1,
            rows_seen: 0,
            height: c.h,
            hide_identity,
            change_context,
            desc: c.description(),
        };
        let mut out = Cursor::new(Vec::new());
        let err = encode_passes(
            c.encoder(),
            Input::source(src),
            SeekBack::new(&mut out).unwrap(),
            PolicySet::new(),
            Preset,
            options(),
            &NoPulse,
        )
        .unwrap_err();
        // A declaration change is seen right after the read that made it (the
        // last request of pass 0); content/identity changes when pass 1 reads.
        let pass = if change_context { 0 } else { 1 };
        assert_eq!(
            refusal(&err),
            Some(Refusal::SourceChanged { pass }),
            "hide_identity={hide_identity} change_context={change_context}"
        );
        assert_eq!(err.error().stage, Stage::Read);
        assert_eq!(err.error().emitted, 0);
        assert!(
            out.into_inner().is_empty(),
            "no output from inconsistent passes"
        );
        // Digest refusals happen at the changed row, not before pass 1 starts.
        let read_in_pass1 = err
            .error()
            .trace
            .reads
            .iter()
            .filter(|r| r.pass == 1)
            .count();
        eprintln!(
            "identity hidden={hide_identity} context={change_context}: {read_in_pass1} pass-1 requests before refusal"
        );
        if !hide_identity || change_context {
            assert_eq!(
                read_in_pass1, 0,
                "identity/declaration checks refuse before pass 1 reads"
            );
        } else {
            assert!(
                read_in_pass1 >= 1,
                "only the digest of the changed row can see it"
            );
        }
    }
}

struct TripAfter {
    n: usize,
    count: AtomicUsize,
}
impl Stop for TripAfter {
    fn check(&self) -> Result<(), StopReason> {
        if self.count.fetch_add(1, Ordering::Relaxed) >= self.n {
            Err(StopReason::Cancelled)
        } else {
            Ok(())
        }
    }
}

#[test]
fn cancellation_at_every_boundary_keeps_reason_and_writes_nothing_finish_first() {
    let c = &cases()[0];
    let (tight, stride) = c.raster(0);
    // Count checks of a full run, then trip at spread points through it.
    let probe = TripAfter {
        n: usize::MAX,
        count: AtomicUsize::new(0),
    };
    let pixels = PixelSlice::new(&tight, c.w, c.h, stride, c.desc).unwrap();
    let mut sink = Cursor::new(Vec::new());
    encode_passes(
        c.encoder(),
        Input::raster(pixels),
        SeekBack::new(&mut sink).unwrap(),
        PolicySet::new(),
        Preset,
        options(),
        &probe,
    )
    .unwrap();
    let total = probe.count.load(Ordering::Relaxed);
    let mut stages = std::collections::BTreeSet::new();
    for k in (0..total).step_by((total / 23).max(1)) {
        let stop = TripAfter {
            n: k,
            count: AtomicUsize::new(0),
        };
        let mut out = Cursor::new(Vec::new());
        let pixels = PixelSlice::new(&tight, c.w, c.h, stride, c.desc).unwrap();
        let err = encode_passes(
            c.encoder(),
            Input::raster(pixels),
            SeekBack::new(&mut out).unwrap(),
            PolicySet::new(),
            Preset,
            options(),
            &stop,
        )
        .unwrap_err();
        assert!(
            find_cause::<zencodec::CodecError>(&err).is_some(),
            "k={k}: stop reason retained"
        );
        assert!(
            find_cause::<ImiError>(&err).is_none(),
            "k={k}: native error not substituted for the stop"
        );
        stages.insert(format!("{:?}/{:?}", err.error().stage, err.error().pass));
        if err.error().stage != Stage::Write && err.error().stage != Stage::Patch {
            assert_eq!(err.error().emitted, 0);
            assert!(out.into_inner().is_empty());
        }
    }
    eprintln!("cancelled at {total} checks across {stages:?}");
    assert!(stages.len() >= 3);
}

struct FailingRead {
    inner: FileRows,
    fail_pass_row: (u32, u32),
    rows_seen: u32,
    height: u32,
}
impl PassSource for FailingRead {
    type Error = io::Error;
    fn description(&self) -> Description {
        self.inner.description()
    }
    fn replay(&self) -> Replay {
        Replay::Positional
    }
    fn identity(&mut self) -> io::Result<Option<SourceIdentity>> {
        self.inner.identity()
    }
    fn read_rows(
        &mut self,
        first: u32,
        rows: PixelSliceMut<'_>,
        stop: &dyn zencodec::Pulse,
    ) -> io::Result<()> {
        let pass = self.rows_seen / self.height;
        if (pass, first) >= self.fail_pass_row {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionReset,
                "range read failed",
            ));
        }
        self.rows_seen += rows.rows();
        self.inner.read_rows(first, rows, stop)
    }
}

/// Writes at most `limit` bytes, then fails; records reads observed before
/// the first byte.
struct Sink {
    bytes: Vec<u8>,
    limit: usize,
}
impl Write for Sink {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        if self.bytes.len() >= self.limit {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        let n = b.len().min(self.limit - self.bytes.len()).min(97);
        self.bytes.extend_from_slice(&b[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn partial_failures_keep_original_cause_stage_pass_and_confirmed_bytes() {
    let c = &cases()[0];
    let (tight, stride) = c.raster(0);
    // Source read failure in the second pass.
    let (file, _p) = rw_file("failing");
    file.write_all_at(&packed(&tight, stride, c.rb()), 0)
        .unwrap();
    let mut out = Cursor::new(Vec::new());
    let err = encode_passes(
        c.encoder(),
        Input::source(FailingRead {
            inner: FileRows::new(file, 0, c.rb() as u64, c.description()),
            fail_pass_row: (1, 64),
            rows_seen: 0,
            height: c.h,
        }),
        SeekBack::new(&mut out).unwrap(),
        PolicySet::new(),
        Preset,
        options(),
        &NoPulse,
    )
    .unwrap_err();
    assert_eq!(err.error().stage, Stage::Read);
    assert_eq!(err.error().pass, Some(1));
    assert_eq!(
        find_cause::<io::Error>(&err).unwrap().kind(),
        io::ErrorKind::ConnectionReset
    );
    assert!(
        find_cause::<ImiError>(&err).is_none(),
        "driver cause is primary"
    );
    assert!(out.into_inner().is_empty());

    // Disk staging write failure (read-only scratch).
    let (_, path) = rw_file("readonly");
    let ro = File::open(&path).unwrap();
    let mut out = Vec::new();
    let err = encode_passes(
        c.encoder(),
        Input::source(ReaderRows::new(
            Cursor::new(&tight),
            stride,
            c.description(),
        )),
        Forward(&mut out),
        PolicySet::new(),
        Preset,
        options()
            .with_stage_file(ro, u64::MAX)
            .with_replay_policy(ReplayPolicy {
                forbid_memory_staging: true,
                ..Default::default()
            }),
        &NoPulse,
    )
    .unwrap_err();
    assert_eq!(err.error().stage, Stage::Staging);
    assert_eq!(err.error().pass, Some(0));
    assert!(find_cause::<io::Error>(&err).is_some());
    assert!(out.is_empty());

    // Live output: writer fails mid-stream after confirmed bytes.
    let (expect, _) = run_borrowed(c, &tight, stride, false);
    for limit in [0usize, 1, expect.len() / 2, expect.len() - 1] {
        let pixels = PixelSlice::new(&tight, c.w, c.h, stride, c.desc).unwrap();
        let mut sink = Sink {
            bytes: Vec::new(),
            limit,
        };
        let err = encode_passes(
            c.encoder(),
            Input::raster(pixels),
            Forward(&mut sink),
            PolicySet::new().require_output_schedule(OutputSchedule::Live),
            Preset,
            options(),
            &NoPulse,
        )
        .unwrap_err();
        assert_eq!(err.error().stage, Stage::Write, "limit {limit}");
        assert_eq!(err.error().emitted, limit as u64);
        assert_eq!(sink.bytes, expect[..limit], "confirmed prefix is exact");
        assert_eq!(
            find_cause::<io::Error>(&err).unwrap().kind(),
            io::ErrorKind::BrokenPipe
        );
    }

    // Output allowance.
    let pixels = PixelSlice::new(&tight, c.w, c.h, stride, c.desc).unwrap();
    let mut out = Vec::new();
    let err = encode_passes(
        c.encoder(),
        Input::raster(pixels),
        Forward(&mut out),
        PolicySet::new(),
        Preset,
        multipass_validation::Options::new(StorageBudget::new(BIG, 1 << 16), 1000),
        &NoPulse,
    )
    .unwrap_err();
    assert_eq!(refusal(&err), Some(Refusal::OutputLimit));
    assert!(out.is_empty());
}

#[test]
fn live_output_starts_only_in_the_final_pass_and_matches() {
    let c = &cases()[1];
    let (tight, stride) = c.raster(0);
    let (expect, done) = run_borrowed(c, &tight, stride, false);
    let passes = done.trace.passes;
    let reads = Rc::new(Cell::new(0u32));
    struct Watch {
        bytes: Vec<u8>,
        reads: Rc<Cell<u32>>,
        first: Option<u32>,
    }
    impl Write for Watch {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.first.get_or_insert(self.reads.get());
            self.bytes.extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let pixels = PixelSlice::new(&tight, c.w, c.h, stride, c.desc).unwrap();
    let mut w = Watch {
        bytes: Vec::new(),
        reads: reads.clone(),
        first: None,
    };
    let live = encode_passes(
        c.encoder(),
        Input::source(Counting {
            inner: Borrowed::new(pixels),
            reads: reads.clone(),
        }),
        Forward(&mut w),
        PolicySet::new().require_output_schedule(OutputSchedule::Live),
        Preset,
        options(),
        &NoPulse,
    )
    .unwrap();
    assert_eq!(w.bytes, expect);
    assert_eq!(live.schedule, OutputSchedule::Live);
    let before_final = done
        .trace
        .reads
        .iter()
        .filter(|r| r.pass + 1 < passes)
        .count() as u32;
    let first = w.first.unwrap();
    assert!(
        first >= before_final,
        "first byte after {first} reads; {before_final} precede the final pass"
    );
    assert!(first < reads.get(), "bytes stream during the final pass");
}

#[test]
fn unsupported_interpretation_refuses_at_prepare_without_relabeling() {
    let c = &cases()[0];
    let (tight, stride) = c.raster(0);
    let p3 = Arc::new(ColorContext::from_cicp(zencodec::Cicp::DISPLAY_P3));
    let pixels = PixelSlice::new(&tight, c.w, c.h, stride, c.desc)
        .unwrap()
        .with_color_context(p3);
    let mut out = Vec::new();
    let err = encode_passes(
        c.encoder(),
        Input::raster(pixels),
        Forward(&mut out),
        PolicySet::new(),
        Preset,
        options(),
        &NoPulse,
    )
    .unwrap_err();
    assert_eq!(err.error().stage, Stage::Prepare);
    assert!(matches!(
        find_cause::<ImiError>(&err),
        Some(ImiError::Unsupported(_))
    ));
    assert!(err.error().trace.reads.is_empty());
    assert!(out.is_empty());
    // A matching sRGB context is carried on every request.
    let srgb = Arc::new(ColorContext::from_cicp(zencodec::Cicp::SRGB));
    let pixels = PixelSlice::new(&tight, c.w, c.h, stride, c.desc)
        .unwrap()
        .with_color_context(srgb);
    let mut out = Cursor::new(Vec::new());
    encode_passes(
        c.encoder(),
        Input::raster(pixels),
        SeekBack::new(&mut out).unwrap(),
        PolicySet::new(),
        Preset,
        options(),
        &NoPulse,
    )
    .unwrap();
    assert_eq!(
        out.into_inner(),
        c.native(&tight, stride, LossyUnitIndex::Reserved(4))
    );
}
