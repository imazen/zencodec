//! Candidate pass-plan contract against real IMI bounded lossy encoding
//! (copied into the candidate export's `native/tests/`). The adapter takes
//! its pass range and band from `BoundedLossySession::passes()`, computed
//! from the native plan before any source row.
use imij::stream_encode::{BoundedLossyConfig, LossyUnitIndex, encode_lossy_bounded};
use imij::{DecodeSpeed, ImiEncoderConfig, Metadata};
use multipass_native::{ImiBounded, Scratch, image_info};
use multipass_validation::{
    Input, Interrupted, MultiPassEncoder, Options, PassCount, PassOutput, PassProgress, PassRows,
    PassSource, Preset, Refusal, Replay, ReplayPolicy, ReplayRoute, RowRequest, Stage,
    encode_passes,
};
use std::{
    io::{self, Cursor},
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering::Relaxed},
    },
};
use zencodec::how_far::NoPulse;
use zencodec::{
    encode::OutputTiming, find_cause, operation::PolicySet, output_patch::OutputAccess,
    storage::StorageBudget,
};
use zencodec_io::{
    destination::{Forward, SeekBack},
    row_encode::Description,
};
use zenpixels::{PixelDescriptor, PixelSlice, PixelSliceMut};

const MAX_INDEX: u64 = 1 << 20;
const W: u32 = 420;
const H: u32 = 330;

/// Fast tier with exact blocks: the nested pass runs only when blocks are
/// selected, so content decides between the plan's two counts.
fn exact() -> ImiEncoderConfig {
    ImiEncoderConfig::lossy(70.0)
        .with_decode_speed(DecodeSpeed::Fast)
        .tune_lossy(|t| t.exact_blocks = Some(1.5))
}

fn encoder(cfg: ImiEncoderConfig) -> ImiBounded {
    ImiBounded::new(
        cfg,
        Metadata::default(),
        Scratch::Memory { cap: 64 << 20 },
        MAX_INDEX,
    )
}

/// Flat white panels and strokes (exact blocks qualify), plus texture.
fn graphic(w: u32, h: u32) -> Vec<u8> {
    let (w, h) = (w as usize, h as usize);
    let mut out = vec![0u8; w * h * 3];
    for y in 0..h {
        for x in 0..w {
            for c in 0..3 {
                out[(y * w + x) * 3 + c] = match (x / 23 + y / 17) % 4 {
                    0 => 255,
                    1 if (x + y) % 5 < 2 => 20,
                    1 => 240,
                    _ => (((x * 5 + y * 3 + c * 70) ^ (x * y / 7)) % 256) as u8,
                };
            }
        }
    }
    out
}

/// A smooth horizontal gradient: no exact block qualifies.
fn gradient(w: u32, h: u32) -> Vec<u8> {
    let w = w as usize;
    (0..w * h as usize * 3)
        .map(|i| ((i / 3) % w * 200 / w) as u8)
        .collect()
}

fn desc(w: u32, h: u32) -> Description {
    Description::new(w, h, PixelDescriptor::RGB8_SRGB)
}

fn options(policy: ReplayPolicy) -> Options {
    Options::new(StorageBudget::new(256 << 20, 1 << 16), u64::MAX).with_replay_policy(policy)
}

fn native(cfg: &ImiEncoderConfig, px: &[u8], w: u32, h: u32, index: LossyUnitIndex) -> Vec<u8> {
    let info = image_info(&desc(w, h)).unwrap();
    let (out, _, _) = encode_lossy_bounded(
        &mut Cursor::new(px),
        Cursor::new(Vec::new()),
        Cursor::new(Vec::new()),
        None::<Cursor<Vec<u8>>>,
        info,
        &Metadata::default(),
        cfg,
        BoundedLossyConfig::new(u64::from(w) * 3, index, MAX_INDEX),
        &NoPulse,
    )
    .unwrap();
    out.into_inner()
}

/// Supplies rows from a packed raster and logs every request.
struct Feed<'a> {
    px: &'a [u8],
    rb: usize,
    /// (pass ordinal, final flag, first row, rows)
    log: Vec<(u32, bool, u32, u32)>,
}
impl PassRows for Feed<'_> {
    fn fill(&mut self, request: RowRequest<'_>) -> Result<(), Interrupted> {
        let RowRequest {
            pass,
            final_pass,
            first,
            mut rows,
            ..
        } = request;
        for y in 0..rows.rows() {
            let at = (first + y) as usize * self.rb;
            rows.row_mut(y).copy_from_slice(&self.px[at..at + self.rb]);
        }
        self.log.push((pass, final_pass, first, rows.rows()));
        Ok(())
    }
}

