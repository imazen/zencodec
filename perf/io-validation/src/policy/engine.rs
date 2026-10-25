use super::{alloc::boxed::Box, resolver::*, types::*};
use core::{
    error::Error,
    fmt,
    sync::atomic::{AtomicUsize, Ordering},
};
use zencodec::{
    At,
    enough::{Stop, StopReason},
};

static NEXT_QUESTION: AtomicUsize = AtomicUsize::new(1);
pub(crate) const MAX_CHOICES: usize = 64;
pub(crate) const JOURNAL_CAPACITY: usize = 16;

/// Checked exhaustion never wraps a ticket back onto a stale choice.
pub(crate) fn issue_ticket(counter: &AtomicUsize) -> Option<u64> {
    let mut observed = counter.load(Ordering::Relaxed);
    loop {
        let next = observed.checked_add(1)?;
        match counter.compare_exchange_weak(observed, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return u64::try_from(observed).ok(),
            Err(current) => observed = current,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Code {
    Selected,
    Declined,
    Callback,
    InvalidChoice,
    NoAlternative,
    InvalidOffer,
    InvalidRequirements,
    TooManyChoices,
    PlanningLimit,
    IdentityExhausted,
    Stopped,
    InterruptedCallback,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Route {
    Resolver,
    Preset,
}

/// Bounded, data-free diagnostic record: no packet contents, URLs, ICC names,
/// input filenames or callback Display text are copied into it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Record {
    pub operation: u64,
    pub question: u64,
    pub ordinal: u32,
    pub kind: Kind,
    pub context: Context,
    pub admitted: u64,
    pub rejected_reasons: u16,
    pub chosen: Option<u8>,
    pub route: Route,
    pub code: Code,
}

pub(crate) enum Cause<E> {
    Framework,
    Declined(Decline),
    Callback(E),
    Stopped(StopReason),
}

/// Boxing is on failure only. The trace stays in the single outer At wrapper.
pub(crate) struct Failure<E>(Box<FailureData<E>>);
struct FailureData<E> {
    record: Record,
    cause: Cause<E>,
}
impl<E> Failure<E> {
    fn new(record: Record, cause: Cause<E>) -> Self {
        Self(Box::new(FailureData { record, cause }))
    }
    pub fn record(&self) -> Record {
        self.0.record
    }
    pub fn code(&self) -> Code {
        self.0.record.code
    }
    pub fn cause(&self) -> &Cause<E> {
        &self.0.cause
    }
}
impl<E> fmt::Debug for Failure<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PolicyFailure")
            .field("record", &self.0.record)
            .finish_non_exhaustive()
    }
}
impl<E> fmt::Display for Failure<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "policy {:?} at {:?} decision {} (source revision {}, epoch {})",
            self.code(),
            self.0.record.kind,
            self.0.record.ordinal,
            self.0.record.context.source_revision,
            self.0.record.context.epoch
        )
    }
}
impl<E: Error + 'static> Error for Failure<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &self.0.cause {
            Cause::Framework => None,
            Cause::Declined(e) => Some(e),
            Cause::Callback(e) => Some(e),
            // StopReason is typed data, not an Error in the pinned enough API.
            Cause::Stopped(_) => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Ready,
    Calling,
}

pub(crate) struct Engine {
    operation: Option<u64>,
    preset: PolicySet,
    limit: u32,
    count: u32,
    state: State,
    records: [Option<Record>; JOURNAL_CAPACITY],
    next_record: usize,
    record_count: usize,
    dropped: u64,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Selected {
    pub operation: u64,
    pub requirements: Requirements,
    pub alternative: Alternative,
    pub index: usize,
    pub context: Context,
    pub question: u64,
}

impl Engine {
    pub fn new(preset: PolicySet, max_decisions: u32) -> Self {
        Self {
            operation: issue_ticket(&NEXT_QUESTION),
            preset,
            limit: max_decisions,
            count: 0,
            state: State::Ready,
            records: [None; JOURNAL_CAPACITY],
            next_record: 0,
            record_count: 0,
            dropped: 0,
        }
    }
    pub fn records(&self) -> impl Iterator<Item = Record> + '_ {
        let start = if self.record_count == JOURNAL_CAPACITY {
            self.next_record
        } else {
            0
        };
        (0..self.record_count).filter_map(move |n| self.records[(start + n) % JOURNAL_CAPACITY])
    }
    pub fn dropped_records(&self) -> u64 {
        self.dropped
    }
    fn record(&mut self, record: Record) {
        self.records[self.next_record] = Some(record);
        self.next_record = (self.next_record + 1) % JOURNAL_CAPACITY;
        if self.record_count < JOURNAL_CAPACITY {
            self.record_count += 1;
        } else {
            self.dropped = self.dropped.saturating_add(1);
        }
    }
    #[track_caller]
    fn fail<E>(&mut self, mut record: Record, code: Code, cause: Cause<E>) -> At<Failure<E>> {
        record.code = code;
        self.record(record);
        At::wrap(Failure::new(record, cause)).at()
    }

