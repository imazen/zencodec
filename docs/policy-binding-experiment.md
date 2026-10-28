# One policy binding, with a replaceable callback

This is a tested API experiment, **not a production API**. The complete runnable
implementation is in [`policy_binding.rs`](../zencodec-testkit/tests/policy_binding.rs).
Run `just policy-binding-check`. It uses the production admission engine; the
binding itself is private to the test.

## Start with a required output bound

```rust
let mut policy = BoundPolicy::new(
    PolicySet::new().with_output_wait_limit(8),
    preset, // returns Decision::UsePreset
);

// Finish-first alone does not prove a memory bound during destination waits.
let result = policy.schedule(&[
    OutputScheduleOffer::new(OutputSchedule::FinishFirst),
]);
// Refuses: OutputWait(Unknown), before invoking the callback.
```

The eight-byte limit is a small test value. It is not a recommended memory budget
or a measurement of any native codec. The offer must establish the required bound;
choosing finish-first does not establish it automatically.

## Add application logic without restating the requirement

```rust
let mut policy = BoundPolicy::new(
    PolicySet::new().with_output_wait_limit(8),
    |question: Question<'_>| -> Result<Decision, At<Infallible>> {
        let Question::OutputSchedule { choices, .. } = question else {
            panic!("this experiment exercises output scheduling only")
        };
        Ok(Decision::choose(choices.first().unwrap()))
    },
);
```

The test supplies three original offers: unknown retention, nine bytes, and eight
bytes. The callback receives only the last offer. Its token still selects original
index 2, and the record retains both excluded alternatives' reasons. Application
selection cannot bypass admission.

## Add logging around the callback

```rust
struct BoundPolicy<R> {
    requirements: PolicySet,
    resolver: R,
}

impl<R: PolicyResolver> BoundPolicy<R> {
    // Operation entry points use both fields together.
}

// A logging wrapper forwards resolve(), with no policy-forwarding hook.
let mut policy = BoundPolicy::new(required_set, Logged { inner: preset, calls });
```

The binding deliberately does **not** implement `PolicyResolver`. Operations take
the binding, while logging wraps only its callback. A proposed optional
`PolicyResolver::policy_set()` hook would depend on every wrapper forwarding that
new method. Keeping the requirement set outside the callback avoids that extra
obligation. This design has not yet been adopted by the production entry points.

## Keep application errors inspectable

The callback still returns `Result<Decision, At<E>>`. The experiment returns a
typed tenant error, checks `DecisionCode::CallbackFailed`, and finds the original
cause through `zencodec::find_cause`. Default error formatting excludes the private
tenant message. Binding policy must not stringify errors or turn them into an
ordinary policy refusal.

## The unresolved part: decode, process, encode

```rust
policy.boundary(InterpretationState::Pending)?; // discovery may continue
policy.boundary(InterpretationState::Final)?;  // refuses an unproven output rule
```

A decoder's adoption boundary can authorize sink output. An eventual checked
encoder is not proof that those intermediate effects are contained. The experiment
retains that refusal. A complete pipeline contract must establish who owns
intermediate rows, which effects adoption enables, and where each deferred
obligation is discharged before publication. Merely ignoring output requirements
on decode questions would weaken the existing guarantee.

Five controls pass: wrapped-callback admission, original-index selection, named
schedule refusal, discovery versus final adoption, and typed application errors.
Strict Clippy also passes. The build/test receipt is `rc=0`, 3 seconds, peak RSS
0.30 GiB, minimum available memory 6448 MiB and peak load 0.64. These are resource
readings for this check, not runtime policy overhead measurements.

Missing before promotion: a production binding accepted by all operation entry
points; removal/deprecation of duplicate preset placement; a qualified intermediate
ownership contract; native decode/process/encode validation; and performance
qualification. The production two-placement diagnostic gap remains open in
[issue 173](https://github.com/imazen/zencodec/issues/173).
