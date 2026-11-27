//! Contract tests for PR 1 core additions: `StorageReservation`, `JobResources`,
//! `SourceAccess`/`NativeBound`, strict `InputRequest::Range`, and multi-provider
//! typed roles on `CodecSet`.

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use enough::{Stop, StopReason};
use zencodec::decode::{DecodePolicy, Input, InputRequest};
use zencodec::encode::EncodePolicy;
use zencodec::how_far::{Execution, PhaseSpec, Report, StopOnly, Total};
use zencodec::source::{AccessPattern, Acquisition, ByteOffer, NativeBound, SourceAccess};
use zencodec::storage::{StorageBudget, StorageError};
use zencodec::{
    AllocPreference, CodecSet, EncodedTransformFactory, EncodedTransformRole, FactoryRole,
    ImageFormat, JobResources, LimitExceeded, NamedFactory, Pulse, RegistrationError,
    RegistrationId, ResourceLimits, SharedPulse, ThreadingPolicy,
};

#[test]
fn storage_reserve_backing_enforces_caps_and_releases_on_drop() {
    let budget = StorageBudget::new(1024, 3);
    let r1 = budget
        .reserve_backing(600, NonZeroUsize::new(2).unwrap())
        .unwrap();
    assert_eq!(r1.reserved_bytes(), 600);
    assert_eq!(r1.reserved_objects(), 2);
    assert_eq!(budget.reserved_bytes(), 600);
    assert_eq!(budget.reserved_objects(), 2);

    // Exceeding byte cap fails without charging.
    let err = budget
        .reserve_backing(500, NonZeroUsize::new(1).unwrap())
        .unwrap_err();
    assert!(matches!(err, StorageError::Bytes { .. }));
    assert_eq!(budget.reserved_bytes(), 600);
    assert_eq!(budget.reserved_objects(), 2);

    // Exceeding object cap fails without charging.
    let err = budget
        .reserve_backing(100, NonZeroUsize::new(2).unwrap())
        .unwrap_err();
    assert!(matches!(err, StorageError::Objects { .. }));
    assert_eq!(budget.reserved_bytes(), 600);
    assert_eq!(budget.reserved_objects(), 2);

    // Remaining exact headroom succeeds.
    let r2 = budget
        .reserve_backing(424, NonZeroUsize::new(1).unwrap())
        .unwrap();
    assert_eq!(budget.reserved_bytes(), 1024);
    assert_eq!(budget.reserved_objects(), 3);

    drop(r1);
    assert_eq!(budget.reserved_bytes(), 424);
    assert_eq!(budget.reserved_objects(), 1);

    drop(r2);
    assert_eq!(budget.reserved_bytes(), 0);
    assert_eq!(budget.reserved_objects(), 0);
}

struct ReasonStop {
    tripped: Arc<AtomicBool>,
    reason: StopReason,
}
impl Stop for ReasonStop {
    fn check(&self) -> Result<(), StopReason> {
        if self.tripped.load(Ordering::SeqCst) {
            Err(self.reason)
        } else {
            Ok(())
        }
    }
}

struct MeterPulse {
    tripped: Arc<AtomicBool>,
    reason: StopReason,
    units: Arc<AtomicU64>,
}
impl Stop for MeterPulse {
    fn check(&self) -> Result<(), StopReason> {
        if self.tripped.load(Ordering::SeqCst) {
            Err(self.reason)
        } else {
            Ok(())
        }
    }
}
impl Report for MeterPulse {
    fn advance(&self, n: u64) {
        self.units.fetch_add(n, Ordering::SeqCst);
    }
}
impl Pulse for MeterPulse {
    fn split(
        &self,
        e: Execution,
        parts: &[PhaseSpec<'_>],
    ) -> Result<Vec<zencodec::how_far::Child<'_>>, zencodec::how_far::PlanError> {
        zencodec::NoPulse.split(e, parts)
    }
    fn share(&self) -> Result<SharedPulse, zencodec::how_far::PlanError> {
        Ok(SharedPulse::new(Self {
            tripped: Arc::clone(&self.tripped),
            reason: self.reason,
            units: Arc::clone(&self.units),
        }))
    }
}

