# Shared track timeline review — 2026-10-30

This qualifies shared declarations and admission, not native edit parsing,
remuxing or playback. The native gates in [the review](../docs/track-timeline-review.md)
remain required before landing the API.

## Frozen source and commands

Source: `e2e120d74f2acaf715f076adacbec66ed4cf2537`.
Archive SHA256: `d884a8b32af20e6771d07d6cfe64bbda8cbbb3cbad1be4ee88202c4c6fe81c34`.

`just track-timeline-check` passes:

- Three media unit tests cover reduced exact rates (zero/reverse/signed minimum),
  unknown/unedited/empty/finite-zero/open distinctions, open-end ordering,
  shared list ownership and order-independent legacy/full conflicts.
- All 79 media-testkit tests pass, none ignored or filtered. Added controls cover
  typed/erased default refusal, no known-program fallback to packet-only or
  legacy output, mixed unknown refusal, supported synthetic dispatch, and a
  strict preservation request refusing all-unknown evidence before native calls.
  Conformance comparison now includes the full timeline; the existing ten
  window-dependent metadata cases remain and an eleventh timeline case is added
  at both headers and configuration changes.
- Strict all-target Clippy passes for media and its testkit.
- The media crate builds without default features for wasm32-unknown-unknown.
- The doctest command succeeds with zero doctests; it is not executable
  qualification of the new Markdown example.

The final serialized batch also regenerated all eight compiler catalogs. It
used an isolated export, locked root dependency graph, a 3 GiB cap, two jobs,
900-second timeout, and explicit empty Rust flags for portable target selection.
Run-heavy receipt: rc0, 52s, peak RSS 0.32 GiB, minimum available 14356 MiB,
peak load 0.75. Source paths in the log identify the frozen export.

All 26 site source checks pass (rc0, 4s, peak RSS 0.23 GiB, minimum available
13715 MiB, peak load 0.63). Browser interaction tests were not rerun. These are
build/test resource receipts, not playback performance or operation-memory
bounds. No zenpixels API changes are needed by these declarations.

## Retained earlier attempts

The initial batch passed three media tests, 78 testkit tests and strict Clippy,
then failed because the WASM target was absent (rc101, 29s, peak RSS 0.32 GiB).
Installing that target and rerunning the unfinished gate passed, with warnings
about an inherited host CPU flag. The final complete batch above explicitly
cleared both Rust flag variables and passed without that target warning. The
initial and rerun logs remain distinct; no assertions were relaxed.

## Limits

The synthetic header-only muxer refuses every packet. Its success proves driver
and shim dispatch, not preservation by a real container. Existing all-unknown
legacy tracks retain their prior packet-timing route; only the explicit
preservation constructor certifies that unknown evidence cannot enter native
preparation. Known timelines never silently use that legacy route.

Native direct begin paths, MP4 parsing/writing, independent playback, bounded
map retention, replay retirement and presentation-only epoch changes remain
unqualified. Native backends must implement the new binding before a declared
program can succeed through the checked driver.

## Native compatibility projection follow-through

The deprecated offset getter now derives the historical first-media-start minus
leading-gaps projection from a retained full program. It explicitly omits later
edits and rates; native writing uses the complete program, never this projection.
The native MP4 snapshot `9cd12ba7` passed all 38 unit/integration controls against
this core source, including unchanged signed-rounding compatibility assertions,
full ordered edits, typed admission refusal and clean retry. Native library
source remained identical in `89d23600`, whose strict all-target Clippy and
no-default WASM checks pass after test-only lint repairs. This uses an isolated
path bridge, not a qualified portable git graph or independent playback oracle.

All eight compiler catalogs regenerated successfully: rc0, 29s, peak RSS
0.32 GiB, minimum available 14427 MiB, peak load 1.18. Full receipts remain
in the native repository's `benchmarks/mp4_timeline_2026-10-30.md`.

The updated compiler catalog also passes all 26 site source checks: rc0, 4s,
peak RSS 0.34 GiB, minimum available 13483 MiB, peak load 0.34. Native
[PR79](https://github.com/imazen/zenavif/pull/79) subsequently qualifies the
portable core `41b1357e` graph (38 tests, Clippy, WASM), recorded in its receipt.
Independent playback and broader consumer qualification remain open.

## Native consumer and guide adoption

IVF/WebM [PR56](https://github.com/imazen/zenextras/pull/56) refuses full
edit programs before native writer output. Both new refusal controls fail
against original production sources; all 29 container tests, strict Clippy
and WASM pass on the corrected candidate. IVF reports unedited input; WebM
retains unknown. Neither yet certifies exact presentation preservation.

Guide setup now pins MP4 `0070de3f` and IVF/WebM `2ebeca7d`. Frozen core
`c35638e2d283c481f4a4530209eeeba604000553` passes the unchanged
`just docs-site-examples` commands: 77 tests across eight integration targets
and two binaries, zero ignored/filtered, followed by strict Clippy. The
committed harness lock remains byte-identical. Run receipt: rc0, 97s, peak
RSS 0.40GiB, minimum available 14145MiB, peak load 1.59, 3G/jobs2/timeout900.
Full log: `~/tmp/container-timeline-20261030/guide.log`. This corrects the
old pinned containers' deprecated-scalar lint failure; it does not suppress
warnings. Transport CSV execution and browser controls were not rerun here.
