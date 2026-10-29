//! The public driver runs native JPEG; the wrappers observe lifetimes only.
use crate::fixtures;
use std::{
    cell::Cell,
    io::{self, Write},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use zencodec::enough::{Stop, StopReason};
use zencodec::{
    UnsupportedOperation,
    encode::{EncodeJob, EncodeOutput, Encoder, EncoderConfig, OutputTiming},
    operation::{OutputSchedule, PolicySet},
    storage::StorageBudget,
};
use zencodec_io::row_encode::{self, Description, Options, Refusal, Source, Stage};
use zenpixels::{PixelDescriptor, PixelSlice, PixelSliceMut};

struct Native<E: Encoder> {
    inner: Option<E>,
    live: Rc<Cell<bool>>,
    expected_context: Option<Arc<zenpixels::ColorContext>>,
}
impl<E: Encoder> Drop for Native<E> {
    fn drop(&mut self) {
        self.live.set(false);
    }
}
impl<E: Encoder> Encoder for Native<E> {
    type Error = E::Error;
    fn reject(op: UnsupportedOperation) -> Self::Error {
        E::reject(op)
    }
    fn encode(mut self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, Self::Error> {
        self.inner.take().unwrap().encode(pixels)
    }
    fn begin(&mut self, w: u32, h: u32, d: PixelDescriptor) -> Result<OutputTiming, Self::Error> {
        self.inner.as_mut().unwrap().begin(w, h, d)
    }
    fn push_rows(&mut self, p: PixelSlice<'_>) -> Result<(), Self::Error> {
        assert_eq!(p.color_context(), self.expected_context.as_ref());
        self.inner.as_mut().unwrap().push_rows(p)
    }
    fn pending_output(&self) -> &[u8] {
        self.inner.as_ref().unwrap().pending_output()
    }
    fn consume_output(&mut self, n: usize) {
        self.inner.as_mut().unwrap().consume_output(n)
    }
    fn finish(mut self) -> Result<EncodeOutput, Self::Error> {
        self.inner.take().unwrap().finish()
    }
}
struct Cancel(Arc<AtomicBool>);
impl Stop for Cancel {
    fn check(&self) -> Result<(), StopReason> {
        if self.0.load(Ordering::SeqCst) {
            Err(StopReason::TimedOut)
        } else {
            Ok(())
        }
    }
}
struct Rows {
    live: Rc<Cell<bool>>,
    calls: Rc<Cell<u32>>,
    random: u32,
    fault: &'static str,
    stop: Arc<AtomicBool>,
    changed: bool,
}
impl Drop for Rows {
    fn drop(&mut self) {
        self.live.set(false);
    }
}
impl Source for Rows {
    type Error = io::Error;
    fn description(&self) -> Description {
        Description::new(
            fixtures::WIDTH + u32::from(self.changed),
            fixtures::HEIGHT,
            PixelDescriptor::RGB8_SRGB,
        )
    }
    fn read_rows(
        &mut self,
        first: u32,
        mut rows: PixelSliceMut<'_>,
        _: Option<&dyn zencodec::Pulse>,
    ) -> io::Result<()> {
        assert_eq!(first, self.calls.get());
        if first >= 16 {
            match self.fault {
                "source" => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "private source detail",
                    ));
                }
                "panic" => panic!("controlled source unwind"),
                "cancel" => self.stop.store(true, Ordering::SeqCst),
                "drift" => self.changed = true,
                _ => {}
            }
        }
        for y in 0..rows.rows() {
            for b in rows.row_mut(y) {
                self.random ^= self.random << 13;
                self.random ^= self.random >> 17;
                self.random ^= self.random << 5;
                *b = self.random as u8;
            }
        }
        self.calls.set(first + rows.rows());
        Ok(())
    }
}
struct Writer<'a> {
    bytes: &'a mut Vec<u8>,
    source: Rc<Cell<bool>>,
    encoder: Rc<Cell<bool>>,
    schedule: OutputSchedule,
    fail: bool,
}
impl Write for Writer<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.schedule == OutputSchedule::FinishFirst {
            assert!(!self.source.get(), "source retained at destination wait");
            assert!(!self.encoder.get(), "encoder retained at destination wait");
        }
        if self.fail && self.bytes.len() == 13 {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "private destination detail",
            ));
        }
        let n = bytes.len().min(3).min(if self.fail {
            13 - self.bytes.len()
        } else {
            usize::MAX
        });
        self.bytes.extend_from_slice(&bytes[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
#[test]
fn public_row_driver_native_schedules_preserve_pixels_owners_and_failure_context() {
    for schedule in [OutputSchedule::FinishFirst, OutputSchedule::Live] {
        for fault in [
            "none", "source", "drift", "cancel", "writer", "budget", "limit", "panic",
        ] {
            let enc_live = Rc::new(Cell::new(true));
            let src_live = Rc::new(Cell::new(true));
            let calls = Rc::new(Cell::new(0));
            let flag = Arc::new(AtomicBool::new(false));
            let stop = Cancel(flag.clone());
            let config = fixtures::config(false, false);
            let native = Native {
                inner: Some(config.clone().job().encoder().unwrap()),
                live: enc_live.clone(),
                expected_context: None,
            };
            let source = Rows {
                live: src_live.clone(),
                calls: calls.clone(),
                random: 0x12345678,
                fault,
                stop: flag,
                changed: false,
            };
            let budget = StorageBudget::new(if fault == "budget" { 0 } else { 1 << 20 }, 1024);
            let mut bytes = Vec::new();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                row_encode::encode(
                    native,
                    source,
                    Writer {
                        bytes: &mut bytes,
                        source: src_live.clone(),
                        encoder: enc_live.clone(),
                        schedule,
                        fail: fault == "writer",
                    },
                    PolicySet::new().require_output_schedule(schedule),
                    Options::new(budget.clone(), if fault == "limit" { 0 } else { 1 << 20 }),
                    Some(&zencodec::how_far::StopOnly::borrowed(&stop)),
                )
            }));
            assert!(!enc_live.get());
            assert!(!src_live.get());
            assert_eq!(budget.reserved_bytes(), 0);
            if fault == "panic" {
                assert!(result.is_err());
                continue;
            }
            let result = result.unwrap();
            if fault == "none" {
                let completed = result.unwrap();
                assert_eq!(completed.emitted as usize, bytes.len());
                assert_eq!(completed.report.decisions().count(), 1);
                assert_eq!(calls.get(), fixtures::HEIGHT);
                let reference = config.encode(fixtures::pixels(&fixtures::raw())).unwrap();
                assert_eq!(fixtures::oracle(&bytes), fixtures::oracle(reference.data()));
            } else {
                let error = result.unwrap_err();
                let failure = error.error();
                assert_eq!(failure.emitted as usize, bytes.len());
                assert_eq!(
                    failure.report.decisions().count(),
                    usize::from(fault != "budget")
                );
                assert!(!format!("{error:?}").contains("private "));
                match fault {
                    "source" => {
                        assert_eq!(failure.stage, Stage::Read);
                        assert_eq!(
                            zencodec::find_cause::<io::Error>(&error).unwrap().kind(),
                            io::ErrorKind::UnexpectedEof
                        );
                    }
                    "drift" => assert_eq!(
                        zencodec::find_cause::<Refusal>(&error),
                        Some(&Refusal::SourceChanged)
                    ),
                    "cancel" => {
                        use zencodec::CodecErrorExt;
                        assert_eq!(
                            error.error_category(),
                            Some(zencodec::ErrorCategory::Stopped(StopReason::TimedOut))
                        );
                    }
                    "writer" => {
                        assert_eq!(failure.stage, Stage::Write);
                        assert_eq!(failure.emitted, 13);
                        assert_eq!(
                            zencodec::find_cause::<io::Error>(&error).unwrap().kind(),
                            io::ErrorKind::BrokenPipe
                        );
                    }
                    "budget" => assert!(
                        zencodec::find_cause::<zencodec::storage::StorageError>(&error).is_some()
                    ),
                    "limit" => assert_eq!(
                        zencodec::find_cause::<Refusal>(&error),
                        Some(&Refusal::OutputLimit)
                    ),
                    _ => unreachable!(),
                }
                if schedule == OutputSchedule::FinishFirst && fault != "writer" {
                    assert!(bytes.is_empty());
                }
            }
        }
    }
}

