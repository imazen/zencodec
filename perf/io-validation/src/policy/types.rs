use core::fmt;
use zencodec::At;
use zencodec::encode::Fidelity;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Execution,
    Metadata,
    Fidelity,
    Media,
    Boundary,
    Unknown(u16),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Evidence {
    Upper(u64),
    /// Every growth in the stated scope must reserve before allocating. This
    /// promises a cap or failure, not successful completion within the cap.
    Enforced(u64),
    Estimate(u64),
    Unknown,
}
impl Evidence {
    pub fn fits(self, limit: Option<u64>) -> bool {
        match (self, limit) {
            (_, None) => true,
            (Self::Upper(n) | Self::Enforced(n), Some(cap)) => n <= cap,
            _ => false,
        }
    }
    pub fn bound(self) -> Option<u64> {
        match self {
            Self::Upper(n) | Self::Enforced(n) => Some(n),
            Self::Estimate(_) | Self::Unknown => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Accounting {
    Driver,
    Operation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Execution {
    Buffered,
    Sequential,
    DiskStaged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MetadataAction {
    KeepExact,
    FilterPublic,
    DropDescriptive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MediaAction {
    KeepTrack,
    DropTrack,
}

/// Choices at a newly discovered interpretation or disclosure boundary.
/// Adapters qualify these alternatives before presenting them to the resolver.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BoundaryAction {
    Wait,
    Keep,
    Regenerate,
    Drop,
    Adopt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Carrier {
    Xmp,
    Icc,
    Reconstruction,
    NestedObject,
    UnknownCritical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Inspection {
    Unavailable,
    Malformed,
    Limit,
    SyntaxOnly,
    SupportedSubset,
    Canonical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FactStatus {
    Pending,
    Provisional,
    Final,
    Unavailable,
}

/// These are independent commitments: zero body bytes does not imply that a
/// new format, sample interpretation or encoder configuration remains possible.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Commitments {
    pub samples_changed: bool,
    pub encoder_started: bool,
    pub headers_sent: bool,
    pub body_bytes: u64,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum BoundaryTrigger {
    Disclosure {
        carrier: Carrier,
        inspection: Inspection,
        bytes: u64,
        required: bool,
        strict: bool,
    },
    Interpretation {
        status: FactStatus,
        cicp: Option<zencodec::Cicp>,
        changes_interpretation: bool,
    },
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct BoundaryFacts {
    pub trigger: BoundaryTrigger,
    pub commitments: Commitments,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Action {
    Execution(Execution),
    Metadata(MetadataAction),
    Fidelity(Fidelity),
    Media(MediaAction),
    Boundary(BoundaryAction),
}
impl Action {
    pub fn kind(self) -> Kind {
        match self {
            Self::Execution(_) => Kind::Execution,
            Self::Metadata(_) => Kind::Metadata,
            Self::Fidelity(_) => Kind::Fidelity,
            Self::Media(_) => Kind::Media,
            Self::Boundary(_) => Kind::Boundary,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Effects(pub u8);
impl Effects {
    pub const RIGHTS: Self = Self(1);
    pub const NO_GPS: Self = Self(2);
    pub const EXACT_METADATA: Self = Self(4);
    pub const EARLY_OUTPUT: Self = Self(8);
    pub const LOSSLESS: Self = Self(16);
    pub const KEEP_TRACK: Self = Self(32);
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
    pub fn domain(self, kind: Kind) -> Self {
        Self(
            self.0
                & match kind {
                    Kind::Metadata => 7,
                    Kind::Execution => 8,
                    Kind::Fidelity => 16,
                    Kind::Media => 32,
                    Kind::Boundary | Kind::Unknown(_) => 0,
                },
        )
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Alternative {
    pub action: Action,
    pub ram: Evidence,
    pub disk: Evidence,
    pub accounting: Accounting,
    pub effects: Effects,
    pub needs_replay: bool,
    pub preserves_published: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Requirements {
    pub ram: Option<u64>,
    pub disk: Option<u64>,
    pub accounting: Accounting,
    pub effects: Effects,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RejectedBy {
    Ram {
        cost: Evidence,
        limit: u64,
    },
    Disk {
        cost: Evidence,
        limit: u64,
    },
    Scope {
        available: Accounting,
        required: Accounting,
    },
    Effects {
        available: Effects,
        required: Effects,
    },
    Replay,
    PublishedPrefix,
}
impl Alternative {
    pub fn rejections(
        self,
        requirements: Requirements,
        kind: Kind,
        context: Context,
    ) -> [Option<RejectedBy>; 6] {
        let effects = requirements.effects.domain(kind);
        [
            requirements
                .ram
                .filter(|_| !self.ram.fits(requirements.ram))
                .map(|limit| RejectedBy::Ram {
                    cost: self.ram,
                    limit,
                }),
            requirements
                .disk
                .filter(|_| !self.disk.fits(requirements.disk))
                .map(|limit| RejectedBy::Disk {
                    cost: self.disk,
                    limit,
                }),
            (requirements.ram.is_some()
                && requirements.accounting == Accounting::Operation
                && self.accounting != Accounting::Operation)
                .then_some(RejectedBy::Scope {
                    available: self.accounting,
                    required: requirements.accounting,
                }),
            (!self.effects.contains(effects)).then_some(RejectedBy::Effects {
                available: self.effects,
                required: effects,
            }),
            (self.needs_replay && (!context.replayable || context.published_bytes != 0))
                .then_some(RejectedBy::Replay),
            (context.published_bytes != 0 && !self.preserves_published)
                .then_some(RejectedBy::PublishedPrefix),
        ]
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Context {
    pub source_revision: u64,
    pub plan_revision: u64,
    pub epoch: u64,
    pub published_bytes: u64,
    pub replayable: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ExecutionFacts {
    pub resident_input: u64,
    pub source_length: Option<u64>,
    pub network: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct MetadataFacts {
    /// Scope is the supplied descriptive packet, not whole-file anonymity.
    pub packet_bytes: u64,
    pub parse_complete: bool,
    pub has_location: Option<bool>,
    pub has_rights: Option<bool>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct FidelityFacts {
    pub alpha: bool,
    pub bits_per_sample: u8,
}

/// Application-layer facts copied from the media contract, with no dependency
/// from the core resolver vocabulary back to its media implementation.
#[derive(Clone, Copy, Debug)]
pub(crate) struct MediaFacts {
    pub track: u32,
    pub epoch: u32,
    pub audio: bool,
    pub first_packet_bytes: u64,
    pub first_pts_known: bool,
    pub random_access: Option<bool>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Facts {
    Execution(ExecutionFacts),
    Metadata(MetadataFacts),
    Fidelity(FidelityFacts),
    Media(MediaFacts),
    Boundary(BoundaryFacts),
    Unknown(u16),
}
impl Facts {
    pub fn kind(self) -> Kind {
        match self {
            Self::Execution(_) => Kind::Execution,
            Self::Metadata(_) => Kind::Metadata,
            Self::Fidelity(_) => Kind::Fidelity,
            Self::Media(_) => Kind::Media,
            Self::Boundary(_) => Kind::Boundary,
            Self::Unknown(code) => Kind::Unknown(code),
        }
    }
}

/// An ID cannot be manufactured outside this module. The engine verifies both
/// its globally issued question ticket and membership of the offered set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ChoiceId {
    ticket: u64,
    index: u8,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Choice<'a> {
    pub id: ChoiceId,
    pub value: &'a Alternative,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Choices<'a> {
    ticket: u64,
    admitted: u64,
    entries: &'a [Alternative],
}
impl<'a> Choices<'a> {
    pub(super) fn new(ticket: u64, admitted: u64, entries: &'a [Alternative]) -> Self {
        Self {
            ticket,
            admitted,
            entries,
        }
    }
    pub fn iter(self) -> impl Iterator<Item = Choice<'a>> {
        self.entries
            .iter()
            .enumerate()
            .filter_map(move |(index, value)| {
                (self.admitted & (1u64 << index) != 0).then_some(Choice {
                    id: ChoiceId {
                        ticket: self.ticket,
                        index: index as u8,
                    },
                    value,
                })
            })
    }
    pub fn first(self) -> Option<Choice<'a>> {
        self.iter().next()
    }
    pub(super) fn validate(self, id: ChoiceId) -> Option<(usize, &'a Alternative)> {
        let index = usize::from(id.index);
        if id.ticket != self.ticket
            || index >= self.entries.len()
            || self.admitted & (1u64 << index) == 0
        {
            return None;
        }
        Some((index, &self.entries[index]))
    }
}

#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub(crate) enum Question<'a> {
    Execution {
        facts: ExecutionFacts,
        choices: Choices<'a>,
    },
    Metadata {
        facts: MetadataFacts,
        choices: Choices<'a>,
    },
    Fidelity {
        facts: FidelityFacts,
        choices: Choices<'a>,
    },
    Media {
        facts: MediaFacts,
        choices: Choices<'a>,
    },
    Boundary {
        facts: BoundaryFacts,
        choices: Choices<'a>,
    },
    /// A compatibility control for a question not understood by this version.
    Unknown { code: u16 },
}
impl<'a> Question<'a> {
    pub(super) fn new(facts: Facts, choices: Choices<'a>) -> Self {
        match facts {
            Facts::Execution(facts) => Self::Execution { facts, choices },
            Facts::Metadata(facts) => Self::Metadata { facts, choices },
            Facts::Fidelity(facts) => Self::Fidelity { facts, choices },
            Facts::Media(facts) => Self::Media { facts, choices },
            Facts::Boundary(facts) => Self::Boundary { facts, choices },
            Facts::Unknown(code) => Self::Unknown { code },
        }
    }
    pub fn choices(self) -> Option<Choices<'a>> {
        match self {
            Self::Execution { choices, .. }
            | Self::Metadata { choices, .. }
            | Self::Fidelity { choices, .. } => Some(choices),
            Self::Media { choices, .. } | Self::Boundary { choices, .. } => Some(choices),
            Self::Unknown { .. } => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Decline {
    ApplicationRule,
    NoPreferredChoice,
    UnknownQuestion,
}
impl fmt::Display for Decline {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "policy declined: {self:?}")
    }
}
impl core::error::Error for Decline {}

#[derive(Debug)]
pub(crate) enum Decision {
    Choose(ChoiceId),
    UsePreset,
    /// Expected denial has an origin trace too. Engine conversion transfers
    /// this trace instead of nesting or discarding it.
    Reject(At<Decline>),
}
impl Decision {
    pub fn choose(choice: Choice<'_>) -> Self {
        Self::Choose(choice.id)
    }
    #[track_caller]
    pub fn reject(reason: Decline) -> Self {
        Self::Reject(At::wrap(reason).at())
    }
}
