//! Application decisions over immutable, prepared alternatives.

use crate::{At, ImageFormat, encode::Fidelity};
use alloc::boxed::Box;
use core::{
    convert::Infallible,
    error::Error,
    fmt,
    sync::atomic::{AtomicUsize, Ordering},
};
use enough::{Stop, StopReason};
use zenpixels::PixelDescriptor;

/// A token for one offer at one decision. Tokens cannot authorize a different
/// operation or a later question, even if its alternatives look identical.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChoiceId {
    ticket: u64,
    index: u8,
}

/// One immutable offer and its selection token.
#[derive(Debug)]
pub struct Choice<'a, T> {
    id: ChoiceId,
    value: &'a T,
}
impl<T> Copy for Choice<'_, T> {}
impl<T> Clone for Choice<'_, T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<'a, T> Choice<'a, T> {
    /// Facts and effects of this offered implementation.
    pub const fn value(self) -> &'a T {
        self.value
    }
    /// Opaque token, valid only for the question that supplied this choice.
    pub const fn id(self) -> ChoiceId {
        self.id
    }
}

/// A borrowed set of prepared alternatives of one domain-specific type.
#[derive(Debug)]
pub struct Choices<'a, T> {
    ticket: u64,
    entries: &'a [T],
}
impl<T> Copy for Choices<'_, T> {}
impl<T> Clone for Choices<'_, T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<'a, T> Choices<'a, T> {
    /// The number of offered alternatives.
    pub const fn len(self) -> usize {
        self.entries.len()
    }
    /// Whether no implementation was offered.
    pub const fn is_empty(self) -> bool {
        self.entries.is_empty()
    }
    /// Choices in request preference order.
    pub fn iter(self) -> impl ExactSizeIterator<Item = Choice<'a, T>> {
        self.entries
            .iter()
            .enumerate()
            .map(move |(index, value)| Choice {
                id: ChoiceId {
                    ticket: self.ticket,
                    index: index as u8,
                },
                value,
            })
    }
    /// First prepared choice, if any.
    pub fn first(self) -> Option<Choice<'a, T>> {
        self.iter().next()
    }
    fn selected(self, id: ChoiceId) -> Option<usize> {
        let index = usize::from(id.index);
        (id.ticket == self.ticket && index < self.entries.len()).then_some(index)
    }
}

/// A prepared still-image encoder job. The selected job is retained unchanged
/// across the callback; the adapter does not reconfigure a fresh job afterward.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct EncodeOffer {
    /// Destination format of this route.
    pub format: ImageFormat,
    /// Confirmed fidelity target, or `None` for the registered template's mode.
    pub fidelity: Option<Fidelity>,
    /// Index in the request's original preference list, before filtering.
    pub preference_index: usize,
}

/// The qualified contract chosen at a policy boundary. Contains facts, never
/// input bytes, paths, metadata payloads or application error text.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum Selection {
    /// The selected prepared encoder configuration.
    Encode(EncodeOffer),
}

/// Facts about the actual pixels awaiting encoding.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct EncodeFacts {
    /// Image width.
    pub width: u32,
    /// Image height.
    pub height: u32,
    /// Current sample interpretation and layout; no implicit conversion is offered.
    pub descriptor: PixelDescriptor,
}

/// One situational decision. Each variant carries only its own typed offers.
///
/// The current production path covers prepared still-image encoding. New
/// domains are added with their executing adapters, not as unchecked actions.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum Question<'a> {
    /// Select a prepared encoder configuration for these pixels.
    Encode {
        /// Input facts; no pixel bytes or metadata payloads are exposed here.
        facts: EncodeFacts,
        /// Jobs that passed configuration validation and the request's policy.
        choices: Choices<'a, EncodeOffer>,
    },
}

/// Predefined expected policy refusals. Application failures use `Err(At<E>)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Decline {
    /// An application rule forbids all offered choices.
    ApplicationRule,
    /// No offered implementation matches the policy's preference.
    NoPreferredChoice,
    /// The policy does not understand this question domain.
    UnsupportedQuestion,
}
impl fmt::Display for Decline {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "policy declined: {self:?}")
    }
}
impl Error for Decline {}

/// A policy may select an offered token, delegate once to the configured
/// built-in policy, or refuse. There is no ignore-error/resume-anyway response.
#[derive(Debug)]
#[non_exhaustive]
pub enum Decision {
    /// Select a token from the current question.
    Choose(ChoiceId),
    /// Let the operation's configured `PolicySet` answer this question once.
    UsePreset,
    /// Expected policy refusal with a location trace.
    Decline(At<Decline>),
}
impl Decision {
    /// Select this offered choice.
    pub fn choose<T>(choice: Choice<'_, T>) -> Self {
        Self::Choose(choice.id())
    }
    /// Refuse with a predefined reason and capture the caller's location.
    #[track_caller]
    pub fn decline(reason: Decline) -> Self {
        Self::Decline(At::wrap(reason).at())
    }
}