#[test]
#[allow(deprecated)] // Preserve the legacy-placement regression during migration.
fn public_row_driver_raster_adapter_preserves_strides_and_never_substitutes_schedule() {
    use row_encode::Raster;
    use zencodec::operation::{Decision, OutputSource, Question};
    let raw = fixtures::raw();
    for progressive in [false, true] {
        for optimized in [false, true] {
            for schedule in [OutputSchedule::FinishFirst, OutputSchedule::Live] {
                let config = fixtures::config(progressive, optimized);
                let mut bytes = Vec::new();
                let mut asked = 0;
                let policy = PolicySet::new().require_output_schedule(schedule);
                let result = row_encode::encode(
                    config.clone().job().encoder().unwrap(),
                    Raster::new(fixtures::pixels(&raw)),
                    &mut bytes,
                    |q: Question<'_>| -> Result<Decision, zencodec::At<std::convert::Infallible>> {
                        asked += 1;
                        let Question::OutputSchedule { facts, .. } = q else {
                            panic!()
                        };
                        assert_eq!(facts.source, OutputSource::BorrowedRaster);
                        Ok(Decision::UsePreset)
                    },
                    Options::new(StorageBudget::new(1 << 20, 1024), 1 << 20)
                        .with_policy_set(policy),
                    Some(&zencodec::how_far::NoPulse),
                );
                if schedule == OutputSchedule::Live && (progressive || optimized) {
                    assert_eq!(asked, 0, "mandatory admission precedes callback");
                    assert!(result.is_err());
                    assert!(bytes.is_empty());
                } else {
                    assert_eq!(asked, 1);
                    result.unwrap();
                    let reference = config.encode(fixtures::pixels(&raw)).unwrap();
                    assert_eq!(fixtures::oracle(&bytes), fixtures::oracle(reference.data()));
                }
            }
        }
    }
}