#[test]
fn job_resources_counters_tightening_alloc_merge_and_composed_pulse() {
    let host_tripped = Arc::new(AtomicBool::new(false));
    let host_units = Arc::new(AtomicU64::new(0));
    let host_pulse = SharedPulse::new(MeterPulse {
        tripped: Arc::clone(&host_tripped),
        reason: StopReason::TimedOut,
        units: Arc::clone(&host_units),
    });
    let host_limits = ResourceLimits::none()
        .with_max_width(4096)
        .with_max_input_bytes(1000)
        .with_max_output(500)
        .with_max_frames(10)
        .with_max_total_pixels(10_000)
        .with_prefer_fallible_allocations(AllocPreference::CodecDefault);
    let budget = StorageBudget::new(4096, 8);
    let host = JobResources::new(host_limits, budget, host_pulse);

    // Tighten may only narrow limits and shares counters/storage/pulse.
    let mut loose_or_tight = ResourceLimits::none()
        .with_max_width(8192) // looser than host -> host 4096 must win
        .with_max_height(2048) // new tighter bound -> adopted
        .with_max_input_bytes(600) // tighter -> adopted
        .with_max_output(2000) // looser -> host 500 must win
        .with_prefer_fallible_allocations(AllocPreference::Infallible); // looser -> CodecDefault wins
    loose_or_tight.threading = ThreadingPolicy::Sequential;
    let derived = host.tighten_limits(loose_or_tight);

    assert_eq!(derived.limits().max_width, Some(4096));
    assert_eq!(derived.limits().max_height, Some(2048));
    assert_eq!(derived.limits().max_input_bytes, Some(600));
    assert_eq!(derived.limits().max_output_bytes, Some(500));
    assert_eq!(derived.limits().threading, ThreadingPolicy::Sequential);
    assert_eq!(
        derived.limits().prefer_fallible_allocations,
        AllocPreference::CodecDefault
    );

    // Fallible beats CodecDefault and Infallible monotonically.
    let fallible = derived.tighten_limits(
        ResourceLimits::none().with_prefer_fallible_allocations(AllocPreference::Fallible),
    );
    assert_eq!(
        fallible.limits().prefer_fallible_allocations,
        AllocPreference::Fallible
    );
    let still_fallible = fallible.tighten_limits(
        ResourceLimits::none().with_prefer_fallible_allocations(AllocPreference::Infallible),
    );
    assert_eq!(
        still_fallible.limits().prefer_fallible_allocations,
        AllocPreference::Fallible
    );

    // Shared atomic counters observe updates across host and derived handles,
    // while tightened numeric caps stay handle-local.
    assert_eq!(derived.record_input_bytes(400).unwrap(), 400);
    assert_eq!(host.recorded_input_bytes(), 400);
    assert!(matches!(
        derived.record_input_bytes(201),
        Err(LimitExceeded::InputSize {
            actual: 601,
            max: 600
        })
    ));
    // Failed charge leaves counter untouched.
    assert_eq!(derived.recorded_input_bytes(), 400);
    // Overflow is rejected without mutating the counter.
    assert!(matches!(
        host.record_input_bytes(u64::MAX),
        Err(LimitExceeded::InputSize { .. })
    ));
    assert_eq!(host.recorded_input_bytes(), 400);

    assert_eq!(derived.record_output_bytes(500).unwrap(), 500);
    assert!(matches!(
        derived.record_output_bytes(1),
        Err(LimitExceeded::OutputSize {
            actual: 501,
            max: 500
        })
    ));

    assert_eq!(derived.record_frames(10).unwrap(), 10);
    assert!(matches!(
        derived.record_frames(1),
        Err(LimitExceeded::Frames {
            actual: 11,
            max: 10
        })
    ));

    assert_eq!(derived.record_pixels(10_000).unwrap(), 10_000);
    assert!(matches!(
        derived.record_pixels(1),
        Err(LimitExceeded::TotalPixels {
            actual: 10_001,
            max: 10_000
        })
    ));

    // Composing a default/child pulse NEVER drops host cancellation, preserves host
    // StopReason, forwards progress, and survives split() & share().
    let child_tripped = Arc::new(AtomicBool::new(false));
    let child_pulse = SharedPulse::new(StopOnly::new(ReasonStop {
        tripped: Arc::clone(&child_tripped),
        reason: StopReason::Cancelled,
    }));
    let with_default = host.with_child_pulse(SharedPulse::default());
    let with_child = host.with_child_pulse(child_pulse);
    let shared_child_pulse = with_default.pulse().share().unwrap();
    let split_children = with_default
        .pulse()
        .split(
            Execution::Unspecified,
            &[PhaseSpec::new("stage", 1, Total::Exact(10))],
        )
        .unwrap();

    with_default.pulse().advance(7);
    split_children[0].advance(3);
    assert_eq!(host_units.load(Ordering::SeqCst), 10);

    // Child-only trip stops derived handle without cancelling host.
    child_tripped.store(true, Ordering::SeqCst);
    assert_eq!(with_child.check_stop(), Err(StopReason::Cancelled));
    assert!(host.check_stop().is_ok());
    assert!(with_default.check_stop().is_ok());
    child_tripped.store(false, Ordering::SeqCst);

    // Host-only trip stops EVERY derived handle (including SharedPulse::default(),
    // shared clones, and split children) with host's exact StopReason::TimedOut!
    host_tripped.store(true, Ordering::SeqCst);
    assert_eq!(host.check_stop(), Err(StopReason::TimedOut));
    assert_eq!(derived.check_stop(), Err(StopReason::TimedOut));
    assert_eq!(with_default.check_stop(), Err(StopReason::TimedOut));
    assert_eq!(with_child.check_stop(), Err(StopReason::TimedOut));
    assert_eq!(shared_child_pulse.check(), Err(StopReason::TimedOut));
    assert_eq!(split_children[0].check(), Err(StopReason::TimedOut));
}

