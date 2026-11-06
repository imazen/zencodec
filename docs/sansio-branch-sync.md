# Sans-I/O branch synchronization, 2026-11-06

## Open gates

The canonical PNG graph resolves through the manifest-only
[draft PR55](https://github.com/imazen/zenpng/pull/55), preserving floors and
explicitly admitting SIMD beta.2. Its owner's public-facade candidate6b740345
is directly atop that repair and clears all three original private-helper errors.
No PNG source or arithmetic was edited by this lane. Temporary <=beta.2 caps
exclude stable0.9.31; replace them after stable publication and unchanged gates.

The recovered JPEG implementation is [draft PR258](https://github.com/imazen/zenjpeg/pull/258).
The exact remote PNG6b/HDR302 main cohort now passes full strict library/test/
bench Clippy with the original CI flags and verified instrumented C++ enabled.
Minimal HDR public-facade and existing fixture API migrations retain ICC,
orientation, dimension and sample assertions. A real converter refusal retains
the concrete ConvertError and original location trace through typed/erased
codec envelopes. Complete release user/default/FFI/expert/recompress runtime
gates, supported examples/doctests and full API snapshots now pass. Broad
colorspace-oracle and platform gates remain open. Earlier dependency/compile
failures remain history, not the current full-lint result. Default triangle
remains whole-file backed and typed/erased factory promotion is unqualified.

[UltraHDR draft PR43](https://github.com/imazen/ultrahdr/pull/43) separates
SIMD admission from exact-content BT.2446 forwarding. The first feature-supplying
pixel commit is e70065f; a version0.3.1 label alone does not identify those
contents. Main-cohort fde59b96 is pinned and the feature forwarded explicitly.
Unchanged all-feature core unit/reference/SIMD tests pass on main and a distinct
matching PR199 export; main strict library/test Clippy passes. A separate benchmark-only
relaxed-alias migration preserves the exact beta.2 operation and runtime floors;
main all-target strict Clippy and doctests now pass. Public API, rounding and
numeric expectations remain unchanged; whole UltraHDR workspace/platform gates
stay open.

Core PR200's runtime workflows pass, while its native documentation/integration
jobs fail stable-only SIMD resolution in their independently pinned graphs.
Those failures remain gates; the focused checks below do not qualify them.

## Branch inventory and disposition

| Repository / published ref | Disposition |
|---|---|
| zencodec `sansio` (`666be3a8`) | Forward-merge current main, retaining current step/Input/optional-Pulse source; obsolete push/pull definitions are superseded |
| zencodec `feat/sansio-feed-step`, `feat/whole-file-incremental` | Already ancestors of main |
| zencodec `review/sansio-incremental-decoding` | Historical proposal; current Input/request/step contract supersedes it |
| zencodec `feat/sansio-seekable-source` | Historical separate seekable tier; current ReadAt/job source binding and companion I/O crate supersede it |
| zencodec `feat/sansio-coroutine-source` | Historical in-core coroutine and review notes; current companion crate and terminal/input controls supersede it |
| zenjpeg `sansio` (`7c59f19e`) | Forward-merged with main `cae3a9f3`, migrated and qualified in PR258; not merged to main |
| zenpng `sansio` (`11b26dd9`) | Source buffers until EOF before pixel decoding; it is not a native input-progress implementation |
| zenwebp `sansio` (`e62de277`) | Legacy push/pull adapter; no migrated native qualification in this lane |
| zenbitmaps `sansio` (`7e09b6a7`) | Historical QOI work; current main already contains the step-native QOI route; no replay of the old trait surface |
| zenjpeg/zenpng/zenwebp/zengif/zenavif/heic/zenextras/zenjxl `feat/sansio-whole-file` | Already ancestors of their inspected main histories; keep those implementations, not the historical names |

Local cached sibling main refs were stale. Live GitHub branch/PR inspection found
PNG main `a26cdc64`, JPEG main `cae3a9f3`, and core main `1fb997bb`.
Primary JPEG/PNG checkouts contained unrelated WIP; source work used an isolated
JPEG workspace and left those changes untouched. No historical branch was
force-pushed or deleted.

## Graphs qualified separately

| Identity | Main qualification | PR199 compatibility only |
|---|---|---|
| zencodec family | `1fb997bb88e7405e6a315bf1f0b22f71e99be272` | `88fc5871f38f34153d1dfe77379c94e1a2abb35c` |
| pixels/converter | `fde59b96632557592cffa9b7301acd8db96ed078` | `b6b12bbe702e7ca2f93db559858ae1f233a066ec` |
| garb | `605276d7` | `58ce9cad` |

PR199 remains unmerged. Its gray ICC/CICP and source-depth APIs were not put into
main. The selected JPEG consumer passed on each graph using a separate export
and target. Its lock comparison changes exactly the core family, pixel family
and garb; each graph has one shared core/pixel/Pulse/whereat identity. Enough and
almost-enough resolve to 0.4.5, how-far to 0.1.0 and whereat to 0.1.5.

Both shared graphs pass incremental testkit controls, coroutine and all-feature
I/O library/integration suites, strict I/O/coroutine Clippy, WASM and genuine
freestanding no-default checks. `sansio-sync-core-check` records the reproducible
commands. The actual native JPEG receipt and source/consumer limitations live in
[its owning repository](https://github.com/imazen/zenjpeg/blob/sansio/benchmarks/sansio_sync_2026-11-06.md).
The small [UltraHDR clone repair](https://github.com/imazen/ultrahdr/commit/aadf102169086fc232205bdb3b92572cfda07032)
is on remote main with green CI; it preserves the existing infallible signature
across old and fallible pixel-copy cohorts.

Shared main wrapper: rc0, 14s, peak RSS 0.30 GiB; shared PR199 wrapper: rc0, 48s,
peak RSS 0.43 GiB. These are serialized run-heavy validation resources on r7900x,
not codec performance or memory bounds. The initial `--all-targets` invocation
also launched executable benchmark targets; it was terminated (rc143), and its
partial timings are not evidence. The corrected command selects library/tests.

## Caller and copy implications

The ordinary erased factory remains the caller's common path. JPEG's explicit
review method `into_incremental_decoder(&[PixelDescriptor])` returns a concrete
owner implementing `step(Input, sink, Option<&dyn Pulse>)`; it is not promoted
as a second mandatory application route. `None` adds no borrowed control and
never disables an owned SharedPulse. Sink loans yield independently from input
starvation, and successful callbacks are followed by stop checks.

The JPEG candidate copies accepted compressed input into retained parser/scan
storage, reconstructs through a scratch batch and copies into one sink loan per
step. Marker metadata remains retained; progressive input remains buffered.
Neither a row interface nor early output establishes a whole-operation memory
bound. No copy-cost, throughput, preemption or bounded cancellation-latency
claim was measured in this lane.

## Final graph and API qualification

JPEG162a1f35 pins linear-srgb2969ee84, reusing the already-existing transfer
feature ownership506004d9 and equivalent NaN/nonpositive predicatescb3ef992
on frozen528. Later main numerical changes are not imported. Feature-off105
and all-feature165 library tests, all3 existing tier-parity controls and strict
feature-off/on library Clippy pass. Root and standalone locks change only the
linear source identity relative to the preceding reviewed cohort.

Complete API regeneration and strict checking pass with published API tool0.1.1
and pinned nightly2026-07-13, including featureless zenyuv. The default/internal
JPEG surfaces remain unchanged; only the review incremental decoder type,
explicit constructor and trait implementation enter the feature snapshot.
New recovery helpers remain private. Full original strict Clippy, default
strict library/test/bench Clippy and all11 selected native controls pass again
after these visibility/feature-ownership changes. Earlier full runtime gates
used the same active arithmetic and unchanged numeric expectations.

Complete FFI succeeds at6G/jobs2 with two test threads (83s, peakRSS1.39GiB).
Kernel logs prove that the two earlier3G attempts hit their cgroup caps; their
rc143 results and invalid zero-RSS receipts are retained as failures. This is
validation-job resource usage, not a codec memory bound. JPEG's actual
diagnostics WASM build and frozen legacy metric numeric controls pass while
retaining0.2.7 profiles, weights and API. Core's separately pinned documentation
graphs and whole UltraHDR workspace/platform gates remain open. Main and PR199
remain distinct; no promotion or merge is implied.

UltraHDR full std follow-through is now review head6df45a2e: all three members,
all-feature library/integration suites, four independent SDR/HDR/MPF/probe pixel
controls, doctests and strict all-target Clippy pass. Default/minimal-std suites,
all-feature core WASM and seeded standalone fuzz compilation pass too. API
regeneration/check passes with only one already-existing function recaptured.
Its root pins JPEG162a1f35; JPEG's independently qualified root still pins
UltraHDR30249993, so these review graphs must not be described as identical.

The genuine freestanding check exposes implicit thiserror/std, then43 missing
float methods after making feature ownership explicit. Both freestanding and
WASM without std remain failed. Portable math needs separate numeric review;
no backend, arithmetic expression or threshold was changed to accept it.
UltraHDR stays draft with that blocker and platform gates open. JPEG's broad
color-model suite now passes all three independent readers with unchanged
thresholds. PR199 remains open and unmerged at88fc5871.