/// One pull: the request it filled (None at Done) and the bytes it made pending.
struct Pull {
    request: Option<(u32, bool, u32, u32)>,
    pending: usize,
}

/// Drives the adapter directly; drains after every pull. Returns the plan,
/// every pull, and the complete output with finish tail and patches applied.
fn drive(
    cfg: ImiEncoderConfig,
    px: &[u8],
    w: u32,
    h: u32,
    access: OutputAccess,
) -> (multipass_validation::PassPlan, Vec<Pull>, Vec<u8>) {
    let mut enc = encoder(cfg);
    let plan = enc.begin_passes(&desc(w, h), access).unwrap();
    let mut feed = Feed {
        px,
        rb: w as usize * 3,
        log: Vec::new(),
    };
    let mut pulls = Vec::new();
    let mut out = Vec::new();
    loop {
        let before = feed.log.len();
        let progress = enc.pull_rows(&mut feed, Some(&NoPulse)).unwrap();
        let pending = enc.pending_output().len();
        out.extend_from_slice(enc.pending_output());
        enc.consume_output(pending);
        pulls.push(Pull {
            request: (feed.log.len() > before).then(|| *feed.log.last().unwrap()),
            pending,
        });
        if progress == PassProgress::Done {
            break;
        }
    }
    let finished = enc.finish_output().unwrap();
    out.extend_from_slice(finished.tail.data());
    for patch in &finished.patches {
        let at = patch.offset as usize;
        out[at..at + patch.bytes.len()].copy_from_slice(&patch.bytes);
    }
    (plan, pulls, out)
}

#[test]
fn imi_plans_a_pass_range_and_content_reaches_each_end() {
    let (plan, _, _) = drive(exact(), &gradient(W, H), W, H, OutputAccess::SeekBack);
    assert_eq!(plan.passes, PassCount::Range { min: 3, max: 4 });
    assert_eq!(plan.output, PassOutput::FinalPass);
    for (name, px, want) in [
        ("graphic", graphic(W, H), 4),
        ("gradient", gradient(W, H), 3),
    ] {
        let pixels =
            PixelSlice::new(&px, W, H, W as usize * 3, PixelDescriptor::RGB8_SRGB).unwrap();
        let mut out = Cursor::new(Vec::new());
        let done = encode_passes(
            encoder(exact()),
            Input::raster(pixels),
            SeekBack::new(&mut out).unwrap(),
            PolicySet::new(),
            Preset,
            options(ReplayPolicy::default()),
            Some(&NoPulse),
        )
        .unwrap();
        assert_eq!(done.trace.passes, want, "{name}");
        assert_eq!(
            out.into_inner(),
            native(&exact(), &px, W, H, LossyUnitIndex::Reserved(4)),
            "{name}: native bytes"
        );
    }
}

#[test]
fn imi_output_starts_in_the_coding_pass_and_holds_one_band_within_it() {
    let px = graphic(W, H);
    let (plan, pulls, out) = drive(exact(), &px, W, H, OutputAccess::SeekBack);
    assert_eq!(
        out,
        native(&exact(), &px, W, H, LossyUnitIndex::Reserved(4))
    );
    let band = pulls
        .iter()
        .filter_map(|p| p.request.map(|r| r.3))
        .max()
        .unwrap();
    assert_eq!(plan.timing.lag_rows(), Some(band), "lag is one native band");
    assert!(band < H);

    // Analysis passes: nothing pending after any pull.
    let first_final = pulls
        .iter()
        .position(|p| p.request.is_some_and(|r| r.1))
        .unwrap();
    for (i, p) in pulls[..first_final].iter().enumerate() {
        assert_eq!(p.pending, 0, "pull {i} before the coding pass");
    }
    // The replaced declaration promised row 0 pending once H rows were
    // supplied. After pass 0 (H rows) nothing is pending.
    let legacy = OutputTiming::streaming(H - 1);
    let end_of_first_pass = pulls
        .iter()
        .position(|p| p.request.is_some_and(|r| r.0 == 0 && r.2 + r.3 == H))
        .unwrap();
    assert_eq!(pulls[end_of_first_pass].pending, 0);
    assert!(legacy.lag_rows().is_some_and(|lag| lag < H));
    assert_ne!(plan.timing, legacy);

    // Coding pass, measured within it: the first pull also carries the head
    // written before the first coding request; every later pull carries the
    // previous band's records, so at most one request's rows are held.
    let finals: Vec<&Pull> = pulls[first_final..]
        .iter()
        .filter(|p| p.request.is_some())
        .collect();
    assert!(finals.len() > 2, "multi-band coding pass");
    for (i, p) in finals.iter().enumerate() {
        assert!(p.pending > 0, "coding pull {i}: bytes pending");
    }
    let supplied_before_first_byte: u32 = pulls[..first_final]
        .iter()
        .filter_map(|p| p.request.map(|r| r.3))
        .sum();
    eprintln!(
        "imi exact graphic {W}x{H}: {:?}, band {band}, {} rows supplied before the coding pass",
        plan.passes, supplied_before_first_byte
    );
}