#[test]
fn public_row_driver_preserves_attached_color_context_on_every_native_push() {
    let raw = fixtures::raw();
    let context = Arc::new(zenpixels::ColorContext::from_cicp(zencodec::Cicp::SRGB));
    let pixels = fixtures::pixels(&raw).with_color_context(context.clone());
    let config = fixtures::config(false, false);
    let live = Rc::new(Cell::new(true));
    let native = Native {
        inner: Some(config.job().encoder().unwrap()),
        live: live.clone(),
        expected_context: Some(context),
    };
    let mut bytes = Vec::new();
    row_encode::encode(
        native,
        row_encode::Raster::new(pixels),
        &mut bytes,
        PolicySet::new(),
        Options::new(StorageBudget::new(1 << 20, 1024), 1 << 20),
        Some(&zencodec::how_far::NoPulse),
    )
    .unwrap();
    assert!(!live.get());
    assert!(!fixtures::oracle(&bytes).is_empty());
}

#[test]
fn public_row_driver_staging_admission_failure_releases_owners_before_any_output() {
    let enc_live = Rc::new(Cell::new(true));
    let src_live = Rc::new(Cell::new(true));
    let calls = Rc::new(Cell::new(0));
    let flag = Arc::new(AtomicBool::new(false));
    let native = Native {
        inner: Some(fixtures::config(false, false).job().encoder().unwrap()),
        live: enc_live.clone(),
        expected_context: None,
    };
    let source = Rows {
        live: src_live.clone(),
        calls: calls.clone(),
        random: 0x12345678,
        fault: "none",
        stop: flag.clone(),
        changed: false,
    };
    // Permit the row batch and header, but not the growing compressed sequence.
    let budget = StorageBudget::new(fixtures::WIDTH as usize * 3 * 16 + 1024, 1024);
    let mut bytes = Vec::new();
    let error = row_encode::encode(
        native,
        source,
        &mut bytes,
        PolicySet::new(),
        Options::new(budget.clone(), 1 << 20),
        Some(&zencodec::how_far::StopOnly::borrowed(&Cancel(flag))),
    )
    .unwrap_err();
    assert!(
        calls.get() > 0,
        "this must exercise compressed staging, not initial row admission"
    );
    assert!(zencodec::find_cause::<zencodec::storage::StorageError>(&error).is_some());
    assert_eq!(error.error().report.decisions().count(), 1);
    assert!(bytes.is_empty());
    assert_eq!(budget.reserved_bytes(), 0);
    assert!(!enc_live.get());
    assert!(!src_live.get());
}

