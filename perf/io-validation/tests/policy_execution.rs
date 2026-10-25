#![forbid(unsafe_code)]
#[path = "../src/admission.rs"]
mod admission;
#[path = "../src/policy_execution.rs"]
mod execution;
#[path = "../src/policy/mod.rs"]
mod policy;
use admission::{Limits, Storage};
use execution::*;
use policy::plan::*;
use policy::*;
use zencodec::{Unstoppable, decode::ByteRange, encode::Fidelity};

fn storage(bytes: usize) -> Storage {
    Storage::new(Limits {
        bytes,
        descriptors: 64,
        range_bytes: bytes,
        merge_gap: 0,
    })
}
fn context() -> Context {
    Context {
        source_revision: 4,
        plan_revision: 1,
        epoch: 2,
        replayable: true,
        published_bytes: 0,
    }
}
fn req() -> Requirements {
    Requirements {
        ram: Some(64),
        disk: Some(8),
        accounting: Accounting::Driver,
        effects: Effects::LOSSLESS,
    }
}
fn choices(engine: &mut Engine, req: Requirements, context: Context) -> Vec<Selected> {
    [
        (
            Facts::Execution(ExecutionFacts {
                resident_input: 0,
                source_length: None,
                network: true,
            }),
            Action::Execution(Execution::DiskStaged),
        ),
        (
            Facts::Metadata(MetadataFacts {
                packet_bytes: 0,
                parse_complete: true,
                has_location: Some(false),
                has_rights: Some(false),
            }),
            Action::Metadata(MetadataAction::DropDescriptive),
        ),
        (
            Facts::Fidelity(FidelityFacts {
                alpha: true,
                bits_per_sample: 8,
            }),
            Action::Fidelity(Fidelity::Lossless),
        ),
    ]
    .into_iter()
    .map(|(facts, action)| {
        let execution = matches!(action, Action::Execution(_));
        engine
            .resolve(
                facts,
                &[Alternative {
                    action,
                    ram: Evidence::Enforced(if execution { 32 } else { 8 }),
                    disk: Evidence::Upper(if execution { 8 } else { 0 }),
                    accounting: Accounting::Driver,
                    effects: if matches!(action, Action::Fidelity(_)) {
                        Effects::LOSSLESS
                    } else {
                        Effects(0)
                    },
                    needs_replay: false,
                    preserves_published: true,
                }],
                req,
                context,
                &mut PolicySet::default(),
                &Unstoppable,
            )
            .unwrap()
    })
    .collect()
}
fn plan() -> Plan {
    let mut engine = Engine::new(PolicySet::default(), 8);
    Plan::combine(&choices(&mut engine, req(), context()), context()).unwrap()
}

#[test]
fn joint_plan_rejects_cross_operation_request_and_context_mixing() {
    let mut engine = Engine::new(PolicySet::default(), 32);
    let selected = choices(&mut engine, req(), context());
    let plan = Plan::combine(&selected, context()).unwrap();
    assert_eq!((plan.ram, plan.disk), (48, 8));
    assert_eq!(
        *Plan::combine(&selected[..2], context())
            .unwrap_err()
            .error(),
        PlanError::Missing
    );
    let mut duplicate = selected.clone();
    duplicate.push(selected[0]);
    assert_eq!(
        *Plan::combine(&duplicate, context()).unwrap_err().error(),
        PlanError::Duplicate
    );
    let mut mixed = selected.clone();
    mixed[1] = choices(&mut Engine::new(PolicySet::default(), 8), req(), context())[1];
    assert_eq!(
        *Plan::combine(&mixed, context()).unwrap_err().error(),
        PlanError::ForeignOperation
    );
    let different = Requirements {
        ram: Some(128),
        ..req()
    };
    mixed[1] = choices(&mut engine, different, context())[1];
    assert_eq!(
        *Plan::combine(&mixed, context()).unwrap_err().error(),
        PlanError::ChangedRequest
    );
    for changed in [
        Context {
            epoch: 3,
            ..context()
        },
        Context {
            source_revision: 5,
            ..context()
        },
        Context {
            plan_revision: 2,
            ..context()
        },
        Context {
            published_bytes: 1,
            ..context()
        },
    ] {
        assert_eq!(
            *Plan::combine(&selected, changed).unwrap_err().error(),
            PlanError::Stale
        );
        assert_eq!(
            *plan.validate_start(changed).unwrap_err().error(),
            PlanError::Stale
        );
    }
}

#[test]
fn individual_choices_do_not_establish_a_combined_bound() {
    let req = Requirements {
        ram: Some(40),
        ..req()
    };
    let selected = choices(&mut Engine::new(PolicySet::default(), 8), req, context());
    assert_eq!(
        *Plan::combine(&selected, context()).unwrap_err().error(),
        PlanError::Ram
    );
    let mut unknown = selected.clone();
    unknown[0].alternative.ram = Evidence::Unknown;
    assert_eq!(
        *Plan::combine(&unknown, context()).unwrap_err().error(),
        PlanError::UnprovenCost
    );
    unknown[0].alternative.ram = Evidence::Upper(u64::MAX);
    assert_eq!(
        *Plan::combine(&unknown, context()).unwrap_err().error(),
        PlanError::Overflow
    );
}

#[test]
fn partial_admission_rolls_back_and_competing_reservations_refuse() {
    let memory = storage(48);
    let disk = storage(8);
    let competing = disk.reserve(1).unwrap();
    assert!(matches!(
        Admitted::start(plan(), context(), &memory, &disk, &Unstoppable)
            .err()
            .unwrap()
            .error(),
        AdmissionError::Capacity(admission::Refusal::Bytes)
    ));
    assert_eq!(memory.occupancy().bytes, 0);
    drop(competing);
    let admitted = Admitted::start(plan(), context(), &memory, &disk, &Unstoppable).unwrap();
    assert!(Admitted::start(plan(), context(), &memory, &disk, &Unstoppable).is_err());
    drop(admitted);
    assert_eq!((memory.occupancy().bytes, disk.occupancy().bytes), (0, 0));
}

