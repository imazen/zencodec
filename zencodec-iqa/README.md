# zencodec-iqa

An unpublished, `no_std + alloc` contract for configured still-image quality
scorers. The core depends on zenpixels, how-far and whereat; codecs and
metric implementations remain outside it.

Each scorer exposes one named scalar with its native direction, unit and
nonfinite policy. Checked calls retain its complete configured identity,
validate dimensions and the caller's per-image pixel ceiling, and observe
cancellation before and after backend work. Implementations validate actual
color contexts, alpha, precision and luminance and poll during long work.
Packed rows keep their native strides.

`Metric` represents full-reference scoring; `NoReferenceMetric` represents a
single-image operation. Optional prepared references must reuse real work and
preserve cold-score bits. Unsupported preparation refuses explicitly.

Public records and enums are non-exhaustive. Construct identities and controls
through `MetricId::new`, `ScoreSpec::new`, `Descriptor::new` and
`EvalOpts::new().with_pulse(pulse).with_max_pixels(limit)`; retain wildcard arms
when matching enums. Optional trait operations must default to explicit refusal
so adding them can preserve existing implementations.

Checked operations borrow the full [how-far Pulse protocol](https://github.com/imazen/how-far)
and own one child phase. They complete that child with the actual successful,
failed or cancelled outcome; the caller retains completion ownership of its
parent. Implementations can split work, report native units with `opts.step`,
set totals and share observation with owned workers. Join workers before
returning and use the new pulse supplied to each prepared-reference evaluation.
Rejected phase planning records a diagnostic and retains cancellation through
how-far's untracked fallback; it does not invent progress or retain reporting.

Errors use [whereat](https://github.com/imazen/whereat) as `At<Error>`: start a
native trace with `start_at()`, propagate with `.at()`, and transfer an existing
trace with `Error::from_located(kind, cause)`. The original typed cause remains
available through `Error::source`. `stop_reason(&error)` supplies cancellation
classification to outer how-far `*_classified` result handoffs; the underlying
`Error` also implements `AsStopReason`. Native failures retain priority over a
stop observed after backend work.

The `codec` feature adds `codec::score_outputs` for packed `ImageOutput` pairs.
It requires full-canvas composition and identity orientation, holds both
mapping guards while scoring, and passes actual pixel contexts unchanged.
Render/crop/convert explicitly first. The bridge refuses unresolved geometry,
composition and native component interpretation.

This initial contract includes no metric kernels, native planar ingress,
spatial maps, temporal pooling, encoder search, metric registry, model loader,
GPU handles or implicit conversion. Pixel admission is **not** a backend
memory/workspace budget. The caller retains input/preparation provenance;
metric identities bind score-affecting preprocessing and model hashes.
Existing zenmetrics implementations are not migrated by adding this crate.

Run `just iqa-check` and `just iqa-portability-check` from the repository root.
The standalone manifest pins the same unpublished pixel source as codec main; publication needs released
dependencies and separate owner approval.

Validation on 2026-11-04: native contract tests pass with default features
disabled and with the codec bridge; strict all-target Clippy and Rustdoc pass.
Rust 1.88 checks all features, and the core builds on freestanding ARM; core
and codec features also compile for wasm32-unknown-unknown. These are protocol
controls covering nested phase ownership, worker sharing/reporting, planning
refusal, cancellation, trace/cause transfer and extension constraints, not a
qualification of a metric algorithm or every decoder/backend.
