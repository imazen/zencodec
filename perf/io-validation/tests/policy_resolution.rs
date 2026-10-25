//! Decision fixtures and reservation races; no large buffers are allocated.
//! Numeric costs are authored inputs, not measured codec memory requirements.
#![forbid(unsafe_code)]
#[path = "../src/admission.rs"]
mod admission;
#[path = "../src/policy_resolution.rs"]
mod policy_resolution;
use admission::{Limits, Refusal, Storage};
use policy_resolution::{Bound, Error, Offer, Request, Strategy, admit};

fn storage(bytes: usize) -> Storage {
    Storage::new(Limits {
        bytes,
        descriptors: 16,
        range_bytes: bytes,
        merge_gap: 0,
    })
}

fn request(memory_limit: usize, disk_limit: usize) -> Request {
    Request {
        memory_limit,
        disk_limit,
        require_copyright: true,
        require_gps_removal: true,
    }
}

fn offer(strategy: Strategy, memory: usize, disk: usize) -> Offer {
    Offer {
        strategy,
        memory: Bound::Upper(memory),
        disk: Bound::Upper(disk),
        retains_copyright: true,
        removes_gps: true,
    }
}

fn buffered_first(alternatives: &[&Offer]) -> Option<usize> {
    alternatives
        .iter()
        .position(|p| p.strategy == Strategy::Buffered)
        .or(Some(0))
}

#[test]
fn same_preference_chooses_different_paths_for_small_and_large_jobs() {
    let memory = storage(100);
    let disk = storage(0);
    for (raster, expected) in [(20, Strategy::Buffered), (200, Strategy::Sequential)] {
        let offers = [
            offer(Strategy::Sequential, 10, 0),
            offer(Strategy::Buffered, raster, 0),
        ];
        let plan = admit(
            &offers,
            request(100, 0),
            &memory,
            &disk,
            &mut buffered_first,
        )
        .unwrap();
        assert_eq!(plan.offer.strategy, expected);
        assert!(memory.occupancy().bytes <= 100);
        drop(plan);
        assert_eq!(memory.occupancy().bytes, 0);
    }
}

#[test]
fn streaming_label_does_not_override_whole_operation_retention() {
    let memory = storage(100);
    let disk = storage(50);
    // A streaming-shaped interface can retain large intermediate storage.
    // A DISTINCT disk-backed algorithm is assumed to have the smaller bound;
    // staging output alone cannot lower an encoder's required raw-memory peak.
    let offers = [
        offer(Strategy::Sequential, 200, 0),
        offer(Strategy::DiskStaged, 30, 40),
    ];
    let plan = admit(
        &offers,
        request(100, 50),
        &memory,
        &disk,
        &mut buffered_first,
    )
    .unwrap();
    assert_eq!(plan.offer.strategy, Strategy::DiskStaged);
    drop(plan);
    let mut called = false;
    let mut resolver = |_: &[&Offer]| {
        called = true;
        Some(0)
    };
    assert!(matches!(
        admit(&offers, request(100, 0), &memory, &disk, &mut resolver),
        Err(Error::NoAdmissiblePlan)
    ));
    assert!(
        !called,
        "a callback cannot conjure a feasible implementation"
    );
    assert_eq!(memory.occupancy().bytes, 0);
    assert_eq!(disk.occupancy().bytes, 0);
}

#[test]
fn an_already_resident_raster_cannot_be_subtracted_from_peak_memory() {
    let memory = storage(100);
    let disk = storage(100);
    // Every candidate includes the same already-resident caller input. Releasing
    // it earlier can reduce retention time, but cannot erase its peak occupancy.
    let offers = [
        offer(Strategy::Buffered, 200, 0),
        offer(Strategy::Sequential, 200, 0),
        offer(Strategy::DiskStaged, 200, 40),
    ];
    assert!(matches!(
        admit(
            &offers,
            request(100, 100),
            &memory,
            &disk,
            &mut buffered_first
        ),
        Err(Error::NoAdmissiblePlan)
    ));
    assert_eq!(memory.occupancy().bytes, 0);
    assert_eq!(disk.occupancy().bytes, 0);
}