#[test]
fn source_access_native_bound_and_strict_range_eof() {
    let borrowed = SourceAccess::borrowed(128);
    assert_eq!(borrowed.acquisition, Acquisition::Borrowed);
    assert!(borrowed.immutable);
    assert!(borrowed.satisfies(AccessPattern::RandomAccess));
    assert!(borrowed.satisfies(AccessPattern::Sequential));

    let forward = SourceAccess::forward_stream();
    assert_eq!(forward.acquisition, Acquisition::ForwardStream);
    assert!(forward.satisfies(AccessPattern::Sequential));
    assert!(!forward.satisfies(AccessPattern::RandomAccess));
    assert!(!forward.satisfies(AccessPattern::BoundedBacktrack { window_bytes: 16 }));

    let offer = ByteOffer::new(10, 20, Some(100));
    assert_eq!(
        (offer.offset, offer.len, offer.total_len),
        (10, 20, Some(100))
    );

    assert_eq!(NativeBound::Exact(64u64).known(), Some(64));
    assert_eq!(NativeBound::UpperBound(128u64).known(), Some(128));
    assert_eq!(NativeBound::<u64>::Unknown.known(), None);
    assert!(NativeBound::<u64>::Unknown.is_unknown());

    // Exact InputRequest::Range MUST NOT match a short slice even when total_len reaches EOF.
    let req = InputRequest::range(10, 8);
    let short_at_eof = Input::new(10, &[1, 2, 3, 4]).with_total_len(14);
    assert!(
        !req.is_satisfied_by(&short_at_eof),
        "short slice at EOF must not satisfy exact Range(10, 8)"
    );
    let full = Input::new(10, &[1, 2, 3, 4, 5, 6, 7, 8]).with_total_len(18);
    assert!(req.is_satisfied_by(&full));
}

#[test]
fn registration_id_validation() {
    assert!(RegistrationId::new("org.imazen.jpeg.zenjpeg").is_ok());
    assert!(RegistrationId::new("a1.b_2.c-3").is_ok());
    assert!(RegistrationId::new("x").is_ok());
    assert!(RegistrationId::new(&"a".repeat(128)).is_ok());

    for bad in [
        "",
        ".leading",
        "trailing.",
        "double..dot",
        "Upper.case",
        "bad.-dash",
        "bad._under",
        "space in.id",
        "slash/id",
        &"a".repeat(129),
    ] {
        assert_eq!(
            RegistrationId::new(bad),
            Err(RegistrationError::InvalidId),
            "should reject `{bad}`"
        );
    }
}