/// One callback interface for application policy. A closure can capture tenant,
/// latency or resource policy without putting application types in codec traits.
pub trait PolicyResolver {
    /// Application-specific callback failure, preserved with its original trace.
    type Error: Error + Send + Sync + 'static;
    /// Select among the supplied alternatives or return a traced application error.
    fn resolve(&mut self, question: Question<'_>) -> Result<Decision, At<Self::Error>>;
}
impl<F, E> PolicyResolver for F
where
    F: FnMut(Question<'_>) -> Result<Decision, At<E>>,
    E: Error + Send + Sync + 'static,
{
    type Error = E;
    fn resolve(&mut self, question: Question<'_>) -> Result<Decision, At<E>> {
        self(question)
    }
}

/// Configurable built-in policy; applications can use it directly or delegate
/// selected questions to it with `Decision::UsePreset`.
#[derive(Clone, Copy, Debug, Default)]
pub struct PolicySet {
    prefer_lossless: bool,
}
impl PolicySet {
    /// Choose the first prepared alternative in request order.
    pub const fn new() -> Self {
        Self {
            prefer_lossless: false,
        }
    }
    /// Prefer a confirmed lossless mode when offered; otherwise preserve the
    /// request's first available choice. This never weakens a required target.
    pub const fn prefer_lossless(mut self, enabled: bool) -> Self {
        self.prefer_lossless = enabled;
        self
    }
}
impl PolicyResolver for PolicySet {
    type Error = Infallible;
    fn resolve(&mut self, question: Question<'_>) -> Result<Decision, At<Infallible>> {
        let Question::Encode { choices, .. } = question;
        let choice = if self.prefer_lossless {
            choices
                .iter()
                .find(|choice| choice.value().fidelity == Some(Fidelity::Lossless))
                .or_else(|| choices.first())
        } else {
            choices.first()
        };
        Ok(choice
            .map(Decision::choose)
            .unwrap_or_else(|| Decision::decline(Decline::NoPreferredChoice)))
    }
}

/// Data-free outcome code for a policy decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DecisionCode {
    /// An offered implementation was selected.
    Selected,
    /// The application intentionally declined.
    Declined,
    /// Application callback returned an error.
    CallbackFailed,
    /// An answer came from another question or operation.
    InvalidChoice,
    /// Cancellation prevented selection or execution.
    Stopped,
    /// The finite question/ticket allowance was exhausted.
    Exhausted,
}

/// Numeric/enum diagnostic record, deliberately excluding payloads, paths,
/// profile names and callback error text. Native errors remain in `source()`.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct DecisionRecord {
    /// Globally unique question identity (zero only if identity allocation failed).
    pub question: u64,
    /// Number of qualified alternatives presented.
    pub offered: u8,
    /// Selected contract, including its original request preference index.
    pub selected: Option<Selection>,
    /// Whether the built-in policy resolved this decision after delegation.
    pub used_preset: bool,
    /// Selection or refusal category.
    pub code: DecisionCode,
}

/// Bounded decision report for a completed or failed operation.
#[derive(Clone, Debug, Default)]
pub struct OperationReport {
    records: [Option<DecisionRecord>; 16],
    count: usize,
    dropped: u64,
}
impl OperationReport {
    /// Records in decision order; older records are dropped when the bound fills.
    pub fn decisions(&self) -> impl Iterator<Item = &DecisionRecord> {
        self.records[..self.count].iter().filter_map(Option::as_ref)
    }
    /// Number of older records omitted from this bounded report.
    pub const fn dropped_decisions(&self) -> u64 {
        self.dropped
    }
    pub(super) fn push(&mut self, record: DecisionRecord) {
        if self.count == self.records.len() {
            self.records.rotate_left(1);
            self.count -= 1;
            self.dropped = self.dropped.saturating_add(1);
        }
        self.records[self.count] = Some(record);
        self.count += 1;
    }
}

