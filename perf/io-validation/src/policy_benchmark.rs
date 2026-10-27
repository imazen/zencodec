//! Common workload for wall-clock and allocation profiling. All successful
//! arms run the same engine validation, choose index 1, and retain diagnostics.
use crate::policy::*;
use std::{convert::Infallible, error::Error, fmt, hint::black_box};
use zencodec::{At, NoPulse};

#[derive(Clone, Copy, Debug)]
pub(crate) enum Mode {
    Direct,
    Closure,
    Dyn,
    ErasedSuccess,
    Decline,
    CallbackError,
    NoAlternative,
}
impl Mode {
    pub const ALL: [(Self, &'static str); 7] = [
        (Self::Direct, "direct-preset"),
        (Self::Closure, "closure-fallback"),
        (Self::Dyn, "dyn-preset"),
        (Self::ErasedSuccess, "erased-success"),
        (Self::Decline, "decline"),
        (Self::CallbackError, "callback-error"),
        (Self::NoAlternative, "no-alternative"),
    ];
}
#[derive(Debug)]
struct AppError(u64);
impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "application rule {} unavailable", self.0)
    }
}
impl Error for AppError {}
fn fail(_: Question<'_>) -> Result<Decision, At<AppError>> {
    Err(At::wrap(AppError(7)).at_named("application rule"))
}
fn delegate(_: Question<'_>) -> Result<Decision, At<Infallible>> {
    Ok(Decision::UsePreset)
}
fn decline(_: Question<'_>) -> Result<Decision, At<Infallible>> {
    Ok(Decision::reject(Decline::ApplicationRule))
}

#[inline(never)]
fn run<R: PolicyResolver + ?Sized>(
    engine: &mut Engine,
    offers: &[Alternative],
    requirements: Requirements,
    resolver: &mut R,
) -> usize {
    match engine.resolve(
        Facts::Execution(ExecutionFacts {
            resident_input: 0,
            source_length: Some(4096),
            network: false,
        }),
        black_box(offers),
        requirements,
        Context::default(),
        resolver,
        &NoPulse,
    ) {
        Ok(selected) => {
            assert_eq!(selected.index, 1);
            black_box(selected.index)
        }
        Err(error) => {
            black_box(error.error().details());
            black_box(error.frame_count());
            3
        }
    }
}
/// One independent operation per iteration, including identity issuance and
/// bounded journal initialization. Dyn resolvers are borrowed stack objects.
#[inline(never)]
pub(crate) fn operation(mode: Mode) -> usize {
    let preset = PolicySet {
        execution: ExecutionPreference::SmallestBound,
        ..PolicySet::default()
    };
    let mut engine = Engine::new(preset, 1);
    let requirements = Requirements {
        ram: Some(64),
        disk: Some(0),
        accounting: Accounting::Operation,
        effects: Effects(0),
    };
    let offers = [
        (Execution::Buffered, 64),
        (Execution::Sequential, 16),
        (Execution::DiskStaged, 32),
    ]
    .map(|(strategy, n)| Alternative {
        action: Action::Execution(strategy),
        ram: Evidence::Upper(n),
        disk: Evidence::Upper(0),
        accounting: Accounting::Operation,
        effects: Effects(0),
        needs_replay: false,
        preserves_published: true,
    });
    let value = match mode {
        Mode::Direct => run(&mut engine, &offers, requirements, &mut preset.clone()),
        Mode::Closure => run(&mut engine, &offers, requirements, &mut delegate),
        Mode::Dyn => {
            let mut preset = preset;
            let resolver: &mut dyn PolicyResolver<Error = Infallible> = black_box(&mut preset);
            run(&mut engine, &offers, requirements, resolver)
        }
        Mode::ErasedSuccess => {
            let mut erased = Erased(delegate);
            let resolver: &mut dyn PolicyResolver<Error = ErasedError> = black_box(&mut erased);
            run(&mut engine, &offers, requirements, resolver)
        }
        Mode::Decline => run(&mut engine, &offers, requirements, &mut decline),
        Mode::CallbackError => {
            let mut erased = Erased(fail);
            let resolver: &mut dyn PolicyResolver<Error = ErasedError> = black_box(&mut erased);
            run(&mut engine, &offers, requirements, resolver)
        }
        Mode::NoAlternative => run(
            &mut engine,
            &offers,
            Requirements {
                ram: Some(0),
                ..requirements
            },
            &mut delegate,
        ),
    };
    black_box(engine.records().count());
    assert_eq!(
        value,
        match mode {
            Mode::Direct | Mode::Closure | Mode::Dyn | Mode::ErasedSuccess => 1,
            _ => 3,
        }
    );
    value
}
