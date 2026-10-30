# Explicit native timing for unedited streams — 2026-10-30

The draft driver rejected `with_container_timing` whenever a demuxer reported
any known timeline, even if every track was explicitly unedited. This made
truthful IVF discovery disable the caller's explicit native-timing choice.

The correction permits that route only when every track is `Unedited`.
Edited programs and mixed known/unknown declarations retain their refusal;
`new` and `preserving_presentation` retain strict admission. No existing
assertion was relaxed. This does not add exact preservation to IVF or WebM.

## Shared controls

New regression source `950617bbb7786313d05f910e13a50d06a96dd141` fails against
the original driver with `Unsupported(PresentationTimeline)` before output.
Receipt: rc101, 12s, peak RSS 0.32GiB, minimum available 14533MiB, load 0.35.
Two earlier setup attempts ran no tests: the export lacked a root lock, then
the committed workspace lock lacked the testkit progress-helper dependencies.
Offline resolution added critical-section/how-far-along/how-far-really and
selected cached Tokio 1.54.1 instead of1.54.0. This temporary lock is retained
with both before/after runs; it is not presented as an unchanged release graph.

Corrected source `801242af2e97603c54287aab301b478680212497` passes
`just track-timeline-check`: 3 media unit tests, all 80 media-testkit tests,
strict all-target Clippy, no-default WASM and the zero-case doctest command.
All 8 compiler catalogs regenerated. Receipt: rc0, 75s, peak RSS 0.35GiB,
minimum available 14247MiB, load 1.00. Package cleaning forced rebuilding from
the fixed export before tests; the original failing source remains intact.

## Native caller and guide

`perf/io-validation/tests/presentation_routes.rs` constructs an actual IVF,
demuxes it through SliceCursor, requires an explicit Unedited declaration,
then remuxes through Output::with_container_timing. It checks identical bytes
and timestamps 0, 2, 5. Strict presentation still refuses before destination
bytes. Payloads are opaque AV1 packets: this proves container routing and
packet retention, not decoded pixels or independently verified playback.

The first native compile stopped because the harness did not enable the
media/std output module. Source `e7a872fc1b9fd30c6991bc3ae1227529c0f53263`
adds that harness feature; production defaults are unchanged. The corrected
run passes all 78 guide tests, strict Clippy and the exact committed transport
CSV comparison. The test is included in both docs-site-examples and CI.
Receipt: rc0, 45s, peak RSS 0.34GiB, minimum available 14118MiB, load 1.30.
The earlier compile failure remains recorded: rc101, 62s, peak RSS 0.40GiB.

All 26 site source checks pass: rc0, 4s, peak RSS 0.34GiB,
minimum available 13308MiB, load 0.23. Browser tests were not rerun.
All runs use 3G/jobs2 caps and command timeouts. These are build/test resource
receipts, not codec performance or whole-operation memory measurements.
Logs, frozen sources and locks remain under `~/tmp/container-timeline-20261030/`
and the native qualification run directory. No generated executables archived.