#[test]
#[allow(deprecated)] // Preserve the legacy-placement regression during migration.
fn public_row_driver_revalidates_source_across_policy_and_writer_callbacks() {
    use zencodec::operation::{Decision, OutputSource, Question};
    struct Changed {
        after_policy: Rc<Cell<bool>>,
        mode: &'static str,
        reads: Rc<Cell<u32>>,
    }
    impl Source for Changed {
        type Error = io::Error;
        fn description(&self) -> Description {
            let changed = self.after_policy.get();
            let mut desc = Description::new(
                fixtures::WIDTH + u32::from(changed && self.mode == "width"),
                fixtures::HEIGHT,
                PixelDescriptor::RGB8_SRGB,
            );
            if changed && self.mode == "context" {
                desc = desc.with_color_context(Arc::new(zenpixels::ColorContext::from_cicp(
                    zencodec::Cicp::SRGB,
                )));
            }
            desc
        }
        fn mechanism(&self) -> OutputSource {
            if self.after_policy.get() && self.mode == "mechanism" {
                OutputSource::Unknown
            } else {
                OutputSource::PullRows
            }
        }
        fn read_rows(
            &mut self,
            _: u32,
            _: PixelSliceMut<'_>,
            _: Option<&dyn zencodec::Pulse>,
        ) -> io::Result<()> {
            self.reads.set(self.reads.get() + 1);
            Err(io::Error::other(
                "source must not run after preparation changed",
            ))
        }
    }
    struct MutatingWriter<'a> {
        bytes: &'a mut Vec<u8>,
        changed: Rc<Cell<bool>>,
        mutate: bool,
    }
    impl Write for MutatingWriter<'_> {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.mutate {
                self.changed.set(true);
            }
            let n = bytes.len().min(3);
            self.bytes.extend_from_slice(&bytes[..n]);
            Ok(n)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    for (mode, phase) in ["width", "context", "mechanism"]
        .into_iter()
        .flat_map(|mode| ["policy", "writer"].map(|phase| (mode, phase)))
    {
        let changed = Rc::new(Cell::new(false));
        let reads = Rc::new(Cell::new(0));
        let live = Rc::new(Cell::new(true));
        let mut bytes = Vec::new();
        let native = Native {
            inner: Some(fixtures::config(false, false).job().encoder().unwrap()),
            live: live.clone(),
            expected_context: None,
        };
        let error = row_encode::encode(
            native,
            Changed {
                after_policy: changed.clone(),
                mode,
                reads: reads.clone(),
            },
            MutatingWriter {
                bytes: &mut bytes,
                changed: changed.clone(),
                mutate: phase == "writer",
            },
            |_: Question<'_>| -> Result<Decision, zencodec::At<std::convert::Infallible>> {
                if phase == "policy" {
                    changed.set(true);
                }
                Ok(Decision::UsePreset)
            },
            Options::new(StorageBudget::new(1 << 20, 1024), 1 << 20)
                .with_policy_set(PolicySet::new().require_output_schedule(OutputSchedule::Live)),
            Some(&zencodec::how_far::NoPulse),
        )
        .unwrap_err();
        if phase == "policy" {
            assert!(
                bytes.is_empty(),
                "changed {mode} escaped into header publication"
            );
        } else {
            assert!(!bytes.is_empty(), "mutation must follow actual publication");
        }
        assert_eq!(reads.get(), 0);
        assert_eq!(error.error().emitted as usize, bytes.len());
        assert_eq!(error.error().stage, Stage::Read);
        assert_eq!(error.error().report.decisions().count(), 1);
        assert_eq!(
            zencodec::find_cause::<Refusal>(&error),
            Some(&Refusal::SourceChanged)
        );
        assert!(!live.get());
    }
}

