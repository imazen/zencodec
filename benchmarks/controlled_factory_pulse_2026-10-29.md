# Image/video factory cancellation alignment — 2026-10-29

Base: `4decf60217c0` (abbreviated source revision).

The image adapter now offers `try_new(factory, policy, Option<&dyn Pulse>)`,
matching the existing video constructor's call pattern. It checks cancellation
before and after a successful factory, drops an unpublished decoder cancelled
during construction, and keeps an actual factory failure instead of replacing
it with a concurrent stop. `StepError::Cancelled` retains the original stop reason.
The earlier image `try_from_factory` remains as a deprecated compatibility method.

13 native controlled tests, 7 native decode-admission tests, all IO feature tests
and doctests, and strict native/IO Clippy pass: rc0, 28s, peak RSS 0.72 GiB,
minimum available 13495 MiB, peak load 0.73. Separate existing video frame-policy
suite: 13 passed, rc0, 1s, peak RSS 0.18 GiB, minimum available 14172 MiB.
`just controlled-factory-pulse-check` reproduces these gates in an isolated export.

The new cancellation test initially used a coroutine constructor without charged
storage and failed its positive retained-owner assertion. It now uses the existing
storage-admitted native constructor, preserving the assertion and checking zero
charged storage after cancellation. Original failure retained: rc101, 11s,
peak RSS 0.72 GiB. The first video command selected a library filter containing
zero tests; it provides no video coverage. The separate frame_policy target above
is the actual video evidence. No old assertions or applicability were relaxed.

Real baseline/progressive JPEG completion now calls the canonical constructor
with None, preserving exact rows, metadata, extras and owner transfer. Existing
compatibility tests still exercise the deprecated constructor. New controls cover
already-cancelled and during-construction timeout plus concurrent native failure.
No decoder trait or zenpixels changes are required. Long factories must poll
cooperatively; checks cannot preempt synchronous native work.

Raw receipts: /home/lilith/tmp/zenpixels-release-reconcile-20261029/
decoder-factory-pulse/full.log (initial failure),
decoder-factory-pulse-retry/retry.log and media.log (qualification).
Wrapper resource figures measure validation, not native memory bounds.
