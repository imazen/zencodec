# One retained operation Pulse

```rust,ignore
// shared: SharedPulse; data: Vec<u8>; preferred: &[PixelDescriptor]
let job = config.job().with_pulse(shared);
let mut frames = job.animation_frame_decoder(Cow::Owned(data), preferred)?;
while let Some(frame) = frames.render_next_frame(None)? {
    consume(frame)?;
}
```

The job setter retains the operation's cancellation, progress, phase planning
and sharing interfaces. A session created from that job must retain its control;
`None` supplies no additional per-call control. It does not disable the job's
deadline. Codecs still need individual qualification: a Pulse-shaped signature
does not establish that an adapter passed the control to the native work.

```rust,ignore
// frame_deadline: &dyn Pulse
let next = frames.render_next_frame(Some(frame_deadline))?;
```

The current animation trait documentation specifies cooperative cancellation
for the per-call parameter. It does not specify a portable destination or unit
for per-call progress reports. Native IMI currently reports frames to the job
pulse and uses the per-call pulse for cancellation. Keep that distinction
explicit until the shared progress-composition contract is qualified. Retaining
a Pulse also does not guarantee that a native codec implements phase splitting
or reports fractional progress.

## Cancellation-only callers use the same entry point

```rust,ignore
let control = SharedPulse::new(how_far::StopOnly::new(stop));
let job = config.job().with_pulse(control);
```

Construct the desired control before setting it on the job. The deprecated
`with_stop` default delegates to `with_pulse(StopOnly(...))`; it replaces a
previously installed reporting pulse. Existing native overrides can instead
combine the old and new controls. Chaining those two setters therefore has
codec-specific legacy behavior. Migration must preserve established controls
and document the difference; changing it silently would lose cancellation or
progress for existing callers.

An adapter may borrow a retained Pulse as `Stop` when calling a native
cancellation-only primitive. Storing only that reduced interface loses the
operation's progress capabilities. Conversely, storing the same Pulse both as
the native pulse and inside a separate stop token can poll it twice. A
`may_report() == false` optimization can omit work needed only for progress;
it must retain cancellation checks.

## Native evidence and open gates

- [GIF PR32](https://github.com/imazen/zengif/pull/32), head `efb7e43b`,
  passed its [12 CI jobs](https://github.com/imazen/zengif/actions/runs/40673732867).
  The local migration control compared cancellation polls across 11 operations
  with the previous head and found identical counts. It does not establish
  timing performance or add native progress reporting.
- The WebP lane executed its local control against
  [PR105's baseline `872289ff`](https://github.com/imazen/zenwebp/tree/872289ffd22065f02a58d3141feba57cf67b4a5f).
  The export repairs a dependency repository URL without changing its revision
  or native codec code.
  Lossy decode observed 6 checks with a legacy token and 11 with a quiet Pulse;
  animation rendering observed 99 and 193. Native still encoding observed 24
  stop checks versus 28 pulse checks, while the adapter's quiet Pulse was checked
  56 times. These are fixture-specific poll counts, not timings or universal
  per-frame costs. The proposed fix is not yet qualified.
- The same WebP control found a separate diagnostic problem: a timeout at
  legacy decode poll 5 of 6 returned `Cancelled` instead of `TimedOut`.
  Preserve the expected timeout reason when repairing the path; removing
  duplicate polls alone does not certify cancellation correctness.
- The zenpipe lane found an animation request path that did not call
  `set_pulse` on its retained codec job. Its new failing-first control initially
  failed to compile, so the behavioral witness and fix remain unqualified.
  Source inspection alone is not a passing regression test.

The WebP and zenpipe controls above are local work under review; their final
commits and complete passing receipts must be recorded before closing the
qualification gate. Existing pixel and timing assertions remain unchanged.
The [probe receipt](../benchmarks/pulse_review_2026-10-28.json) retains the
baseline, local control commit, exact command, observed counts and failed-run
resource receipt. That build/test receipt is not a native performance result.

For each route, test job-only and per-call-only cancellation, quiet and reporting
pulses, first and late cancellation, exact error reasons, progress forwarding,
and behavior after terminal failure. Verify retained controls through concrete
and erased adapters. Compare the baseline and changed path before claiming
polling equivalence. A compound control may intentionally check independent
sources; the same source should not be duplicated accidentally.

The remaining contract decision is where a per-call Pulse receives progress
when a job Pulse also exists. The migration currently proves cancellation
carriage; it must not imply an unimplemented progress-composition guarantee.
