//! Private suspension protocol. The native integration uses QOI; this controller
//! does not pretend that existing IncrementalDecode metadata events are barriers.
#![allow(dead_code)]
use crate::admission::{Identity, Lease, Limits, Refusal, Storage, Transfer};
use crate::policy::*;
use core::{error::Error, fmt, sync::atomic::AtomicUsize};
use zencodec::{At, decode::ByteRange};

static NEXT_GATE: AtomicUsize = AtomicUsize::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Interpretation {
    Srgb,
    Linear,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Knowledge {
    Pending,
    Provisional(Interpretation),
    Final(Interpretation),
    Unavailable,
}
impl Knowledge {
    fn value(self) -> Option<Interpretation> {
        match self {
            Self::Provisional(v) | Self::Final(v) => Some(v),
            _ => None,
        }
    }
    fn status(self) -> FactStatus {
        match self {
            Self::Pending => FactStatus::Pending,
            Self::Provisional(_) => FactStatus::Provisional,
            Self::Final(_) => FactStatus::Final,
            Self::Unavailable => FactStatus::Unavailable,
        }
    }
    fn cicp(self) -> Option<zencodec::Cicp> {
        self.value().map(|value| match value {
            Interpretation::Srgb => zencodec::Cicp::SRGB,
            Interpretation::Linear => zencodec::Cicp::new(1, 8, 0, true),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GateCode {
    IdentityExhausted,
    Stale,
    Suspended,
    Terminal,
    ContradictoryFinality,
    Incomplete,
    EpochNotSupported,
    Overflow,
    Capacity(Refusal),
    Stopped,
}
impl fmt::Display for GateCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "stream gate: {self:?}")
    }
}
impl Error for GateCode {}

#[derive(Debug)]
pub(crate) enum ResolveFailure<E> {
    Gate(GateCode),
    Policy(Failure<E>),
}
impl<E> fmt::Display for ResolveFailure<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gate(e) => e.fmt(f),
            Self::Policy(e) => e.fmt(f),
        }
    }
}
impl<E: Error + 'static> Error for ResolveFailure<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(match self {
            Self::Gate(e) => e,
            Self::Policy(e) => e,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Suspended,
    Holding,
    Ready,
    Terminal,
}

/// No public constructor; authorization is bound to this gate and fact revision.
#[derive(Debug)]
pub(crate) struct Receipt {
    gate: u64,
    revision: u64,
    epoch: u64,
    action: BoundaryAction,
}
impl Receipt {
    pub fn action(&self) -> BoundaryAction {
        self.action
    }
}

pub(crate) struct Gate {
    id: u64,
    context: Context,
    knowledge: Knowledge,
    previous: Option<Interpretation>,
    phase: Phase,
    commitments: Commitments,
    staging_limit: usize,
    memory: Storage,
    staged: Vec<Lease>,
    staged_end: u64,
    segmented: bool,
}
impl Gate {
    pub fn new(source: u64, staging_limit: usize, segmented: bool) -> Result<Self, At<GateCode>> {
        let id = issue_ticket(&NEXT_GATE).ok_or_else(|| At::wrap(GateCode::IdentityExhausted))?;
        Ok(Self {
            id,
            context: Context {
                source_revision: source,
                ..Context::default()
            },
            knowledge: Knowledge::Pending,
            previous: None,
            phase: Phase::Suspended,
            commitments: Commitments::default(),
            staging_limit,
            memory: Storage::new(Limits {
                bytes: staging_limit,
                descriptors: 64,
                range_bytes: staging_limit,
                merge_gap: 0,
            }),
            staged: Vec::new(),
            staged_end: 0,
            segmented,
        })
    }

    /// Import a legacy adapter's already committed provisional interpretation.
    /// This is an adversarial integration control, not a recommended start path.
    pub fn with_prior_effects(mut self, value: Interpretation, commitments: Commitments) -> Self {
        self.previous = Some(value);
        self.knowledge = Knowledge::Provisional(value);
        self.context.published_bytes = commitments.body_bytes;
        self.commitments = commitments;
        self
    }

