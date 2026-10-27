//! Executing scope witness, not a proposed public adapter. The retained native
//! rewrite plan owns/borrows the exact chunks later emitted. Driver accounting
//! here excludes native planning, the caller's input and the destination.
use crate::{fixtures, policy::*};
use plan::{PendingPlan, PlanError};
use std::{convert::Infallible, error::Error, io::Write};
use zencodec::{
    At, NoPulse,
    encode::Fidelity,
    enough::Stop,
    metadata_rewrite::{Plan as NativePlan, RewriteRequest},
};

#[path = "../../../zencodec-testkit/src/fixtures.rs"]
#[allow(dead_code)]
mod metadata_fixtures;

fn source_with_gps(progressive: bool, placement: fixtures::Placement) -> Vec<u8> {
    let mut source = fixtures::fixture(progressive, placement);
    let span = zenjpeg::container::marker::iter(&source)
        .find(|s| s.payload.starts_with(b"Exif\0\0"))
        .unwrap();
    let range = span.offset..span.offset + span.length;
    let mut payload = b"Exif\0\0".to_vec();
    payload.extend(metadata_fixtures::rich_exif_le());
    source.splice(range, fixtures::segment(0xe1, &payload));
    let exif = exif::Reader::new()
        .read_from_container(&mut std::io::Cursor::new(&source))
        .unwrap();
    assert!(exif.fields().any(|f| f.tag.context() == exif::Context::Gps));
    source
}

type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn context() -> Context {
    Context {
        source_revision: 1,
        replayable: true,
        ..Context::default()
    }
}
fn requirements() -> Requirements {
    Requirements {
        ram: None,
        disk: None,
        accounting: Accounting::Driver,
        effects: Effects::NO_GPS.union(Effects::LOSSLESS),
    }
}
fn select(
    engine: &mut Engine,
    resolver: &mut impl PolicyResolver<Error = Infallible>,
    facts: Facts,
    action: Action,
    effects: Effects,
    requirements: Requirements,
    context: Context,
) -> Result<Selected> {
    Ok(engine.resolve(
        facts,
        &[Alternative {
            action,
            // Only the fixed-state driver: no claim about native allocations.
            ram: Evidence::Upper(0),
            disk: Evidence::Upper(0),
            accounting: Accounting::Driver,
            effects,
            needs_replay: false,
            preserves_published: true,
        }],
        requirements,
        context,
        resolver,
        &NoPulse,
    )?)
}

struct DeferredRewrite<'a> {
    input: &'a [u8],
    context: Context,
    requirements: Requirements,
    pending: PendingPlan,
    native: Option<NativePlan<'a>>,
}
impl<'a> DeferredRewrite<'a> {
    fn begin(
        input: &'a [u8],
        requirements: Requirements,
        engine: &mut Engine,
        resolver: &mut impl PolicyResolver<Error = Infallible>,
    ) -> Result<Self> {
        let context = context();
        let selected = select(
            engine,
            resolver,
            Facts::Execution(ExecutionFacts {
                resident_input: input.len() as u64,
                source_length: Some(input.len() as u64),
                network: false,
            }),
            Action::Execution(Execution::Sequential),
            Effects(0),
            requirements,
            context,
        )?;
        Ok(Self {
            input,
            context,
            requirements,
            pending: PendingPlan::new(selected, context)?,
            native: None,
        })
    }

    fn inspect(
        mut self,
        engine: &mut Engine,
        resolver: &mut impl PolicyResolver<Error = Infallible>,
    ) -> Result<Self> {
        // Default native rewrite removes descriptive metadata, retains color
        // and orientation. NO_GPS is scoped to descriptive metadata, never ICC,
        // arbitrary XML semantics or anonymity of the image samples.
        let native = zenjpeg::metadata::rewrite(&RewriteRequest::new(self.input))?;
        let metadata = select(
            engine,
            resolver,
            Facts::Metadata(MetadataFacts {
                packet_bytes: self.input.len() as u64,
                parse_complete: true,
                has_location: None,
                has_rights: None,
            }),
            Action::Metadata(MetadataAction::DropDescriptive),
            Effects::NO_GPS,
            self.requirements,
            self.context,
        )?;
        self.pending.accept(metadata, self.context)?;
        let fidelity = select(
            engine,
            resolver,
            Facts::Fidelity(FidelityFacts {
                alpha: false,
                bits_per_sample: 8,
            }),
            Action::Fidelity(Fidelity::Lossless),
            // Metadata-only rewriting copies the compressed samples unchanged.
            Effects::LOSSLESS,
            self.requirements,
            self.context,
        )?;
        self.pending.accept(fidelity, self.context)?;
        self.native = Some(native);
        Ok(self)
    }