/// A policy failure with redacted default formatting and inspectable native cause.
pub struct PolicyFailure(Box<FailureData>);
struct FailureData {
    record: DecisionRecord,
    cause: Cause,
}
enum Cause {
    Framework,
    Declined(At<Decline>),
    Callback(Box<dyn Error + Send + Sync>),
    Stopped(StopReason),
}
impl PolicyFailure {
    /// The numeric decision record that explains where this failure occurred.
    pub fn record(&self) -> DecisionRecord {
        self.0.record
    }
    /// Cancellation reason, when the policy boundary was stopped.
    pub fn stop_reason(&self) -> Option<StopReason> {
        match self.0.cause {
            Cause::Stopped(reason) => Some(reason),
            _ => None,
        }
    }
    fn new(record: DecisionRecord, cause: Cause) -> Self {
        Self(Box::new(FailureData { record, cause }))
    }
}
impl fmt::Debug for PolicyFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PolicyFailure")
            .field("record", &self.0.record)
            .finish_non_exhaustive()
    }
}
impl fmt::Display for PolicyFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "policy {:?} at question {}",
            self.0.record.code, self.0.record.question
        )
    }
}
impl Error for PolicyFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &self.0.cause {
            Cause::Declined(e) => Some(e),
            Cause::Callback(e) => Some(e.as_ref()),
            Cause::Framework | Cause::Stopped(_) => None,
        }
    }
}

static NEXT_QUESTION: AtomicUsize = AtomicUsize::new(1);
pub(super) const MAX_OFFERS: usize = 64;

fn ticket(counter: &AtomicUsize) -> Option<u64> {
    // compare_exchange keeps this path compatible with the crate's MSRV.
    let mut current = counter.load(Ordering::Relaxed);
    loop {
        let next = current.checked_add(1)?;
        match counter.compare_exchange_weak(current, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(value) => return u64::try_from(value).ok(),
            Err(actual) => current = actual,
        }
    }
}

pub(super) fn choose_encode<R: PolicyResolver + ?Sized>(
    facts: EncodeFacts,
    offers: &[EncodeOffer],
    resolver: &mut R,
    mut preset: PolicySet,
    stop: &dyn Stop,
) -> Result<(usize, DecisionRecord), At<PolicyFailure>> {
    let mut record = DecisionRecord {
        question: 0,
        offered: offers.len().min(255) as u8,
        selected: None,
        used_preset: false,
        code: DecisionCode::Exhausted,
    };
    if offers.is_empty() || offers.len() > MAX_OFFERS {
        return Err(At::wrap(PolicyFailure::new(record, Cause::Framework)).at());
    }
    record.question = ticket(&NEXT_QUESTION)
        .ok_or_else(|| At::wrap(PolicyFailure::new(record, Cause::Framework)).at())?;
    if let Err(reason) = stop.check() {
        record.code = DecisionCode::Stopped;
        return Err(At::wrap(PolicyFailure::new(record, Cause::Stopped(reason))).at());
    }
    let choices = Choices {
        ticket: record.question,
        entries: offers,
    };
    let question = Question::Encode { facts, choices };
    // The resolver acquires no resource-budget locks. Prepared jobs remain
    // owned by the caller of this function while application code runs.
    let mut decision = resolver.resolve(question).map_err(|error| {
        record.code = DecisionCode::CallbackFailed;
        At::wrap(PolicyFailure::new(record, Cause::Callback(Box::new(error))))
            .at_named("application policy callback")
    })?;
    if let Err(reason) = stop.check() {
        record.code = DecisionCode::Stopped;
        return Err(At::wrap(PolicyFailure::new(record, Cause::Stopped(reason))).at());
    }
    if matches!(decision, Decision::UsePreset) {
        record.used_preset = true;
        decision = preset
            .resolve(question)
            .unwrap_or_else(|never| match *never.error() {});
    }
    match decision {
        Decision::Choose(id) => match choices.selected(id) {
            Some(index) => {
                record.selected = Some(Selection::Encode(offers[index]));
                record.code = DecisionCode::Selected;
                Ok((index, record))
            }
            None => {
                record.code = DecisionCode::InvalidChoice;
                Err(At::wrap(PolicyFailure::new(record, Cause::Framework)).at())
            }
        },
        Decision::Decline(reason) => {
            record.code = DecisionCode::Declined;
            Err(At::wrap(PolicyFailure::new(record, Cause::Declined(reason))).at())
        }
        Decision::UsePreset => unreachable!("the built-in policy never delegates"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn question_identity_never_wraps() {
        let counter = AtomicUsize::new(usize::MAX - 1);
        assert_eq!(ticket(&counter), Some((usize::MAX - 1) as u64));
        assert_eq!(ticket(&counter), None);
        assert_eq!(ticket(&counter), None);
    }

    #[test]
    fn journal_retains_recent_decisions_and_reports_omissions() {
        let mut report = OperationReport::default();
        for question in 1..=20 {
            report.push(DecisionRecord {
                question,
                offered: 1,
                selected: None,
                used_preset: false,
                code: DecisionCode::Declined,
            });
        }
        assert_eq!(report.dropped_decisions(), 4);
        assert_eq!(
            report
                .decisions()
                .map(|r| r.question)
                .collect::<alloc::vec::Vec<_>>(),
            (5..=20).collect::<alloc::vec::Vec<_>>()
        );
    }
}