    pub fn storage(&self) -> Storage {
        self.memory.clone()
    }
    pub fn context(&self) -> Context {
        self.context
    }
    pub fn commitments(&self) -> Commitments {
        self.commitments
    }
    pub fn bytes_held(&self) -> usize {
        self.memory.occupancy().bytes
    }
    pub fn abort(&mut self) {
        self.phase = Phase::Terminal;
        self.staged.clear();
    }

    /// Called BEFORE affected sink/conversion/publication effects. The native
    /// Event::Metadata contract cannot by itself supply this ordering guarantee.
    pub fn discover(&mut self, fact: Knowledge) -> Result<bool, At<GateCode>> {
        if self.phase == Phase::Terminal {
            return Err(At::wrap(GateCode::Terminal));
        }
        if fact == self.knowledge {
            return Ok(false);
        }
        if matches!(self.knowledge, Knowledge::Final(_)) {
            self.abort();
            return Err(At::wrap(GateCode::ContradictoryFinality));
        }
        let Some(next) = self.context.plan_revision.checked_add(1) else {
            self.abort();
            return Err(At::wrap(GateCode::Overflow));
        };
        self.context.plan_revision = next;
        self.knowledge = fact;
        self.phase = Phase::Suspended;
        Ok(true)
    }

    pub fn resolve<R: PolicyResolver + ?Sized>(
        &mut self,
        engine: &mut Engine,
        resolver: &mut R,
        stop: &dyn zencodec::Pulse,
    ) -> Result<Receipt, At<ResolveFailure<R::Error>>> {
        if self.phase != Phase::Suspended {
            let code = if self.phase == Phase::Terminal {
                GateCode::Terminal
            } else {
                GateCode::Stale
            };
            return Err(At::wrap(ResolveFailure::Gate(code)));
        }
        let changed = self.previous != self.knowledge.value();
        let committed = self.commitments != Commitments::default();
        let action = match self.knowledge {
            Knowledge::Pending | Knowledge::Provisional(_) => Some(BoundaryAction::Wait),
            Knowledge::Final(_) if !changed || !committed => Some(BoundaryAction::Adopt),
            _ => None,
        };
        let alternative = Alternative {
            action: Action::Boundary(action.unwrap_or(BoundaryAction::Adopt)),
            ram: Evidence::Enforced(self.staging_limit as u64),
            disk: Evidence::Upper(0),
            accounting: Accounting::Driver,
            effects: Effects(0),
            needs_replay: false,
            preserves_published: true,
        };
        // A panic leaves the gate terminal. Existing admitted payload remains
        // charged until abort/drop; no budget lock is held while calling policy.
        self.phase = Phase::Terminal;
        let result = engine.resolve(
            Facts::Boundary(BoundaryFacts {
                trigger: BoundaryTrigger::Interpretation {
                    status: self.knowledge.status(),
                    cicp: self.knowledge.cicp(),
                    changes_interpretation: changed,
                },
                commitments: self.commitments,
            }),
            if action.is_some() {
                core::slice::from_ref(&alternative)
            } else {
                &[]
            },
            Requirements {
                ram: Some(self.staging_limit as u64),
                disk: Some(0),
                accounting: Accounting::Driver,
                effects: Effects(0),
            },
            self.context,
            resolver,
            stop,
        );
        let selected = match result {
            Ok(selected) => selected,
            Err(error) => {
                self.staged.clear();
                return Err(error
                    .map_error(ResolveFailure::Policy)
                    .at_named("stream suspended at discovery"));
            }
        };
        let Action::Boundary(action) = selected.alternative.action else {
            unreachable!()
        };
        self.phase = if action == BoundaryAction::Wait {
            Phase::Holding
        } else {
            Phase::Ready
        };
        if self.phase == Phase::Ready {
            self.previous = self.knowledge.value();
        }
        Ok(Receipt {
            gate: self.id,
            revision: self.context.plan_revision,
            epoch: self.context.epoch,
            action,
        })
    }

