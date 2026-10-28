//! Execution control for the policy experiment. The budget covers the exact
//! payload allocations made here, not codec internals, allocator overhead or RSS.
#![allow(dead_code)]
use crate::admission::{Identity, Lease, Limits, Refusal, Reservation, Storage, Transfer};
use crate::policy::{
    Context,
    plan::{Plan, PlanError},
};
use std::{error::Error, fmt, sync::Arc};
use zencodec::{
    At,
    decode::ByteRange,
    enough::StopReason,
};

#[derive(Debug)]
pub(crate) enum AdmissionError {
    Plan(PlanError),
    Capacity(Refusal),
    Size,
    Stopped(StopReason),
}
impl fmt::Display for AdmissionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "policy admission: {self:?}")
    }
}
impl Error for AdmissionError {}

struct Account {
    plan: Plan,
    local: Storage,
    _ram: Reservation,
    _disk: Option<Reservation>,
}
pub(crate) struct Admitted(Arc<Account>);
pub(crate) struct Pending {
    transfer: Transfer,
    account: Arc<Account>,
}
#[derive(Clone)]
pub(crate) struct Buffer {
    // Release bytes and local charge before the shared reservation.
    lease: Lease,
    _account: Arc<Account>,
}

impl Admitted {
    #[track_caller]
    pub fn start(
        plan: Plan,
        current: Context,
        memory: &Storage,
        disk: &Storage,
        stop: &dyn zencodec::Pulse,
    ) -> Result<Self, At<AdmissionError>> {
        plan.validate_start(current)
            .map_err(|e| e.map_error(AdmissionError::Plan))?;
        stop.check()
            .map_err(|e| At::wrap(AdmissionError::Stopped(e)).at())?;
        let size = usize::try_from(plan.ram).map_err(|_| At::wrap(AdmissionError::Size).at())?;
        let disk_size =
            usize::try_from(plan.disk).map_err(|_| At::wrap(AdmissionError::Size).at())?;
        let ram = memory
            .reserve(size)
            .map_err(|e| At::wrap(AdmissionError::Capacity(e)).at())?;
        let disk = if disk_size == 0 {
            None
        } else {
            Some(
                disk.reserve(disk_size)
                    .map_err(|e| At::wrap(AdmissionError::Capacity(e)).at())?,
            )
        };
        // No callback executes while either reservation is held.
        let local = Storage::new(Limits {
            bytes: size,
            descriptors: 64,
            range_bytes: size,
            merge_gap: 0,
        });
        Ok(Self(Arc::new(Account {
            plan,
            local,
            _ram: ram,
            _disk: disk,
        })))
    }

    #[track_caller]
    pub fn allocate(
        &self,
        range: ByteRange,
        current: Context,
        stop: &dyn zencodec::Pulse,
    ) -> Result<Pending, At<AdmissionError>> {
        self.0
            .plan
            .validate_start(current)
            .map_err(|e| e.map_error(AdmissionError::Plan))?;
        stop.check()
            .map_err(|e| At::wrap(AdmissionError::Stopped(e)).at())?;
        let transfer = Transfer::new(&self.0.local, Identity(current.source_revision), range)
            .map_err(|e| At::wrap(AdmissionError::Capacity(e)).at())?;
        Ok(Pending {
            transfer,
            account: self.0.clone(),
        })
    }
    pub fn bytes(&self) -> usize {
        self.0.local.occupancy().bytes
    }
}
impl Pending {
    pub fn append(&mut self, bytes: &[u8]) -> Result<(), Refusal> {
        self.transfer.append(bytes)
    }
    pub fn publish(self, current: Context) -> Result<Buffer, At<AdmissionError>> {
        self.account
            .plan
            .validate_start(current)
            .map_err(|e| e.map_error(AdmissionError::Plan))?;
        let lease = self
            .transfer
            .publish(Identity(current.source_revision))
            .map_err(|e| At::wrap(AdmissionError::Capacity(e)).at())?;
        Ok(Buffer {
            lease,
            _account: self.account,
        })
    }
}
impl Buffer {
    pub fn bytes(&self) -> &[u8] {
        self.lease.bytes()
    }
}