#[test]
fn public_row_driver_disk_staging_preserves_pixels_prefix_policy_and_failure_effects() {
    use std::{
        fs::OpenOptions,
        io::{Seek, SeekFrom},
        num::NonZeroUsize,
    };
    use zencodec::operation::{Decision, Question, StagingKind};
    let root = std::path::PathBuf::from(
        std::env::var_os("JPEG_POLICY_ARTIFACTS").expect("set JPEG_POLICY_ARTIFACTS"),
    );
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = root.join(format!("row-disk-{}-{nonce}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    for progressive in [false, true] {
        for optimized in [false, true] {
            for fault in [
                "none", "readonly", "replay", "limit", "writer", "policy", "source", "cancel",
            ] {
                let path = root.join(format!("{progressive}-{optimized}-{fault}.spool"));
                let mut file = OpenOptions::new()
                    .create_new(true)
                    .read(true)
                    .write(true)
                    .open(&path)
                    .unwrap();
                file.write_all(b"prefix").unwrap();
                if fault == "readonly" {
                    drop(file);
                    file = OpenOptions::new().read(true).open(&path).unwrap();
                    file.seek(SeekFrom::End(0)).unwrap();
                }
                let budget = StorageBudget::new(5000, 1024);
                let options =
                    Options::new(budget.clone(), if fault == "limit" { 1 } else { 1 << 20 })
                        .with_disk_staging(
                            file,
                            NonZeroUsize::new(if fault == "replay" { 100_000 } else { 7 }).unwrap(),
                        )
                        .unwrap();
                let enc_live = Rc::new(Cell::new(true));
                let src_live = Rc::new(Cell::new(true));
                let calls = Rc::new(Cell::new(0));
                let flag = Arc::new(AtomicBool::new(false));
                let stop = Cancel(flag.clone());
                let config = fixtures::config(progressive, optimized);
                let mut bytes = Vec::new();
                let result = row_encode::encode(
                    Native {
                        inner: Some(config.clone().job().encoder().unwrap()),
                        live: enc_live.clone(),
                        expected_context: None,
                    },
                    Rows {
                        live: src_live.clone(),
                        calls: calls.clone(),
                        random: 0x12345678,
                        fault,
                        stop: flag,
                        changed: false,
                    },
                    Writer {
                        bytes: &mut bytes,
                        source: src_live.clone(),
                        encoder: enc_live.clone(),
                        schedule: OutputSchedule::FinishFirst,
                        fail: fault == "writer",
                    },
                    |q: Question<'_>| -> Result<Decision, zencodec::At<io::Error>> {
                        let Question::OutputSchedule { choices, .. } = q else {
                            panic!("unexpected question")
                        };
                        let offer = choices.first().unwrap();
                        assert_eq!(offer.value().staging.unwrap().kind, StagingKind::Disk);
                        assert_eq!(offer.value().staging.unwrap().payload_bytes, 0);
                        if fault == "policy" {
                            return Err(zencodec::At::wrap(io::Error::other(
                                "private policy detail",
                            )));
                        }
                        Ok(Decision::choose(offer))
                    },
                    options,
                    Some(&zencodec::how_far::StopOnly::borrowed(&stop)),
                );
                assert!(!enc_live.get());
                assert!(!src_live.get());
                assert_eq!(budget.reserved_bytes(), 0);
                let spool = std::fs::read(path).unwrap();
                assert_eq!(&spool[..6], b"prefix");
                if fault == "none" {
                    let completed = result.unwrap();
                    assert_eq!(completed.emitted as usize, bytes.len());
                    assert_eq!(&spool[6..], bytes);
                    let reference = config.encode(fixtures::pixels(&fixtures::raw())).unwrap();
                    assert_eq!(bytes, reference.data());
                    assert_eq!(fixtures::oracle(&bytes), fixtures::oracle(reference.data()));
                } else {
                    let error = result.unwrap_err();
                    assert_eq!(error.error().emitted as usize, bytes.len());
                    assert_eq!(error.error().report.decisions().count(), 1);
                    if fault == "writer" {
                        assert_eq!(bytes.len(), 13);
                        assert_eq!(error.error().stage, Stage::Write);
                        assert_eq!(
                            zencodec::find_cause::<io::Error>(&error).unwrap().kind(),
                            io::ErrorKind::BrokenPipe
                        );
                    } else {
                        assert!(bytes.is_empty());
                        if fault == "policy" {
                            assert_eq!(error.error().stage, Stage::Policy);
                            assert_eq!(calls.get(), 0);
                            assert_eq!(spool.len(), 6);
                        }
                        if fault == "source" {
                            assert_eq!(error.error().stage, Stage::Read);
                        }
                        if fault == "replay" {
                            assert_eq!(error.error().stage, Stage::Replay);
                        }
                        if fault == "readonly" {
                            assert!(zencodec::find_cause::<io::Error>(&error).is_some());
                        }
                        if fault == "limit" {
                            assert_eq!(
                                zencodec::find_cause::<Refusal>(&error),
                                Some(&Refusal::OutputLimit)
                            );
                        }
                    }
                }
            }
        }
    }
}
