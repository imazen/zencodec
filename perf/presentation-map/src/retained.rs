//! Private owner-count admission experiment, not a byte/workspace bound.
use crate::{Cursor, Error, Interval, Plan};
use alloc::{
    sync::{Arc, Weak},
    vec::Vec,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdmissionError {
    Limit,
    Allocation,
    Plan(Error),
}

/// One pool per operation. Cloning leases does not consume additional slots.
/// A slot remains occupied until the last cursor/replay lease is dropped.
/// Arc control blocks use the allocator's ordinary allocation behavior.
pub struct PlanPool {
    limit: usize,
    owners: Vec<Weak<Plan>>,
}
impl PlanPool {
    pub fn new(limit: usize) -> Self {
        Self {
            limit,
            owners: Vec::new(),
        }
    }
    pub fn live(&self) -> usize {
        self.owners
            .iter()
            .filter(|owner| owner.strong_count() != 0)
            .count()
    }
    pub fn prepare(
        &mut self,
        build: impl FnOnce() -> Result<Plan, Error>,
    ) -> Result<RetainedPlan, AdmissionError> {
        self.owners.retain(|owner| owner.strong_count() != 0);
        if self.owners.len() >= self.limit {
            return Err(AdmissionError::Limit);
        }
        // Admission and bookkeeping reservation precede descriptor construction.
        self.owners
            .try_reserve(1)
            .map_err(|_| AdmissionError::Allocation)?;
        let plan = Arc::new(build().map_err(AdmissionError::Plan)?);
        self.owners.push(Arc::downgrade(&plan));
        Ok(RetainedPlan(plan))
    }
}
#[derive(Clone)]
pub struct RetainedPlan(Arc<Plan>);
impl RetainedPlan {
    pub fn plan(&self) -> &Plan {
        &self.0
    }
    pub fn same_owner(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
    pub fn cursor(&self, segment: usize) -> Result<RetainedCursor, Error> {
        let segment = *self
            .0
            .segments()
            .get(segment)
            .ok_or(Error::InvalidInterval)?;
        Ok(RetainedCursor {
            owner: self.clone(),
            cursor: Cursor::new(segment)?,
        })
    }
}
pub struct RetainedCursor {
    owner: RetainedPlan,
    cursor: Cursor,
}
impl RetainedCursor {
    pub fn owner(&self) -> &RetainedPlan {
        &self.owner
    }
    pub fn push(&mut self, input: Interval) -> Result<Option<Interval>, Error> {
        self.cursor.push(input)
    }
    pub fn finish(&mut self) -> Result<Interval, Error> {
        self.cursor.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zencodec::time::{TimeBase, Timestamp};
    fn empty() -> Result<Plan, Error> {
        Plan::new(Timestamp::new(0, TimeBase::MILLIS), &[])
    }
    #[test]
    fn admission_precedes_build_and_replay_retains_slot() {
        let mut pool = PlanPool::new(1);
        let first = pool.prepare(empty).unwrap();
        let replay = first.clone();
        assert!(first.same_owner(&replay));
        drop(first);
        assert!(matches!(
            pool.prepare(|| panic!("build before admission")),
            Err(AdmissionError::Limit)
        ));
        assert_eq!(pool.live(), 1);
        drop(replay);
        assert_eq!(pool.live(), 0);
        assert!(matches!(
            pool.prepare(|| Err(Error::InvalidInterval)),
            Err(AdmissionError::Plan(Error::InvalidInterval))
        ));
        assert_eq!(pool.live(), 0);
        let next = pool.prepare(empty).unwrap();
        assert_eq!(pool.live(), 1);
        drop(pool);
        assert!(
            next.plan().segments().is_empty(),
            "pool drop must not invalidate an owner"
        );
        assert!(matches!(
            PlanPool::new(0).prepare(|| panic!("zero limit built")),
            Err(AdmissionError::Limit)
        ));
    }
}
