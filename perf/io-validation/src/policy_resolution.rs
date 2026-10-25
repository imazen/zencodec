//! Private control-plane experiment, not a proposed public type inventory.
//! Offers contain authored logical bounds; this module does not certify a
//! codec's allocations, model RSS, or perform encoding/spooling.
use crate::admission::{Refusal, Reservation, Storage};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Strategy {
    Buffered,
    Sequential,
    DiskStaged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Bound {
    Upper(usize),
    Estimate(usize),
    Unknown,
}

impl Bound {
    fn upper(self) -> Option<usize> {
        match self {
            Self::Upper(n) => Some(n),
            Self::Estimate(_) | Self::Unknown => None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Offer {
    pub strategy: Strategy,
    /// Whole-operation logical peak, including retained caller storage.
    pub memory: Bound,
    pub disk: Bound,
    pub retains_copyright: bool,
    pub removes_gps: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct Request {
    pub memory_limit: usize,
    pub disk_limit: usize,
    pub require_copyright: bool,
    pub require_gps_removal: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Error {
    NoAdmissiblePlan,
    Declined,
    InvalidChoice,
    Allocation,
    Capacity(Refusal),
}

/// A callback can choose an offered alternative or refuse. It cannot rewrite
/// requirements, raise limits, or manufacture an offer. An index is local to
/// this synchronous invocation; this is not an async/reusable plan identifier.
pub(crate) trait PolicyResolver {
    fn choose(&mut self, alternatives: &[&Offer]) -> Option<usize>;
}

impl<F: FnMut(&[&Offer]) -> Option<usize>> PolicyResolver for F {
    fn choose(&mut self, alternatives: &[&Offer]) -> Option<usize> {
        self(alternatives)
    }
}

/// Reservation ownership is part of the result, not a detached boolean check.
pub(crate) struct Admitted<'a> {
    pub offer: &'a Offer,
    _memory: Reservation,
    _disk: Option<Reservation>,
}

pub(crate) fn admit<'a>(
    offers: &'a [Offer],
    request: Request,
    memory: &Storage,
    disk: &Storage,
    resolver: &mut dyn PolicyResolver,
) -> Result<Admitted<'a>, Error> {
    let mut alternatives = Vec::new();
    alternatives
        .try_reserve_exact(offers.len())
        .map_err(|_| Error::Allocation)?;
    alternatives.extend(offers.iter().filter(|offer| {
        offer
            .memory
            .upper()
            .is_some_and(|n| n <= request.memory_limit)
            && offer.disk.upper().is_some_and(|n| n <= request.disk_limit)
            && (!request.require_copyright || offer.retains_copyright)
            && (!request.require_gps_removal || offer.removes_gps)
    }));
    if alternatives.is_empty() {
        return Err(Error::NoAdmissiblePlan);
    }
    let index = resolver.choose(&alternatives).ok_or(Error::Declined)?;
    let offer = *alternatives.get(index).ok_or(Error::InvalidChoice)?;
    // Never trust a capacity snapshot from before the callback. Reserve before
    // execution; partial admission is rolled back if the disk reservation fails.
    let memory = memory
        .reserve(offer.memory.upper().unwrap())
        .map_err(Error::Capacity)?;
    let disk_bytes = offer.disk.upper().unwrap();
    let disk = if disk_bytes == 0 {
        None
    } else {
        Some(disk.reserve(disk_bytes).map_err(Error::Capacity)?)
    };
    Ok(Admitted {
        offer,
        _memory: memory,
        _disk: disk,
    })
}
