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
- [WebP PR105](https://github.com/imazen/zenwebp/pull/105), head
  `5a6cce27`, fixes duplicate adapter polling and preserves the native timeout
  reason. Its [20 CI jobs](https://github.com/imazen/zenwebp/actions/runs/40686426847)
  and [three recompress jobs](https://github.com/imazen/zenwebp/actions/runs/40686426806)
  passed. Local one-shot cancellation, job/per-call and poll controls passed
  in run 52d. The baseline failures remain in the
  [probe receipt](../benchmarks/pulse_review_2026-10-28.json); they describe the
  old implementation, not the fixed head. The root test graph still includes a
  legacy zencodec through its PNG development dependency.
- [Zenpipe PR108](https://github.com/imazen/zenpipe/pull/108) retains the
  request Pulse on the animation decoder job (`b13e550f`) and passes the
  ImageJob control into native frame work (`60127aaf`). Each fix has a
  failing-first behavioral witness. The current head `363f7c50` also checks
  each frame's actual pixel format before passing it to the encoder; its
  20 animation controls pass, including strided input and refusal before write.
  These controls do not establish progress forwarding or exact timeout reasons
  through zenpipe's older `PipeError::Cancelled` variant.
- The native IMI migration is integrated into the shared-core multipass harness
  at `3e9306c4`. Its 17 native tests and strict Clippy pass with a single
  zencodec/pixels/how-far identity. A native one-shot cancellation failure from
  child progress startup now propagates instead of being ignored. This does
  not qualify per-call progress composition or exact stop-reason retention.
  See [the native harness scope](../perf/multipass-validation/README.md).

Zenpipe's overall CI remains failing: AVIF, HEIC and zenextras integration
still need migration at its pinned revisions. Its run 20d also retains two
explicit failures: the latency step executed no tests because all ten were
ignored, and imageflow-compat strict Clippy reported 65 existing lints.
Functional animation qualification is not a green full run. Cancellation
latency and throughput remain unmeasured. A separate
[transcode sequence-loss finding](https://github.com/imazen/zenpipe/issues/111)
remains open: success from its convenience transcode can return only the first
frame. Sequence preservation requires its own contract, independently of Pulse.

For each route, test job-only and per-call-only cancellation, quiet and reporting
pulses, first and late cancellation, exact error reasons, progress forwarding,
and behavior after terminal failure. Verify retained controls through concrete
and erased adapters. Compare the baseline and changed path before claiming
polling equivalence. A compound control may intentionally check independent
sources; the same source should not be duplicated accidentally.

The remaining contract decision is where a per-call Pulse receives progress
when a job Pulse also exists. The migration currently proves cancellation
carriage; it must not imply an unimplemented progress-composition guarantee.
