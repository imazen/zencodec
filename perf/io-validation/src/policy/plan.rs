//! A conservative joint-plan control. Costs are added as if all selected stages
//! overlap. A production planner needs stage lifetimes to tighten that bound.
use super::*;
use core::{error::Error, fmt};
use zencodec::At;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlanError {
    Missing,
    Duplicate,
    ForeignOperation,
    ChangedRequest,
    Stale,
    UnprovenCost,
    Overflow,
    Ram,
    Disk,
    RequiredEffect,
    UnexpectedQuestion,
}
impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "joint policy plan: {self:?}")
    }
}
impl Error for PlanError {}

#[derive(Debug)]
pub(crate) struct Plan {
    pub operation: u64,
    pub context: Context,
    pub requirements: Requirements,
    pub ram: u64,
    pub disk: u64,
    pub execution: Execution,
    pub metadata: MetadataAction,
    pub fidelity: zencodec::encode::Fidelity,
}
impl Plan {
    #[track_caller]
    pub fn combine(selections: &[Selected], context: Context) -> Result<Self, At<PlanError>> {
        let fail = |e| At::wrap(e).at();
        let first = selections.first().ok_or_else(|| fail(PlanError::Missing))?;
        let (mut ram, mut disk) = (0u64, 0u64);
        let (mut execution, mut metadata, mut fidelity) = (None, None, None);
        let mut effects = Effects::default();
        for selected in selections {
            if selected.operation != first.operation {
                return Err(fail(PlanError::ForeignOperation));
            }
            if selected.requirements != first.requirements {
                return Err(fail(PlanError::ChangedRequest));
            }
            if selected.context != context {
                return Err(fail(PlanError::Stale));
            }
            let choice = selected.alternative;
            // This admission experiment only executes bounded plans. Unknown
            // whole-operation codec allocation must not masquerade as bounded.
            let r = choice
                .ram
                .bound()
                .ok_or_else(|| fail(PlanError::UnprovenCost))?;
            let d = choice
                .disk
                .bound()
                .ok_or_else(|| fail(PlanError::UnprovenCost))?;
            ram = ram
                .checked_add(r)
                .ok_or_else(|| fail(PlanError::Overflow))?;
            disk = disk
                .checked_add(d)
                .ok_or_else(|| fail(PlanError::Overflow))?;
            let duplicate = match choice.action {
                Action::Execution(v) => execution.replace(v).is_some(),
                Action::Metadata(v) => metadata.replace(v).is_some(),
                Action::Fidelity(v) => fidelity.replace(v).is_some(),
                Action::Media(_) => return Err(fail(PlanError::UnexpectedQuestion)),
            };
            if duplicate {
                return Err(fail(PlanError::Duplicate));
            }
            effects = effects.union(choice.effects);
        }
        let execution = execution.ok_or_else(|| fail(PlanError::Missing))?;
        let metadata = metadata.ok_or_else(|| fail(PlanError::Missing))?;
        let fidelity = fidelity.ok_or_else(|| fail(PlanError::Missing))?;
        let requirements = first.requirements;
        if requirements.ram.is_some_and(|cap| ram > cap) {
            return Err(fail(PlanError::Ram));
        }
        if requirements.disk.is_some_and(|cap| disk > cap) {
            return Err(fail(PlanError::Disk));
        }
        if !effects.contains(requirements.effects) {
            return Err(fail(PlanError::RequiredEffect));
        }
        Ok(Self {
            operation: first.operation,
            context,
            requirements,
            ram,
            disk,
            execution,
            metadata,
            fidelity,
        })
    }

    #[track_caller]
    pub fn validate_start(&self, current: Context) -> Result<(), At<PlanError>> {
        if current != self.context {
            return Err(At::wrap(PlanError::Stale).at());
        }
        Ok(())
    }
}