struct MockTransform(&'static str);
impl EncodedTransformFactory for MockTransform {
    fn describe(&self) -> &'static str {
        self.0
    }
    fn transform_bytes(
        &self,
        input: &[u8],
        _pulse: Option<&dyn Pulse>,
    ) -> Result<Vec<u8>, zencodec::decode::BoxedError> {
        Ok(input.to_vec())
    }
}

enum CustomRole {}
impl FactoryRole for CustomRole {
    type Key = &'static str;
    type Factory = dyn Fn() -> u32 + Send + Sync;
}

#[test]
fn codec_set_multi_provider_roles_dedup_keys_and_operation_defaults_flag() {
    let mut set = CodecSet::new();
    assert!(!set.has_operation_defaults());
    assert!(
        set.clone()
            .with_limits(ResourceLimits::none())
            .has_operation_defaults()
    );
    assert!(
        set.clone()
            .with_pulse(SharedPulse::default())
            .has_operation_defaults()
    );
    assert!(
        set.clone()
            .with_decode_policy(DecodePolicy::none())
            .has_operation_defaults()
    );
    assert!(
        set.clone()
            .with_encode_policy(EncodePolicy::none())
            .has_operation_defaults()
    );

    let f1 = NamedFactory::<EncodedTransformRole>::try_new(
        "org.imazen.jpeg.lossless_a",
        "0.1.0",
        Arc::new(MockTransform("a")),
    )
    .unwrap();
    let f2 = NamedFactory::<EncodedTransformRole>::try_new(
        "org.imazen.jpeg.lossless_b",
        "0.2.0",
        Arc::new(MockTransform("b")),
    )
    .unwrap();
    let dup = NamedFactory::<EncodedTransformRole>::try_new(
        "org.imazen.jpeg.lossless_a",
        "0.3.0",
        Arc::new(MockTransform("dup")),
    )
    .unwrap();

    // Empty keys rejected.
    assert_eq!(
        set.clone()
            .with_role::<EncodedTransformRole>(core::iter::empty(), f1.clone())
            .unwrap_err(),
        RegistrationError::EmptyKeys
    );

    // Repeated keys in a single registration are deduplicated.
    set = set
        .with_role::<EncodedTransformRole>(
            [
                (ImageFormat::Jpeg, ImageFormat::Jpeg),
                (ImageFormat::Jpeg, ImageFormat::Jpeg),
            ],
            f1,
        )
        .unwrap()
        .with_role::<EncodedTransformRole>([(ImageFormat::Jpeg, ImageFormat::Jpeg)], f2)
        .unwrap();

    // Duplicate ID in the same role rejected.
    let err = set
        .clone()
        .with_role::<EncodedTransformRole>([(ImageFormat::Png, ImageFormat::Png)], dup)
        .unwrap_err();
    assert!(
        matches!(err, RegistrationError::DuplicateId(id) if id.as_str() == "org.imazen.jpeg.lossless_a")
    );

    // Lookup returns both providers once each in deterministic registration order.
    let hits: Vec<_> = set
        .factories::<EncodedTransformRole>(&(ImageFormat::Jpeg, ImageFormat::Jpeg))
        .collect();
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].id.as_str(), "org.imazen.jpeg.lossless_a");
    assert_eq!(hits[0].factory.describe(), "a");
    assert_eq!(hits[1].id.as_str(), "org.imazen.jpeg.lossless_b");
    assert_eq!(hits[1].factory.describe(), "b");

    // Same RegistrationId in a different role is allowed (role-scoped uniqueness).
    let custom = NamedFactory::<CustomRole>::try_new(
        "org.imazen.jpeg.lossless_a",
        "1.0.0",
        Arc::new(|| 42u32),
    )
    .unwrap();
    set = set.with_role::<CustomRole>(["key"], custom).unwrap();
    let custom_hits: Vec<_> = set.factories::<CustomRole>(&"key").collect();
    assert_eq!(custom_hits.len(), 1);
    assert_eq!((custom_hits[0].factory)(), 42);
}