/// A positional in-memory source counting read attempts.
struct Mem {
    px: Vec<u8>,
    rows: Arc<AtomicU32>,
}
impl PassSource for Mem {
    type Error = io::Error;
    fn description(&self) -> Description {
        desc(W, H)
    }
    fn replay(&self) -> Replay {
        Replay::Positional
    }
    fn read_rows(
        &mut self,
        first: u32,
        mut rows: PixelSliceMut<'_>,
        _: Option<&dyn zencodec::Pulse>,
    ) -> io::Result<()> {
        self.rows.fetch_add(rows.rows(), Relaxed);
        let rb = W as usize * 3;
        for y in 0..rows.rows() {
            let at = (first + y) as usize * rb;
            rows.row_mut(y).copy_from_slice(&self.px[at..at + rb]);
        }
        Ok(())
    }
}

#[test]
fn a_budget_below_imis_planned_maximum_refuses_before_any_read() {
    // The gradient needs three passes, but nothing tells before reading it.
    let rows = Arc::new(AtomicU32::new(0));
    let mut out = Vec::new();
    let err = encode_passes(
        encoder(exact()),
        Input::source(Mem {
            px: gradient(W, H),
            rows: rows.clone(),
        }),
        Forward(&mut out),
        PolicySet::new(),
        Preset,
        options(ReplayPolicy {
            max_source_passes: Some(3),
            forbid_memory_staging: true,
            ..Default::default()
        }),
        Some(&NoPulse),
    )
    .unwrap_err();
    assert_eq!(
        find_cause::<Refusal>(&err).copied(),
        Some(Refusal::PassBudget { pass: 0 })
    );
    assert_eq!(err.error().stage, Stage::Policy);
    assert_eq!(rows.load(Relaxed), 0);
    assert!(out.is_empty());

    // At the planned maximum the source is read once per actual pass.
    let rows = Arc::new(AtomicU32::new(0));
    let mut out = Vec::new();
    let done = encode_passes(
        encoder(exact()),
        Input::source(Mem {
            px: gradient(W, H),
            rows: rows.clone(),
        }),
        Forward(&mut out),
        PolicySet::new(),
        Preset,
        options(ReplayPolicy {
            max_source_passes: Some(4),
            forbid_memory_staging: true,
            ..Default::default()
        }),
        Some(&NoPulse),
    )
    .unwrap();
    assert_eq!(done.route, ReplayRoute::Source);
    assert_eq!(done.trace.passes, 3);
    assert_eq!(rows.load(Relaxed), 3 * H);
    assert_eq!(
        out,
        native(&exact(), &gradient(W, H), W, H, LossyUnitIndex::Trailing)
    );
}

#[test]
fn a_single_band_image_withholds_its_head_until_finish() {
    let (w, h) = (96, 40);
    let cfg = ImiEncoderConfig::lossy(70.0).with_decode_speed(DecodeSpeed::Fast);
    let px = graphic(w, h);
    let (plan, pulls, out) = drive(cfg.clone(), &px, w, h, OutputAccess::Forward);
    assert_eq!(plan.timing, OutputTiming::AT_FINISH);
    assert!(
        pulls.iter().all(|p| p.pending == 0),
        "nothing before finish"
    );
    assert_eq!(out, native(&cfg, &px, w, h, LossyUnitIndex::Trailing));
}
