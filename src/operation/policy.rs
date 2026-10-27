//! Application decisions over immutable, prepared alternatives.

use super::{FrameFacts, ImageFacts, MetadataTransfer, PublicationOffer, TranscodeOffer};
use crate::decode::{BoundaryAction, DecodeBoundary, InterpretationFacts, InterpretationState};
use crate::frame::{FrameInterpretation, FrameTiming};
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
    /// Required physical density or unknown-unit aspect, confirmed on this job.
    /// Carrier units may differ, but the represented density must be equivalent.
    pub resolution: Option<crate::Resolution>,
}

/// The qualified effect chosen at a policy boundary. Input facts remain in
/// `DecisionContext`, including on refusal. No input bytes, paths, metadata
/// payloads or application error text are copied into either diagnostic.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum Selection {
    /// Prepared conversion of a held image before destination writes.
    ImageConversion(super::ImageConversionOffer),
    /// Prepared frame encoding and output continuity.
    FrameRoute(super::FrameRouteOffer),
    /// Publication of the exact held selected-image output.
    /// Output facts are retained once in `DecisionContext::Image`.
    Image(PublicationOffer),
    /// Publication of the original held native presentation.
    /// Interpretation and timing remain in `DecisionContext::Frame`.
    Frame(PublicationOffer),
    /// Selected conversion, metadata, encoder and storage route.
    Transcode(TranscodeOffer),
    /// The selected prepared encoder configuration.
    Encode(EncodeOffer),
    /// The selected mechanism at a mandatory interpretation suspension.
    /// Interpretation state and geometry remain in `DecisionContext::Boundary`.
    Boundary(BoundaryAction),
}

/// The facts that prompted a question, retained even when no route was selected.
/// This is diagnostic context, not authority to resume or execute anything.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum DecisionContext {
    /// Held image before executing a selected conversion route.
    ImageConversion(ImageFacts),
    /// Held frame and output state before an encoding transition.
    FrameRoute(super::FrameRouteFacts),
    /// Selected native output before publication to the application.
    Image(ImageFacts),
    /// Native presentation before it reaches conversion or application output.
    Frame(FrameFacts),
    /// Final interpretation for a coupled pixel-transcode route.
    Transcode(InterpretationFacts),
    /// Facts about the pixels awaiting an encoder configuration.
    Encode(EncodeFacts),
    /// Facts at a mandatory interpretation suspension, including any refusal reason.
    Boundary(InterpretationFacts),
}

