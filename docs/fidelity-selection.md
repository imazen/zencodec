# Checked fidelity target selection

`with_fidelity` remains best-effort. `try_with_fidelity` accepts a target only
when `resolved_target_fidelity` confirms the same variant and value; otherwise
it returns `FidelityTargetError::Substituted` or `Unreported`. Non-finite target
values are rejected before configuration. `check_fidelity_target` performs the
same query on a clone without changing the original config.

```rust,ignore
let preferences = [Fidelity::Lossless, Fidelity::butteraugli(1.0),
                   Fidelity::codec_quality(85.0)];
let (index, config) = config.select_fidelity(&preferences)?;
// Apply metadata, stop and resource limits to config.job(), then encode once.
```

The index identifies the selected caller preference. If none is accepted,
`FidelitySelectionError::Rejected` preserves every index, target and reason.
Empty lists, non-finite entries anywhere in the list, and allocation failures
are distinct errors. Each candidate starts from the original config.

For erased configurations, use `check_fidelity_target`, `try_fidelity_job`, or
`select_fidelity_job`. The returned job still accepts metadata, policy, limits
and cancellation. This selection never retries an encode after failure.

This is configuration-level confirmation, not a content admission or delivery
report. Lossless encoding still depends on the input descriptor, alpha, content
and backend; approximate perceptual targets remain approximate. A successful
configuration query cannot certify pixels it has never seen. Native quality is
codec-specific. Substitution to lossless is still a substitution: list it
explicitly when acceptable. The next breaking contract should express required
sample fidelity at encode time, as recorded in the breaking queue.