    fn write(self, current: Context, writer: &mut impl Write, stop: &dyn zencodec::Pulse) -> Result<()> {
        stop.check()
            .map_err(zencodec::metadata_rewrite::Error::Stopped)?;
        self.pending.finish(current)?;
        let native = self
            .native
            .expect("all stage selections retain their native plan");
        for chunk in native.chunks() {
            stop.check()
                .map_err(zencodec::metadata_rewrite::Error::Stopped)?;
            writer.write_all(chunk)?;
        }
        Ok(())
    }
}

#[test]
fn pending_native_rewrite_never_publishes_before_inspection_or_after_source_change() {
    let source = fixtures::fixture(true, fixtures::Placement::BeforeEoi);
    for inspected in [false, true] {
        let mut engine = Engine::new(PolicySet::default(), 8);
        let mut preset = PolicySet::default();
        let mut pending =
            DeferredRewrite::begin(&source, requirements(), &mut engine, &mut preset).unwrap();
        if inspected {
            pending = pending.inspect(&mut engine, &mut preset).unwrap();
        }
        let current = if inspected {
            Context {
                source_revision: 2,
                ..context()
            }
        } else {
            context()
        };
        let mut output = Vec::new();
        let error = pending
            .write(current, &mut output, &NoPulse)
            .unwrap_err();
        assert_eq!(
            *error.downcast_ref::<At<PlanError>>().unwrap().error(),
            if inspected {
                PlanError::Stale
            } else {
                PlanError::Missing
            }
        );
        assert!(output.is_empty());
    }
}

#[test]
fn native_descriptive_privacy_survives_report_wrap_with_one_callback() {
    for progressive in [false, true] {
        for placement in [
            fixtures::Placement::Header,
            fixtures::Placement::BeforeEoi,
            fixtures::Placement::SplitIcc,
        ] {
            let source = source_with_gps(progressive, placement);
            let mut engine = Engine::new(PolicySet::default(), 64);
            let mut domains = Vec::new();
            let mut resolver =
                |question: Question<'_>| -> std::result::Result<Decision, At<Infallible>> {
                    let choice = question.choices().unwrap().first().unwrap();
                    domains.push(choice.value.action.kind());
                    Ok(Decision::choose(choice))
                };
            let pending =
                DeferredRewrite::begin(&source, requirements(), &mut engine, &mut resolver)
                    .unwrap();
            let original_question = engine.records().next().unwrap().question;
            // Independent questions cannot implicitly discharge pending metadata.
            for _ in 0..20 {
                select(
                    &mut engine,
                    &mut resolver,
                    Facts::Fidelity(FidelityFacts {
                        alpha: false,
                        bits_per_sample: 8,
                    }),
                    Action::Fidelity(Fidelity::Lossless),
                    Effects::LOSSLESS,
                    requirements(),
                    context(),
                )
                .unwrap();
            }
            assert!(engine.records().all(|r| r.question != original_question));
            assert!(engine.dropped_records() > 0);
            assert_eq!(
                pending.pending.remaining().collect::<Vec<_>>(),
                [Kind::Metadata, Kind::Fidelity]
            );
            let ready = pending.inspect(&mut engine, &mut resolver).unwrap();
            let mut output = Vec::new();
            ready.write(context(), &mut output, &NoPulse).unwrap();
            assert_eq!(domains.first(), Some(&Kind::Execution));
            assert_eq!(
                &domains[domains.len() - 2..],
                &[Kind::Metadata, Kind::Fidelity]
            );
            assert_eq!(fixtures::scans(&source), fixtures::scans(&output));
            assert_eq!(fixtures::oracle(&source), fixtures::oracle(&output));
            assert!(
                !output
                    .windows(fixtures::PRIVATE.len())
                    .any(|v| v == fixtures::PRIVATE)
            );
            let exif = exif::Reader::new()
                .read_from_container(&mut std::io::Cursor::new(&output))
                .unwrap();
            assert!(exif.fields().all(|f| f.tag.context() != exif::Context::Gps));
            assert_eq!(
                exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY)
                    .unwrap()
                    .value
                    .get_uint(0),
                Some(6)
            );
        }
    }
}