#[test]
fn estimates_and_unknowns_cannot_satisfy_a_required_upper_bound() {
    let memory = storage(100);
    let disk = storage(100);
    for uncertain in [Bound::Estimate(1), Bound::Unknown] {
        for on_disk in [false, true] {
            let mut candidate = offer(Strategy::Buffered, 1, 0);
            if on_disk {
                candidate.disk = uncertain;
            } else {
                candidate.memory = uncertain;
            }
            assert!(matches!(
                admit(
                    &[candidate],
                    request(100, 100),
                    &memory,
                    &disk,
                    &mut buffered_first
                ),
                Err(Error::NoAdmissiblePlan)
            ));
            assert_eq!(memory.occupancy().bytes, 0);
        }
    }
}

#[test]
fn callback_only_sees_alternatives_that_satisfy_required_metadata() {
    let memory = storage(100);
    let disk = storage(0);
    let mut loses_rights = offer(Strategy::Buffered, 10, 0);
    loses_rights.retains_copyright = false;
    let mut leaks_gps = offer(Strategy::Buffered, 10, 0);
    leaks_gps.removes_gps = false;
    let offers = [loses_rights, leaks_gps, offer(Strategy::Sequential, 20, 0)];
    let mut resolver = |alternatives: &[&Offer]| {
        assert_eq!(alternatives.len(), 1);
        assert_eq!(alternatives[0].strategy, Strategy::Sequential);
        Some(0)
    };
    let plan = admit(&offers, request(100, 0), &memory, &disk, &mut resolver).unwrap();
    assert_eq!(plan.offer.strategy, Strategy::Sequential);
    drop(plan);

    let mut permissive = request(100, 0);
    permissive.require_copyright = false;
    let plan = admit(&offers, permissive, &memory, &disk, &mut buffered_first).unwrap();
    assert!(
        !plan.offer.retains_copyright,
        "authorized outcome stays inspectable"
    );
    assert!(plan.offer.removes_gps);
}

#[test]
fn callback_cannot_select_an_unoffered_plan_or_reserve_on_rejection() {
    let memory = storage(100);
    let disk = storage(0);
    let offers = [offer(Strategy::Buffered, 20, 0)];
    for (decision, expected) in [(None, Error::Declined), (Some(1), Error::InvalidChoice)] {
        let mut resolver = |_: &[&Offer]| decision;
        let result = admit(&offers, request(100, 0), &memory, &disk, &mut resolver);
        assert!(matches!(result, Err(error) if error == expected));
        assert_eq!(memory.occupancy().bytes, 0);
    }
}

#[test]
fn capacity_is_reserved_after_resolution_and_failed_disk_admission_rolls_back_ram() {
    for take_disk in [false, true] {
        let memory = storage(100);
        let disk = storage(100);
        let offers = [offer(Strategy::DiskStaged, 60, 60)];
        let mut other_job = None;
        let mut resolver = |_: &[&Offer]| {
            other_job = Some(if take_disk {
                disk.reserve(50).unwrap()
            } else {
                memory.reserve(50).unwrap()
            });
            Some(0)
        };
        assert!(matches!(
            admit(&offers, request(100, 100), &memory, &disk, &mut resolver),
            Err(Error::Capacity(Refusal::Bytes))
        ));
        assert_eq!(memory.occupancy().bytes, if take_disk { 0 } else { 50 });
        assert_eq!(disk.occupancy().bytes, if take_disk { 50 } else { 0 });
        drop(other_job);
        let plan = admit(
            &offers,
            request(100, 100),
            &memory,
            &disk,
            &mut buffered_first,
        )
        .unwrap();
        assert_eq!(memory.occupancy().bytes, 60);
        assert_eq!(disk.occupancy().bytes, 60);
        drop(plan);
        assert_eq!(memory.occupancy().bytes, 0);
        assert_eq!(disk.occupancy().bytes, 0);
    }
}