    fn check(&self, receipt: &Receipt, phase: Phase) -> Result<(), At<GateCode>> {
        if self.phase == Phase::Terminal {
            return Err(At::wrap(GateCode::Terminal));
        }
        if receipt.gate != self.id
            || receipt.revision != self.context.plan_revision
            || receipt.epoch != self.context.epoch
        {
            return Err(At::wrap(GateCode::Stale));
        }
        if self.phase != phase {
            return Err(At::wrap(GateCode::Suspended));
        }
        Ok(())
    }
    pub fn before_effect(&self, receipt: &Receipt) -> Result<(), At<GateCode>> {
        self.check(receipt, Phase::Ready)
    }
    pub fn stage(
        &mut self,
        receipt: &Receipt,
        bytes: &[u8],
        stop: &dyn zencodec::Pulse,
    ) -> Result<(), At<GateCode>> {
        self.check(receipt, Phase::Holding)?;
        let result = (|| {
            stop.check().map_err(|_| GateCode::Stopped)?;
            let end = self
                .staged_end
                .checked_add(bytes.len() as u64)
                .ok_or(GateCode::Overflow)?;
            self.staged
                .try_reserve(1)
                .map_err(|_| GateCode::Capacity(Refusal::Allocation))?;
            let identity = Identity(self.context.source_revision);
            let mut transfer = Transfer::new(
                &self.memory,
                identity,
                ByteRange::new(self.staged_end, bytes.len() as u64),
            )
            .map_err(GateCode::Capacity)?;
            transfer.append(bytes).map_err(GateCode::Capacity)?;
            self.staged
                .push(transfer.publish(identity).map_err(GateCode::Capacity)?);
            self.staged_end = end;
            Ok(())
        })();
        if result.is_err() {
            self.abort();
        }
        result.map_err(At::wrap)
    }
    pub fn take_staged(&mut self, receipt: &Receipt) -> Result<Vec<Lease>, At<GateCode>> {
        self.before_effect(receipt)?;
        Ok(core::mem::take(&mut self.staged))
    }
    pub fn encoder_started(&mut self, receipt: &Receipt) -> Result<(), At<GateCode>> {
        self.before_effect(receipt)?;
        self.commitments.encoder_started = true;
        Ok(())
    }
    pub fn headers_sent(&mut self, receipt: &Receipt) -> Result<(), At<GateCode>> {
        self.before_effect(receipt)?;
        self.commitments.headers_sent = true;
        Ok(())
    }
    pub fn published(&mut self, receipt: &Receipt, bytes: usize) -> Result<(), At<GateCode>> {
        self.before_effect(receipt)?;
        let body = self.commitments.body_bytes.checked_add(bytes as u64);
        let total = self.context.published_bytes.checked_add(bytes as u64);
        match (body, total) {
            (Some(body), Some(total)) => {
                self.commitments.body_bytes = body;
                self.context.published_bytes = total;
                Ok(())
            }
            _ => {
                self.abort();
                Err(At::wrap(GateCode::Overflow))
            }
        }
    }
    pub fn next_epoch(&mut self, receipt: &Receipt) -> Result<(), At<GateCode>> {
        self.before_effect(receipt)?;
        if !self.segmented {
            return Err(At::wrap(GateCode::EpochNotSupported));
        }
        if !self.staged.is_empty() {
            return Err(At::wrap(GateCode::Incomplete));
        }
        self.context.epoch = self
            .context
            .epoch
            .checked_add(1)
            .ok_or_else(|| At::wrap(GateCode::Overflow))?;
        self.phase = Phase::Suspended;
        self.knowledge = Knowledge::Pending;
        self.previous = None;
        // The prior independently interpreted segment stays immutable; global
        // publication count is retained. Native muxer support is not qualified.
        self.commitments = Commitments::default();
        Ok(())
    }
    pub fn finish(&mut self, receipt: &Receipt) -> Result<(), At<GateCode>> {
        let result = self.before_effect(receipt).and_then(|()| {
            if self.staged.is_empty() {
                Ok(())
            } else {
                Err(At::wrap(GateCode::Incomplete))
            }
        });
        self.abort();
        result
    }
}
