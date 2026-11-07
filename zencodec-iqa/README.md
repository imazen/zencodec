# zencodec-iqa

Unpublished, `no_std + alloc` codec-output adapter for
[zenpsy-api](https://github.com/imazen/zenpixels/pull/111). Metric traits,
identity, native scores/maps, preparation and conformance live in that core;
this crate re-exports it as `iqa` without defining a second metric hierarchy.

`score_outputs` maps a reference/candidate `ImageOutput` pair into the core's
checked `score` call. It requires packed, full-canvas, composited, already
oriented samples and holds both mapping guards for the complete evaluation.
Actual pixel contexts and row strides pass unchanged. Native mapping causes,
whereat traces and cancellation classifications survive the bridge.

The adapter owns one how-far mapping phase, with core scoring nested inside it.
It never completes the caller's borrowed parent. No metric kernels, conversion,
layer composition, native-plane mapping or video timeline alignment is included.
Use an explicitly configured presentation/conversion route before scoring.

Run `just iqa-check` and `just iqa-portability-check`. The standalone manifest
pins the reviewed IQA/pixel source together; publication is a separate decision.
Core protocol and real-metric controls belong to the core and its separate
zenmetrics validation harness. Adapter controls cover unresolved geometry,
actual color context and native mapping error/stop propagation.