/// A mechanism offered at a real decoder suspension. Continuing discovery
/// keeps affected output blocked; this offer makes no native memory claim.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct BoundaryOffer {
    /// Mechanism the suspended decoder will execute after this decision.
    pub action: BoundaryAction,
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
/// Domains are added with their executing adapters. Selecting an alternative
/// never authorizes the adapter to change its prepared effects afterward.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum Question<'a> {
    /// Choose a prepared conversion before its destination pixels are written.
    ImageConversion {
        /// Exact input retained by the adapter throughout this decision.
        image: &'a crate::image::ImageOutput,
        /// Source interpretation and ownership facts, without payloads.
        facts: &'a ImageFacts,
        /// Request-compatible retained conversion plans and their effects.
        choices: Choices<'a, super::ImageConversionOffer>,
    },
    /// Choose among prepared encoding routes before accepting this frame or
    /// changing the destination's output sequence.
    FrameRoute {
        /// Full immutable interpretation of the held input.
        interpretation: &'a FrameInterpretation,
        /// Input provenance and current output state, without payloads.
        facts: &'a super::FrameRouteFacts,
        /// Only request- and destination-compatible prepared routes.
        choices: Choices<'a, super::FrameRouteOffer>,
    },
    /// Decide before publishing this exact held selected output. It can be
    /// inspected or mapped; source names/profiles remain unfiltered data.
    Image {
        /// The owned output remains held throughout the callback.
        image: &'a crate::image::ImageOutput,
        /// Borrowed diagnostic facts also copied into the bounded report.
        facts: &'a ImageFacts,
        /// Prepared publication only; no transformation or privacy proof.
        choices: Choices<'a, PublicationOffer>,
    },
    /// Decide before one native frame reaches conversion or application output.
    /// The adapter retains this exact frame while the callback runs.
    Frame {
        /// Full planar interpretation, including component roles and raw claims.
        interpretation: &'a FrameInterpretation,
        /// This presentation's timing and packet provenance.
        timing: FrameTiming,
        /// Prepared native publication mechanism, with no conversion or privacy claim.
        choices: Choices<'a, PublicationOffer>,
    },
    /// Select one complete route while the source decoder is suspended.
    Transcode {
        /// Final source interpretation; no pixel bytes or metadata payloads.
        facts: InterpretationFacts,
        /// Prepared conversion/metadata/encoder combinations with storage scope.
        choices: Choices<'a, TranscodeOffer>,
    },
    /// Decide what to do before effects under newly discovered interpretation.
    Boundary {
        /// Facts about proposed output, excluding metadata payloads.
        facts: InterpretationFacts,
        /// Only mechanisms permitted by these facts.
        choices: Choices<'a, BoundaryOffer>,
    },
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
#[derive(Clone, Copy, Debug)]
pub struct PolicySet {
    prefer_lossless: bool,
    metadata: Option<MetadataTransfer>,
    allow_opaque_profiles: bool,
    workspace_limit: Option<u64>,
    allow_assumed_siting: bool,
    require_extended_range: bool,
}
impl Default for PolicySet {
    fn default() -> Self {
        Self::new()
    }
}
impl PolicySet {
    /// Choose the first prepared alternative in request order.
    pub const fn new() -> Self {
        Self {
            prefer_lossless: false,
            metadata: None,
            allow_opaque_profiles: true,
            workspace_limit: None,
            allow_assumed_siting: false,
            require_extended_range: false,
        }
    }
    /// Prefer a confirmed lossless mode when offered; otherwise preserve the
    /// request's first available choice. This never weakens a required target.
    /// For frame routes, first prefer both sample-preserving conversion and
    /// lossless encoding; then encoder-only lossless evidence. This preference
    /// is not a requirement for end-to-end sample preservation.
    pub const fn prefer_lossless(mut self, enabled: bool) -> Self {
        self.prefer_lossless = enabled;
        self
    }
    /// Require this descriptive-metadata treatment from a transcode route.
    /// This does not certify pixels, opaque profiles or unreported carriers.
    /// An encode-only question lacks the evidence and is refused.
    pub const fn require_metadata(mut self, treatment: MetadataTransfer) -> Self {
        self.metadata = Some(treatment);
        self
    }
    /// Permit retention of opaque source ICC profiles (the default). Disabling
    /// this requires a transcode offer proving that it does not retain one;
    /// encode-only questions cannot establish that and are refused.
    pub const fn allow_opaque_profiles(mut self, allowed: bool) -> Self {
        self.allow_opaque_profiles = allowed;
        self
    }
    /// Require a qualified bound on additional codec/conversion workspace.
    /// Unknown bounds refuse. This excludes managed storage, source storage and
    /// encoded staging: it is deliberately not a whole-operation memory cap.
    /// Encode-only questions have no such bound and are refused.
    pub const fn with_workspace_limit(mut self, bytes: u64) -> Self {
        self.workspace_limit = Some(bytes);
        self
    }
    /// Permit an explicitly disclosed assumption for undeclared chroma siting.
    /// Defaults to false. This never overrides a signaled location or permits
    /// guessing other color metadata. Only prepared conversion questions use it.
    pub const fn allow_assumed_chroma_siting(mut self, allowed: bool) -> Self {
        self.allow_assumed_siting = allowed;
        self
    }
    /// Require a conversion route that guarantees extended-range preservation.
    /// Defaults to false, allowing a requested integer/display conversion.
    /// This is separate from encoding fidelity and tone mapping; it does not
    /// establish an end-to-end guarantee. Other output questions lack this
    /// evidence and are refused; interpretation discovery may still proceed.
    /// Frame routes additionally require confirmed lossless encoding, so a
    /// range-preserving conversion cannot mask an unqualified encoder.
    pub const fn require_extended_range(mut self, required: bool) -> Self {
        self.require_extended_range = required;
        self
    }
    fn permits_transcode(self, offer: &TranscodeOffer) -> bool {
        self.metadata.is_none_or(|m| m == offer.metadata)
            && (self.allow_opaque_profiles || !offer.retains_opaque_profile)
            && self.workspace_limit.is_none_or(|limit| {
                offer
                    .workspace_upper_bound
                    .is_some_and(|bound| bound <= limit)
            })
    }
}
impl PolicyResolver for PolicySet {
    type Error = Infallible;
    fn resolve(&mut self, question: Question<'_>) -> Result<Decision, At<Infallible>> {
        if self.require_extended_range
            && !matches!(
                question,
                Question::ImageConversion { .. }
                    | Question::FrameRoute { .. }
                    | Question::Boundary { .. }
            )
        {
            return Ok(Decision::decline(Decline::UnsupportedQuestion));
        }
        let choices = match question {
            Question::ImageConversion { choices, .. } => {
                // Conversion alone does not certify metadata or opaque-profile
                // treatment. Do not silently ignore an existing obligation.
                if self.metadata.is_some() || !self.allow_opaque_profiles {
                    return Ok(Decision::decline(Decline::UnsupportedQuestion));
                }
                return Ok(choices
                    .iter()
                    .find(|choice| {
                        let offer = choice.value();
                        (offer.assumed_chroma_siting.is_none() || self.allow_assumed_siting)
                            && offer.assumed_chroma_siting
                                != Some(crate::frame::ChromaSiting::Unknown)
                            && (!self.require_extended_range || offer.preserves_extended_range)
                            && self.workspace_limit.is_none_or(|limit| {
                                offer
                                    .workspace_upper_bound
                                    .is_some_and(|bound| bound <= limit)
                            })
                    })
                    .map(Decision::choose)
                    .unwrap_or_else(|| Decision::decline(Decline::NoPreferredChoice)));
            }
            Question::FrameRoute { choices, .. } => {
                // Encoding continuity does not establish metadata treatment or
                // whole-container disclosure evidence.
                if self.metadata.is_some() || !self.allow_opaque_profiles {
                    return Ok(Decision::decline(Decline::UnsupportedQuestion));
                }
                let permitted = || {
                    choices.iter().filter(|choice| {
                        let conversion = choice.value().conversion;
                        let conversion_permitted = conversion.is_none_or(|c| {
                            (c.assumed_chroma_siting.is_none() || self.allow_assumed_siting)
                                && c.assumed_chroma_siting
                                    != Some(crate::frame::ChromaSiting::Unknown)
                        }) && (!self.require_extended_range
                            || (choice.value().fidelity == Some(Fidelity::Lossless)
                                && conversion.is_some_and(|c| c.preserves_extended_range)));
                        conversion_permitted
                            && self.workspace_limit.is_none_or(|limit| {
                                choice
                                    .value()
                                    .workspace_upper_bound
                                    .is_some_and(|bound| bound <= limit)
                            })
                    })
                };
                let selected = if self.prefer_lossless {
                    // Prefer a qualified complete route before considering
                    // encoder-only evidence. This is still a preference, not
                    // permission to violate any required bound.
                    permitted()
                        .find(|choice| {
                            choice.value().fidelity == Some(Fidelity::Lossless)
                                && choice
                                    .value()
                                    .conversion
                                    .is_some_and(|c| c.preserves_samples)
                        })
                        .or_else(|| {
                            permitted()
                                .find(|choice| choice.value().fidelity == Some(Fidelity::Lossless))
                        })
                        .or_else(|| permitted().next())
                } else {
                    permitted().next()
                };
                return Ok(selected
                    .map(Decision::choose)
                    .unwrap_or_else(|| Decision::decline(Decline::NoPreferredChoice)));
            }
            Question::Frame { choices, .. } | Question::Image { choices, .. } => {
                // Native publication has no evidence for these transcode-only
                // obligations. A preset must not silently ignore them.
                if self.metadata.is_some()
                    || !self.allow_opaque_profiles
                    || self.workspace_limit.is_some()
                {
                    return Ok(Decision::decline(Decline::UnsupportedQuestion));
                }
                return Ok(choices
                    .first()
                    .map(Decision::choose)
                    .unwrap_or_else(|| Decision::decline(Decline::NoPreferredChoice)));
            }
            Question::Transcode { choices, .. } => {
                let permitted = || choices.iter().filter(|c| self.permits_transcode(c.value()));
                let choice = if self.prefer_lossless {
                    permitted()
                        .find(|c| c.value().encoder.fidelity == Some(Fidelity::Lossless))
                        .or_else(|| permitted().next())
                } else {
                    permitted().next()
                };
                return Ok(choice
                    .map(Decision::choose)
                    .unwrap_or_else(|| Decision::decline(Decline::NoPreferredChoice)));
            }
            Question::Encode { choices, .. } => {
                if self.metadata.is_some()
                    || !self.allow_opaque_profiles
                    || self.workspace_limit.is_some()
                {
                    return Ok(Decision::decline(Decline::UnsupportedQuestion));
                }
                choices
            }
            Question::Boundary { choices, .. } => {
                return Ok(choices
                    .first()
                    .map(Decision::choose)
                    .unwrap_or_else(|| Decision::decline(Decline::NoPreferredChoice)));
            }
        };
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
    /// What prompted this decision, including on refusal or callback failure.
    pub context: DecisionContext,
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
#[derive(Clone, Default)]
pub struct OperationReport {
    records: [Option<DecisionRecord>; 16],
    head: usize,
    count: usize,
    dropped: u64,
}
impl fmt::Debug for OperationReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        struct Records<'a>(&'a OperationReport);
        impl fmt::Debug for Records<'_> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_list().entries(self.0.decisions()).finish()
            }
        }
        f.debug_struct("OperationReport")
            .field("records", &Records(self))
            .field("dropped", &self.dropped)
            .finish()
    }
}
impl OperationReport {
    /// Records in decision order; older records are dropped when the bound fills.
    pub fn decisions(&self) -> impl Iterator<Item = &DecisionRecord> {
        let first = self.count.min(self.records.len() - self.head);
        self.records[self.head..self.head + first]
            .iter()
            .chain(self.records[..self.count - first].iter())
            .filter_map(Option::as_ref)
    }
    /// Number of older records omitted from this bounded report.
    pub const fn dropped_decisions(&self) -> u64 {
        self.dropped
    }
    /// Append an adapter's decision, retaining only the bounded recent history.
    pub fn record(&mut self, record: DecisionRecord) {
        if self.count == self.records.len() {
            self.records[self.head] = Some(record);
            self.head = (self.head + 1) % self.records.len();
            self.dropped = self.dropped.saturating_add(1);
        } else {
            self.records[self.count] = Some(record);
            self.count += 1;
        }
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
    preset: PolicySet,
    stop: &dyn Stop,
) -> Result<(usize, DecisionRecord), At<PolicyFailure>> {
    choose(
        offers,
        DecisionContext::Encode(facts),
        |choices| Question::Encode { facts, choices },
        Selection::Encode,
        resolver,
        preset,
        stop,
    )
}

/// Resolve a real decoder boundary through the operation's one policy
/// interface. Adapter API: the caller must apply the returned mechanism to this
/// same suspended decoder and retain the decision record, including on failure.
pub fn resolve_boundary<R: PolicyResolver + ?Sized>(
    boundary: DecodeBoundary,
    resolver: &mut R,
    preset: PolicySet,
    stop: &dyn Stop,
) -> Result<(BoundaryAction, DecisionRecord), At<PolicyFailure>> {
    let facts = boundary.facts();
    let action = match facts.state {
        InterpretationState::Pending | InterpretationState::Provisional => {
            BoundaryAction::ContinueReading
        }
        _ => BoundaryAction::AdoptInterpretation,
    };
    let offers = [BoundaryOffer { action }];
    let offers = if action.permits(facts) {
        &offers[..]
    } else {
        &offers[..0]
    };
    let (index, record) = choose(
        offers,
        DecisionContext::Boundary(facts),
        |choices| Question::Boundary { facts, choices },
        |offer| Selection::Boundary(offer.action),
        resolver,
        preset,
        stop,
    )?;
    Ok((offers[index].action, record))
}

pub(super) fn choose<'a, T: Copy, R: PolicyResolver + ?Sized>(
    offers: &'a [T],
    context: DecisionContext,
    question: impl FnOnce(Choices<'a, T>) -> Question<'a>,
    selection: impl FnOnce(T) -> Selection,
    resolver: &mut R,
    mut preset: PolicySet,
    stop: &dyn Stop,
) -> Result<(usize, DecisionRecord), At<PolicyFailure>> {
    let mut record = DecisionRecord {
        question: 0,
        context,
        offered: offers.len().min(255) as u8,
        selected: None,
        used_preset: false,
        code: DecisionCode::Exhausted,
    };
    if offers.len() > MAX_OFFERS {
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
    let question = question(choices);
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
                record.selected = Some(selection(offers[index]));
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
    fn boundaries_refuse_unavailable_facts_and_cross_domain_choice_tokens() {
        let unavailable = DecodeBoundary::new(
            InterpretationFacts::new(2, 3, None, InterpretationState::Unavailable)
                .with_unavailable(crate::decode::InterpretationUnavailable::ForbiddenColorMetadata),
        )
        .unwrap();
        let mut called = false;
        let error = resolve_boundary(
            unavailable,
            &mut |question: Question<'_>| -> Result<Decision, At<Infallible>> {
                let Question::Boundary { choices, .. } = question else {
                    panic!("wrong domain")
                };
                called = true;
                assert!(choices.is_empty());
                Ok(Decision::UsePreset)
            },
            PolicySet::new(),
            &enough::Unstoppable,
        )
        .unwrap_err();
        assert!(called);
        assert_eq!(error.error().record().code, DecisionCode::Declined);
        assert_eq!(error.error().record().offered, 0);
        let DecisionContext::Boundary(facts) = error.error().record().context else {
            panic!("refusal lost its domain");
        };
        assert_eq!(facts, unavailable.facts());
        assert_eq!(
            facts.unavailable,
            Some(crate::decode::InterpretationUnavailable::ForbiddenColorMetadata)
        );
        let mut misleading = facts;
        misleading.state = InterpretationState::Final;
        misleading.descriptor = Some(PixelDescriptor::RGB8_SRGB);
        assert!(!BoundaryAction::AdoptInterpretation.permits(misleading));

        let mut token = None;
        choose_encode(
            EncodeFacts {
                width: 2,
                height: 3,
                descriptor: PixelDescriptor::RGB8_SRGB,
            },
            &[EncodeOffer {
                format: ImageFormat::Jpeg,
                fidelity: None,
                preference_index: 0,
                resolution: None,
            }],
            &mut |question: Question<'_>| -> Result<Decision, At<Infallible>> {
                let Question::Encode { choices, .. } = question else {
                    panic!("wrong domain")
                };
                token = Some(choices.first().unwrap().id());
                Ok(Decision::UsePreset)
            },
            PolicySet::new(),
            &enough::Unstoppable,
        )
        .unwrap();
        let final_facts = DecodeBoundary::new(InterpretationFacts::new(
            2,
            3,
            Some(PixelDescriptor::RGB8_SRGB),
            InterpretationState::Final,
        ))
        .unwrap();
        let error = resolve_boundary(
            final_facts,
            &mut |_: Question<'_>| -> Result<Decision, At<Infallible>> {
                Ok(Decision::Choose(token.unwrap()))
            },
            PolicySet::new(),
            &enough::Unstoppable,
        )
        .unwrap_err();
        assert_eq!(error.error().record().code, DecisionCode::InvalidChoice);
    }
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
            report.record(DecisionRecord {
                question,
                context: DecisionContext::Encode(EncodeFacts {
                    width: 2,
                    height: 3,
                    descriptor: PixelDescriptor::RGB8_SRGB,
                }),
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

    #[test]
    fn journal_matches_recent_history_through_wraps_and_clones() {
        use alloc::{collections::VecDeque, vec::Vec};
        let mut report = OperationReport::default();
        let mut expected = VecDeque::new();
        assert_eq!(report.decisions().count(), 0);
        assert_eq!(report.dropped_decisions(), 0);
        let mut previous = report.clone();
        let mut previous_questions = Vec::new();
        for question in 1..=1024 {
            let record = DecisionRecord {
                question,
                context: DecisionContext::Encode(EncodeFacts {
                    width: question as u32,
                    height: 3,
                    descriptor: PixelDescriptor::RGB8_SRGB,
                }),
                offered: 1,
                selected: None,
                used_preset: question % 2 == 0,
                code: DecisionCode::Declined,
            };
            expected.push_back(record);
            if expected.len() > 16 {
                expected.pop_front();
            }
            report.record(record);
            // Debug includes every retained field and must keep chronological order.
            assert_eq!(
                alloc::format!("{:?}", report.decisions().collect::<Vec<_>>()),
                alloc::format!("{:?}", expected.iter().collect::<Vec<_>>())
            );
            assert_eq!(report.dropped_decisions(), question.saturating_sub(16));
            let actual = report
                .decisions()
                .map(|record| record.question)
                .collect::<Vec<_>>();
            assert_eq!(
                previous
                    .decisions()
                    .map(|record| record.question)
                    .collect::<Vec<_>>(),
                previous_questions,
                "cloning must not share mutable history"
            );
            let debug = alloc::format!("{report:?}");
            let expected_debug = alloc::format!(
                "OperationReport {{ records: {:?}, dropped: {} }}",
                expected.iter().collect::<Vec<_>>(),
                question.saturating_sub(16)
            );
            assert_eq!(debug, expected_debug);
            previous = report.clone();
            previous_questions = actual;
        }
        report.dropped = u64::MAX;
        report.record(*expected.back().unwrap());
        assert_eq!(report.dropped_decisions(), u64::MAX);
        assert_eq!(report.decisions().count(), 16);
    }
}