#[test]
fn actual_growth_checks_caps_and_keeps_charges_until_last_owner() {
    let memory = storage(48);
    let disk = storage(8);
    let admitted = Admitted::start(plan(), context(), &memory, &disk, &Unstoppable).unwrap();
    let mut pending = admitted
        .allocate(ByteRange::new(0, 48), context(), &Unstoppable)
        .unwrap();
    assert_eq!(admitted.bytes(), 48);
    assert!(
        admitted
            .allocate(ByteRange::new(48, 1), context(), &Unstoppable)
            .is_err()
    );
    pending.append(&[7; 48]).unwrap();
    let buffer = pending.publish(context()).unwrap();
    let retained = buffer.clone();
    drop(admitted);
    drop(buffer);
    assert_eq!(retained.bytes(), &[7; 48]);
    assert_eq!((memory.occupancy().bytes, disk.occupancy().bytes), (48, 8));
    drop(retained);
    assert_eq!((memory.occupancy().bytes, disk.occupancy().bytes), (0, 0));
}

#[test]
fn incomplete_and_stale_transfers_release_their_storage() {
    let memory = storage(48);
    let disk = storage(8);
    let admitted = Admitted::start(plan(), context(), &memory, &disk, &Unstoppable).unwrap();
    let pending = admitted
        .allocate(ByteRange::new(0, 48), context(), &Unstoppable)
        .unwrap();
    assert!(pending.publish(context()).is_err());
    assert_eq!(admitted.bytes(), 0);
    let pending = admitted
        .allocate(ByteRange::new(0, 48), context(), &Unstoppable)
        .unwrap();
    assert!(
        pending
            .publish(Context {
                epoch: 3,
                ..context()
            })
            .is_err()
    );
    assert_eq!(admitted.bytes(), 0);
    let pending = admitted
        .allocate(ByteRange::new(0, 48), context(), &Unstoppable)
        .unwrap();
    drop(pending); // Cancellation by dropping the in-flight transfer.
    assert_eq!(admitted.bytes(), 0);
    drop(admitted);
    assert_eq!(memory.occupancy().bytes, 0);
}

#[test]
fn failure_evidence_has_exact_bounds_and_no_payloads() {
    let mut engine = Engine::new(PolicySet::default(), 8);
    let req = Requirements {
        ram: Some(2),
        ..req()
    };
    let offers = [Alternative {
        action: Action::Metadata(MetadataAction::KeepExact),
        ram: Evidence::Upper(10),
        disk: Evidence::Unknown,
        accounting: Accounting::Driver,
        effects: Effects(0),
        needs_replay: false,
        preserves_published: true,
    }];
    let error = engine
        .resolve(
            Facts::Metadata(MetadataFacts {
                packet_bytes: 10,
                parse_complete: false,
                has_location: None,
                has_rights: None,
            }),
            &offers,
            req,
            context(),
            &mut PolicySet::default(),
            &Unstoppable,
        )
        .unwrap_err();
    let details = error.error().details().unwrap();
    assert_eq!(details.requirements.ram, Some(2));
    assert_eq!(details.offered[0].ram, Evidence::Upper(10));
    assert_eq!(details.omitted, 0);
    assert_eq!(error.error().record().rejected_reasons, 3);
    assert_eq!(
        details.rejections(context()).collect::<Vec<_>>(),
        [
            (
                0,
                RejectedBy::Ram {
                    cost: Evidence::Upper(10),
                    limit: 2
                }
            ),
            (
                0,
                RejectedBy::Disk {
                    cost: Evidence::Unknown,
                    limit: 8
                }
            ),
        ]
    );
}

#[test]
fn simultaneous_plans_cannot_both_spend_the_same_capacity() {
    use std::sync::{Arc, Barrier};
    let memory = storage(48);
    let disk = storage(8);
    let barrier = Arc::new(Barrier::new(2));
    let results = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..2)
            .map(|_| {
                scope.spawn(|| {
                    let plan = plan();
                    barrier.wait();
                    let result = Admitted::start(plan, context(), &memory, &disk, &Unstoppable);
                    barrier.wait(); // Keep the winning reservation alive until both tried.
                    result.is_ok()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(results.iter().filter(|v| **v).count(), 1);
    assert_eq!((memory.occupancy().bytes, disk.occupancy().bytes), (0, 0));
}

#[test]
fn cancellation_prevents_admission_and_each_later_growth() {
    struct Cancel;
    impl zencodec::enough::Stop for Cancel {
        fn check(&self) -> Result<(), zencodec::enough::StopReason> {
            Err(zencodec::enough::StopReason::Cancelled)
        }
    }
    let memory = storage(48);
    let disk = storage(8);
    assert!(matches!(
        Admitted::start(plan(), context(), &memory, &disk, &Cancel)
            .err()
            .unwrap()
            .error(),
        AdmissionError::Stopped(_)
    ));
    assert_eq!(memory.occupancy().bytes, 0);
    let admitted = Admitted::start(plan(), context(), &memory, &disk, &Unstoppable).unwrap();
    assert!(matches!(
        admitted
            .allocate(ByteRange::new(0, 1), context(), &Cancel)
            .err()
            .unwrap()
            .error(),
        AdmissionError::Stopped(_)
    ));
    assert_eq!(admitted.bytes(), 0);
}