#[test]
fn conflicting_native_metadata_requirement_refuses_before_metadata_callback_or_output() {
    let source = fixtures::fixture(true, fixtures::Placement::BeforeEoi);
    let mut engine = Engine::new(PolicySet::default(), 8);
    let mut calls = 0;
    let mut resolver = |question: Question<'_>| -> std::result::Result<Decision, At<Infallible>> {
        calls += 1;
        Ok(Decision::choose(
            question.choices().unwrap().first().unwrap(),
        ))
    };
    let requirements = Requirements {
        effects: requirements().effects.union(Effects::EXACT_METADATA),
        ..requirements()
    };
    let pending =
        DeferredRewrite::begin(&source, requirements, &mut engine, &mut resolver).unwrap();
    let error = pending.inspect(&mut engine, &mut resolver).err().unwrap();
    let failure = error
        .downcast_ref::<At<Failure<Infallible>>>()
        .unwrap()
        .error();
    assert_eq!(failure.code(), Code::NoAlternative);
    assert!(
        failure
            .details()
            .unwrap()
            .rejections(context())
            .any(|(_, why)| matches!(why, RejectedBy::Effects { .. }))
    );
    assert_eq!(
        calls, 1,
        "source planning proceeds; noncompliant metadata is never offered"
    );
}

#[test]
fn admitted_native_rewrite_still_honors_cancellation_and_preserves_partial_writer_errors() {
    use std::{
        io,
        sync::atomic::{AtomicBool, Ordering},
    };
    use zencodec::enough::StopReason;
    struct Cancel<'a>(&'a AtomicBool);
    impl Stop for Cancel<'_> {
        fn check(&self) -> std::result::Result<(), StopReason> {
            if self.0.load(Ordering::Relaxed) {
                Err(StopReason::TimedOut)
            } else {
                Ok(())
            }
        }
    }
    struct Sink<'a> {
        bytes: Vec<u8>,
        cancel: Option<&'a AtomicBool>,
        fail_after: Option<usize>,
    }
    impl Write for Sink<'_> {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let count = bytes.len().min(
                self.fail_after
                    .map_or(usize::MAX, |cap| cap - self.bytes.len()),
            );
            if count == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "scope writer failed",
                ));
            }
            self.bytes.extend_from_slice(&bytes[..count]);
            if let Some(cancel) = self.cancel {
                cancel.store(true, Ordering::Relaxed);
            }
            Ok(count)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let source = source_with_gps(true, fixtures::Placement::BeforeEoi);
    let expected = zenjpeg::metadata::rewrite(&RewriteRequest::new(&source))
        .unwrap()
        .to_vec()
        .unwrap();
    let prepare = || {
        let mut engine = Engine::new(PolicySet::default(), 8);
        let mut preset = PolicySet::default();
        DeferredRewrite::begin(&source, requirements(), &mut engine, &mut preset)
            .unwrap()
            .inspect(&mut engine, &mut preset)
            .unwrap()
    };
    for initially_cancelled in [true, false] {
        let cancelled = AtomicBool::new(initially_cancelled);
        let mut sink = Sink {
            bytes: Vec::new(),
            cancel: Some(&cancelled),
            fail_after: None,
        };
        let error = prepare()
            .write(context(), &mut sink, &Cancel(&cancelled))
            .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<zencodec::metadata_rewrite::Error>(),
            Some(zencodec::metadata_rewrite::Error::Stopped(
                StopReason::TimedOut
            ))
        ));
        assert_eq!(sink.bytes.len(), if initially_cancelled { 0 } else { 2 });
        assert_eq!(sink.bytes, expected[..sink.bytes.len()]);
    }
    let mut sink = Sink {
        bytes: Vec::new(),
        cancel: None,
        fail_after: Some(3),
    };
    let error = prepare()
        .write(context(), &mut sink, &NoPulse)
        .unwrap_err();
    let original = error.downcast_ref::<io::Error>().unwrap();
    assert_eq!(original.kind(), io::ErrorKind::BrokenPipe);
    assert_eq!(original.to_string(), "scope writer failed");
    assert_eq!(sink.bytes, expected[..3]);
}