    /// No locks or storage reservations are held across application code.
    /// A panic is not caught: it leaves Calling set so reuse is refused.
    #[track_caller]
    pub fn resolve<R: PolicyResolver + ?Sized>(
        &mut self,
        facts: Facts,
        entries: &[Alternative],
        requirements: Requirements,
        context: Context,
        resolver: &mut R,
        stop: &dyn Stop,
    ) -> Result<Selected, At<Failure<R::Error>>> {
        let mut record = Record {
            operation: self.operation.unwrap_or(0),
            question: 0,
            ordinal: self.count,
            kind: facts.kind(),
            context,
            admitted: 0,
            rejected_reasons: 0,
            chosen: None,
            route: Route::Resolver,
            code: Code::Selected,
        };
        if self.state != State::Ready {
            return Err(self.fail(record, Code::InterruptedCallback, Cause::Framework));
        }
        if self.operation.is_none() {
            return Err(self.fail(record, Code::IdentityExhausted, Cause::Framework));
        }
        if self.count >= self.limit {
            return Err(self.fail(record, Code::PlanningLimit, Cause::Framework));
        }
        self.count += 1;
        if let Err(reason) = stop.check() {
            return Err(self.fail(record, Code::Stopped, Cause::Stopped(reason)));
        }
        if requirements.effects.0 & !31 != 0 {
            return Err(self.fail(record, Code::InvalidRequirements, Cause::Framework));
        }
        if entries.len() > MAX_CHOICES {
            return Err(self.fail(record, Code::TooManyChoices, Cause::Framework));
        }
        for (index, entry) in entries.iter().enumerate() {
            if entry.action.kind() != facts.kind() {
                return Err(self.fail(record, Code::InvalidOffer, Cause::Framework));
            }
            if let Action::Fidelity(target) = entry.action {
                use zencodec::encode::{Fidelity, LossyTarget};
                let finite = if target == Fidelity::Lossless {
                    true
                } else if let Fidelity::Lossy(
                    LossyTarget::ApproxSsim2(v)
                    | LossyTarget::ApproxButteraugli(v)
                    | LossyTarget::ApproxZensimB(v)
                    | LossyTarget::CodecSpecificQuality(v),
                ) = target
                {
                    v.is_finite()
                } else {
                    false
                };
                if !finite
                    || entry.effects.contains(Effects::LOSSLESS) != (target == Fidelity::Lossless)
                {
                    return Err(self.fail(record, Code::InvalidOffer, Cause::Framework));
                }
            }
            if let Facts::Execution(f) = facts
                && entry.accounting == Accounting::Operation
                && entry.ram.bound().is_some_and(|n| n < f.resident_input)
            {
                return Err(self.fail(record, Code::InvalidOffer, Cause::Framework));
            }
            let mut rejected = 0;
            if !entry.ram.fits(requirements.ram) {
                rejected |= 1;
            }
            if !entry.disk.fits(requirements.disk) {
                rejected |= 2;
            }
            if requirements.ram.is_some()
                && requirements.accounting == Accounting::Operation
                && entry.accounting != Accounting::Operation
            {
                rejected |= 4;
            }
            if !entry
                .effects
                .contains(requirements.effects.domain(facts.kind()))
            {
                rejected |= 8;
            }
            if entry.needs_replay && (!context.replayable || context.published_bytes != 0) {
                rejected |= 16;
            }
            if context.published_bytes != 0 && !entry.preserves_published {
                rejected |= 32;
            }
            record.rejected_reasons |= rejected;
            if rejected == 0 {
                record.admitted |= 1u64 << index;
            }
        }
        if record.admitted == 0 && !matches!(facts, Facts::Unknown(_)) {
            return Err(self.fail(record, Code::NoAlternative, Cause::Framework));
        }
        let Some(ticket) = issue_ticket(&NEXT_QUESTION) else {
            return Err(self.fail(record, Code::IdentityExhausted, Cause::Framework));
        };
        record.question = ticket;
        let choices = Choices::new(ticket, record.admitted, entries);
        let question = Question::new(facts, choices);
        self.state = State::Calling;
        let answer = resolver.resolve(question);
        self.state = State::Ready;
        let mut answer = match answer {
            Ok(answer) => answer,
            Err(error) => {
                record.code = Code::Callback;
                self.record(record);
                return Err(error
                    .map_error(|e| Failure::new(record, Cause::Callback(e)))
                    .at_named("policy callback returned an error"));
            }
        };
        if let Err(reason) = stop.check() {
            return Err(self.fail(record, Code::Stopped, Cause::Stopped(reason)));
        }
        if matches!(answer, Decision::UsePreset) {
            record.route = Route::Preset;
            answer = self
                .preset
                .resolve(question)
                .unwrap_or_else(|e| match *e.error() {});
        }
        match answer {
            Decision::Choose(id) => {
                let Some((index, alternative)) = choices.validate(id) else {
                    return Err(self.fail(record, Code::InvalidChoice, Cause::Framework));
                };
                record.chosen = Some(index as u8);
                self.record(record);
                Ok(Selected {
                    operation: record.operation,
                    requirements,
                    alternative: *alternative,
                    index,
                    context,
                    question: ticket,
                })
            }
            Decision::Reject(error) => {
                record.code = Code::Declined;
                self.record(record);
                Err(error
                    .map_error(|e| Failure::new(record, Cause::Declined(e)))
                    .at_named("policy declined this operation"))
            }
            Decision::UsePreset => unreachable!("the built-in preset never delegates"),
        }
    }
}
