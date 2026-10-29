# Release candidate landing gates

The immediate deliverable is a reviewable zencodec release candidate. The larger
codec-agnostic image/video/transcoding goal remains open. Do not label the entire
codec family or every native mode qualified from this candidate's evidence.

## Freeze

PR175 merged as `aa30ebba`, including the joint route and production Rows/Replay driver. The retained-job API candidate in this batch is `EncodeJob::into_dyn_job`: an additive
provided method using the existing adapter, needed to retain native job-specific
configuration through shared dispatch. No new policy domains, proof framework,
source abstraction or rendering redesign enters this landing batch.

The subsequent caller convenience decision makes borrowed per-call controls
`Option<&dyn Pulse>`. `None` does not disable retained job cancellation or
mandatory policy checks. Unknown optional facts do not prevent default success;
unknown evidence for an explicit requirement still refuses before affected
effects. A situational callback may act on later facts while those effects remain
withheld under admitted storage. See [the contract and current native
qualification](optional-pulse.md).

Keep explicit refusals for unproven required evidence. Native AT_FINISH
continues to retain compressed output; this is not a streaming memory guarantee.
The provided erasure method passes core ownership controls and the native public
replay-driver matrix with strict Clippy on `a39616ba`. The separate native final integration gate and strict library Clippy now pass
on the optional-control graph at `6ab547aa`; its Undefined-slot native/shared
controls preserve the existing slot bytes and row-padding invariance. See [the optional-control receipt](../benchmarks/optional_pulse_2026-10-29.json) and
[the public-driver receipt](../benchmarks/replay_driver_2026-10-28.md).

## Current main documentation and platform qualification

On `eb4b9835217ed13b2b025de5a5a7d8b94600fb40`, the complete
[CI workflow](https://github.com/imazen/zencodec/actions/runs/40869926926),
[documentation workflow](https://github.com/imazen/zencodec/actions/runs/40869926872)
and [fuzz workflow](https://github.com/imazen/zencodec/actions/runs/40869926834)
passed. The CI jobs include Windows x64/ARM64, Linux x64/ARM64, macOS ARM64,
i686, WASM, MSRV, metadata features, Clippy, format and semver checks.

An isolated export of that same revision also regenerated the eight-crate
Rustdoc reference and passed all 24 guide source tests and the complete browser
suite: migration and checked-API examples, code-first roles/search/copy,
independent context expansion, keyboard interaction and responsive layouts.
The serialized `run-heavy --mem 3G --jobs 2` invocation of
`timeout 180s bash run.sh` ran `just docs-reference` followed by
`just docs-site-browser-check 3547`; it exited 0 in 50 seconds, with wrapper
peak RSS 0.32 GiB and minimum available memory 13045 MiB. Those resource figures
measure documentation validation, not codec execution.

This verifies the current core documentation and platform gates. It does not
qualify the separate application graph, resolve its orientation expectations,
or establish the still-open native workspace and deferred-policy contracts.

## Core landing evidence

1. **Passed on `6ab547aa`, receipts pushed as IMI `daa9c13d`:** native retained-job,
   encode/row/one-shot controls and strict library Clippy, including the corrected
   Undefined-slot native/shared controls. Broader feature/platform qualification
   retains its own scope; preserve the established opaque-slot contract.
2. **Passed on `a39616ba`:** corrected native/public-driver matrix, direct
   configured-job erasure, core ownership tests and strict Clippy. The graph audit
   confirms one core/pixel family. Previous failures and all no-effect, exact-byte,
   cleanup and typed-cause assertions are retained.
3. **Passed on `c910076c`:** refreshed API snapshots and eight catalogs,
   eight-crate Rustdoc HTML, 24 site/source controls and the full browser suite on
   its own server. Generated snapshots are committed unchanged from that export.
4. Inspect PR CI on Linux, Windows x64/ARM64, macOS ARM64, i686, WASM,
   MSRV, Clippy, format and API/semver gates. Locally qualified related changes
   may merge without waiting for CI under the owner's current instruction;
   investigate failures promptly. Publication still requires all release gates
   green. The first Windows run found a
   fixture's HOME assumption; the USERPROFILE correction passes on `d7b5a159`.
   The complete CI, fuzz and documentation workflows pass on `b45cadb2`,
   including the required Windows ARM64, macOS ARM64 and i686 jobs. Later
   guide/harness-pin changes preserve those library sources and pass their
   local native/browser gates on `9b606366`.
5. Review the exact public API diff, deprecated replacements, README, changelog
   and supported-mode matrix. Record remaining unmeasured performance/memory/
   compile-time costs explicitly; validation-wrapper RSS is not an encoder bound.

Merge only qualified commits. Unbuilt native proposals may be reviewed in draft
PRs with their missing gates explicit, and no test
failure is hidden with an ignore, feature exclusion or weaker assertion.

## Separate application integration gate

zenpipe/zencodecs `f0322466` integrates the qualified HEIC/RAW Pulse adapters.
All five portable roots pass source/family checks; the complete configured API
feature profile compiles and passes strict library Clippy. API generation and
freshness checks pass. The core guide's new consumer pin passes 48 native
JPEG/conversion scenarios, strict Clippy, 24 source tests and the full browser
suite on `9b606366`; see [the receipt](../benchmarks/guide_integration_2026-10-29.json).

Two AVIF encode-orientation carrier assertions remain unresolved: the tests
require stored tags while native still encoding bakes pixels. Neither assertion
was weakened. The independent consumer bug dropping the decode OrientationHint
is fixed with exact-pixel controls. Complete application/platform runtime and
Apple DNG extraction coverage remain separate. Do not mark that application
release ready from core CI. Any finding requiring a core contract change
returns to candidate review.

## Remaining measurements

The [2026-10-29 core study](../benchmarks/compile_core_2026-10-29.md) completed
60 fresh-target builds with A/A controls at candidate `e9f2f31d`. Median dev/default
consumer time was 1.93s published versus 2.79s candidate; release/default was
2.34s versus 3.83s. All observations remain exploratory: every invocation recorded
CPU-load flags and 10 recorded heavy-process observations. The observation-window
audit finds all CPU flags after builds, where Iterate's counter interval includes
the build itself; none occur before builds. Ten pre-build heavy-process observations
remain unexplained, and no in-flight samples exist. This is not clean-host evidence.
Dependency graphs and
API scope differ; these are not isolated costs of a single abstraction.

The older [companion compile study](../benchmarks/compile_adapters_2026-10-27.md)
is exploratory: all samples recorded background CPU load, six recorded heavy
processes, and supported features differ between baselines. It is not a clean
comparison of this final candidate. The final-candidate compile observation below remains exploratory. Native replay
throughput, total workspace and whole-operation memory costs remain unmeasured;
validation-wrapper receipts do not establish those bounds. Paired pixel changes
and their migration costs remain documented in the precision contracts.

## Publication

A mergeable release candidate is not publication authorization. Review README
with the owner, obtain explicit release approval, run all-target and doc tests,
verify the coordinated published dependency graph, and require green release CI
before the tag, GitHub release and crate publication sequence.

## Work discipline for this landing

One integration owner, no new exploratory lanes, one serialized qualification
batch per candidate, and focused reruns only for actual failures. Preserve all
reports and source commits. Maintain a bounded failure list in the owning docs;
do not spend remaining capacity repeating already-qualified unchanged matrices.

A fresh outer-consumer resolution on 2026-10-29 rejected the registry requirement
`zenpixels >=0.2.17, <0.4.0` (Cargo reported 0.3.0 yanked). The candidate's root
patch selects `283655c5`; downstream roots must provide that patch until the
paired pixel release is published. Benchmark consumers now carry the immutable
source patch explicitly. A successful workspace build does not qualify an
unpatched crates.io consumer graph.

## PR175 compile observation

[The 45-build comparison](../benchmarks/compile_candidate_2026-10-29.md) completed.
Load flags prevent a confirmed performance claim; retained medians describe only
the measured core/io/media consumer, not crates.io or full native applications.

## Native migration landing update

RAW PR26 merged as `316e6f53`: optional Pulse and retained job cancellation
are on main. Its platform/backend CI jobs passed; a legacy inventory-test
deprecation lint was corrected with scoped compatibility allowances, followed
by passing inventory cancellation tests and all-feature strict Clippy.

AVIF PR75 `122b1474` repairs its standalone consumer lock. The unchanged
locked default consumer passes with the unified core/pixel family. This does
not settle the two application encode-orientation assertions described above.

## Native video queue naming and admission

AVIF PR75 `9be62c4d` adds `Av1EncodeSettings::with_output_pull_threshold`
and deprecates `with_max_queued_frames`. The setting triggers an output pull;
it does not bound retained input or process memory. Small thresholds continue
to make progress. The four-case native audit, all21 media-session tests and
strict queue-consumer Clippy pass; the old builder retains compatibility coverage.

Native pre-allocation capacity evidence and an explicit hard admission requirement
remain required contract work in [issue160](https://github.com/imazen/zencodec/issues/160#issuecomment-6488692572).
A callback cannot establish a bound from output readiness or lookahead alone.

## AVIF landing and remaining decode admission

AVIF PR75 merged as `a21bb7b1`: the optional-Pulse native/media/MP4 migration,
clear output-pull-threshold builder and qualified standalone consumer/fuzz graphs
are on main. The temporary pulse workspace was forgotten and removed after
ancestry verification; ignored fixtures and logs were preserved separately.
Application encode-orientation expectations remain unresolved; this merge does
not change them or certify all native memory bounds.

[Required one-shot decode descriptors](decode-descriptor-admission.md) still
validate after decoding. Existing output prediction cannot safely substitute
for negotiation-aware admission, and switching to a sink must preserve full
metadata/extras. This remains a contract-design gate.

## Registry reuse

`CodecSet::clone()` shares immutable registration lists and templates; extending
a snapshot copies the lists on write. Per-set defaults remain independent. Only
the selected config is cloned for an operation. The
[config-clone controls](../benchmarks/registry_cloning_2026-10-29.md) and
[allocation comparison](../benchmarks/registry_snapshot_2026-10-29.md) distinguish
these costs: populated clones now allocate nothing, while constructing the shared
registry adds one 88-byte allocation on the measured target. This is not a
throughput or whole-operation memory claim.

## Paired pixel retirement integration

Pixel main `7d28c4b7` has exactly the tree already pinned at `283655c5`; PR95
and its output-plan parent PR92 are merged. The separate 0.3.1 retirement PR77
needed reconciliation with those new APIs. Its candidate preserves current
precision and output-plan code while retiring the previously deprecated APIs.

An isolated zencodec workspace resolves to one 0.3.1 pixel core and converter,
with no other package changes. The only core source migration found by that
workspace was removal of an unreachable ambiguous-color constructor call;
existing authority/fallback controls remain unchanged. Full workspace tests and
strict all-feature/all-target Clippy pass on that pair. Native/application
consumers and the published registry graph remain separate release gates;
no package publication is authorized by this qualification.

### Native pixel-pair qualification (2026-10-29)

The JPEG policy harness also passes against pixel retirement source
`c6772adbb89fcc35d75375c48c4da6dd6a059bb9`: 55 library controls, all-target
all-feature compilation, and strict all-target all-feature Clippy. The graph
retains the committed JPEG, PNG, UltraHDR and zencodecs revisions; its only
package identity changes are zenpixels and zenpixels-convert from pinned
0.2.17 to candidate 0.3.1. No native source migration was needed in this subset.

The first run passed 47 controls and failed eight because its runner omitted
`JPEG_POLICY_ARTIFACTS`. Repeating with an explicit artifact directory passed
all 55 without changing tests or libraries. Wrapper receipts: first run rc101,
14 seconds, peak RSS 0.70 GiB; corrected run rc0, 1 second, peak RSS 0.23 GiB.
These are incremental qualification costs, not codec performance measurements.

Reproduction: export the core compatibility change and pixel candidate; replace
only the harness's two pixel patches with candidate paths; start from
`perf/jpeg-policy-validation/dependencies.lock`; resolve and verify that only
those two package identities changed. Set `JPEG_POLICY_ARTIFACTS`, then run
`cargo check --all-targets --all-features --locked`,
`cargo test --lib --all-features --locked`, and
`cargo clippy --all-targets --all-features --locked -- -D warnings`
from the harness directory. The full zenpipe feature graph, other native codecs
and required-platform release checks remain unqualified for this pixel pair.

### Full application graph: first retirement blocker

The unchanged zenpipe `f0322466` full API profile fails compilation against the
paired candidate at four `PixelBuffer::into_vec` calls in zengif `efb7e43b`
(`src/codec.rs`: 1727, 1739, 1746, 1931). These are format negotiation and
animation-buffer reuse. The replacement must distinguish active pixels from
backing allocation: `into_parts()` preserves prefix, stride and trailing bytes;
`into_contiguous()` packs rows but still preserves the initial offset. Blindly
using `into_parts().data` is not a general packed-pixel replacement. Returning
unchanged RGBA directly also avoids reconstructing its owner and color context.

The isolated graph audit passes with exactly six planned identity changes:
four core-family packages and two pixel packages. Both registry and git core
edges need the same path bridge; the first audit correctly rejected the split
when only registry edges were patched. The full-profile compile then stopped
at GIF before broader qualification: rc101, 6 seconds, peak RSS 0.58 GiB.
No feature was removed, no test expectation changed, and the candidate remains
unqualified for this full application profile.

The GIF migration subsequently landed as `67a7e5f3`: 45 adapter tests and strict
Clippy pass on both pixel 0.2.17 and candidate 0.3.1. With that single additional
pin change, the unchanged full codec API profile compiles (rc0, 7 seconds,
peak RSS 0.59 GiB). Its strict Clippy and zenpipe's Tokio/job/JPEG/PNG/GIF/WebP/
filter compilation also pass (rc0, 9 seconds, peak RSS 0.55 GiB). The graph audit
allows exactly seven package identity changes: core family, pixels and GIF.
This qualifies compilation, not every native runtime route. Reconciled pixel
PR77 head `7b502075` now includes the source and qualification record; its final
workspace run passes 1,450 tests with 49 existing ignores and strict Clippy.

The reconciled pixel candidate is merged on main as `f23ac753` (PR77).
PR82's checked projection/error/finalizer functions and regression tests were
verified present; PR90's ICC implementation and tests were verified
byte-identical. Both redundant backport PRs are closed. The older
`release/0.3.1` bookmark was not advanced; main is the consolidated candidate.
Neither pixel package has been published. Required-platform CI and release
approval remain gates; this merge does not change zencodec's existing pixel pin.

GIF follow-through `a258e93c` also aligns its independent fuzz graph and uses
a fixture constructor available on its older pixel pin. The actual pinned
source graph now passes its 45 adapter controls, strict lib Clippy and all fuzz
target compilation. This repairs the initial CI failures without changing
pixel assertions or codec runtime logic.

## Paired pin rollout

The workspace and thirteen standalone harness manifests now select the merged
pixel revision `f23ac753`. Locked graphs have one copy of each pixel package.
IMI `e3b14aa2` widens its registry requirements; JXL encoder `3c1be817` already
has the required converter range. Its companion macros/SIMD crates move together
and its explicit hashbrown requirement changes from 0.17.1 to 0.16.1. The older
video witness now uses the AVIF/parse and UltraHDR revisions already qualified
by the AV1 harness, with the registry zenmeta edge explicitly patched.

All thirteen standalone graphs pass all-target compilation. Core workspace
all-feature tests and strict all-target Clippy pass on the actual git pixel pin.
IMI's 25 native controls and JXL's ten planned-color controls pass; see the
[native qualification record](../benchmarks/paired_pixel_native_2026-10-29.md).
This does not claim that every historical scenario witness is a passing modern
regression: the old multipass gap witnesses retain their original assertions;
only construction was adapted to the current protocol. Their compilation is
covered by `just paired-harness-check`, not their historical runtime outcomes.

## Checked orientation and merged-main validation

PR179 merged as `dec7f375`; native AVIF PR77 merged as `7d9753f1`.
Required stored/baked representation now reaches native jobs and retained plans.
The native matrix includes all eight orientations, plan/start consistency,
declaration refusals and typed resource-limit causes. Application PR108 adds
`EncodeRequest::with_orientation_mode`; its explicit-mode AVIF control and full
API generation pass. The two historical omitted-mode carrier assertions remain
unresolved; default baking has not changed. See [the contract](encode-orientation.md).

Merged-main CI, docs/native guide and fuzz workflows are green on `dec7f375`.
The additional default workspace all-target gate exposed a missing std dev
feature in media-testkit; enabling it for tests preserves the production default
and lets every existing assertion run. All-target tests, doctests and strict
Clippy pass after that repair. [Evidence](../benchmarks/release_workspace_2026-10-29.md).
This is not crate-publication approval or completion of the full application goal.

## Async cancellation and WebP row follow-through

All core CI, native/documentation and fuzz workflows pass on `da247a4a`:
[CI](https://github.com/imazen/zencodec/actions/runs/40943789698),
[docs/native](https://github.com/imazen/zencodec/actions/runs/40943789648),
[fuzz](https://github.com/imazen/zencodec/actions/runs/40943789647).
The async batch driver now polls Pulse before each native step and checks again
before returning successful decode completion. The queued-cancellation control
fails before the fix; 45 driver/quota tests and strict Clippy pass afterward.
This is cooperative cancellation at boundaries, not preemption inside native
steps. See [Tokio execution](tokio-execution.md).

WebP main `014b0e69` repairs inherited `begin` refusal and padded-row corruption.
Four explicit regression controls, all 51 existing codec unit tests and strict
Clippy pass on ARM64, including libwebp lossless readback. The shared testkit
permits configuration-specific row refusal, so passing that helper alone did
not establish that the advertised route worked. This adapter still buffers a
full raw raster and emits at finish. Checked plan/color admission is not covered
by this result; consumer integration is recorded below.
[Native receipt](https://github.com/imazen/zenwebp/blob/014b0e699a7694546f1008768da62086f1d54fc8/benchmarks/declared_rows_2026-10-29.md).

The remaining row-entry candidates from issue 164 now have native repairs:
TGA `9d71b356` retains its declaration instead of silently accepting mismatched
rows; JXL `a3a93aaf` implements the previously inherited refusing begin. Both
preserve the row-boundary Pulse reason and reject one-shot use after admission.
[The TGA receipt](https://github.com/imazen/zenbitmaps/blob/9d71b356f290f6438b810cc53d99f29301f6d247/docs/tga-row-contract.md)
and [JXL receipt](https://github.com/imazen/zenjxl/blob/a3a93aaf068c589675aa5c63090ac9d07f24d07c/benchmarks/declared_rows_2026-10-29.md)
record failing-first controls, preserved assertions and strict Clippy. They
qualify the existing buffered row protocol, not checked planning, workspace
bounds or low-memory streaming.

Zenpipe main `2908ce17` integrates all three repairs across six consumer graphs;
the final locks change only those three source revisions. The five unchanged
streamed-bytes tests pass for JPEG/PNG/WebP/JXL/PNM/Farbfeld/TGA, including
one-shot byte identity, output draining, row protocol and budgets. Native CI is
green for the three repaired codecs. Issue 164 is closed for its reproduced
admission failure; the full all-format dispatch gate remains separate.

The consumer also stopped converting native declaration/row errors to strings.
A failing-first JPEG control lost its typed timeout cause; JPEG/PNG declaration
and completion now preserve `Stopped(TimedOut)`. Existing Pulse tests, row tests
and strict Clippy pass. Buffered encoders may observe cancellation only during
finalization; this adds no latency or memory bound. Exact scope, failed fixture
attempts and resource receipts are in the
[consumer qualification](https://github.com/imazen/zenpipe/blob/2908ce178f86196da1b72e557ac5194d1fcfa2b5/benchmarks/row_consumers_2026-10-29.md).

## Collection admission and consumer strict-fit follow-through

Core `ca325bc81096d5d5894a21b4fbe9669f02b218bc` passes
[CI](https://github.com/imazen/zencodec/actions/runs/40957663410),
[documentation/native qualification](https://github.com/imazen/zencodec/actions/runs/40957663398)
and [fuzz checks](https://github.com/imazen/zencodec/actions/runs/40957663485).
This includes the early collection/render workspace checks and the corrected
native JXL expectation for a named pre-effect refusal. The independent placement
oracle and remaining canvas limits are indexed in [the rendering contract](render-contract.md).

Zenpipe `43301b4cbad056bf5b30256b2063e3d31a5b3377` fixes a consumer CI regression:
strict lossless sRGB-to-PNM fitting succeeded in the pipeline but the native
setter rejected the already-enforced strict flag. Fitted one-shot and streaming
paths now discharge that flag while retaining all native emission requirements;
unfitted retained jobs still receive the complete policy. Nine native color-fit
controls and strict Clippy pass with unchanged dependency lock. Full application
CI is separate. See the [consumer receipt](https://github.com/imazen/zenpipe/blob/43301b4cbad056bf5b30256b2063e3d31a5b3377/benchmarks/strict_color_fit_2026-10-29.md).

## Registry availability recheck (2026-10-29)

The live sparse index still has no unyanked release satisfying the current
`>=0.2.17, <0.4.0` requirement for either pixel crate. Both indexes contain
yanked 0.3.0; the highest unyanked version is 0.2.16. Checked directly from
[zenpixels index](https://index.crates.io/ze/np/zenpixels) and
[converter index](https://index.crates.io/ze/np/zenpixels-convert), after the
crates.io HTTP API returned 403. Index records are not assumed to be sorted
by semantic version. Git-patched qualification therefore still does not prove
a fresh registry-only consumer can resolve this candidate.

The paired source manifests are 0.3.1. Pixel source `f23ac753` has green
[CI](https://github.com/imazen/zenpixels/actions/runs/40833032100).
Documentation-only follow-through `33682afc` corrects the candidate README and
changelog: estimation APIs are removed even with the compatibility feature;
concrete-type conversion extensions and CllMeasure are sealed, while Pixel and
CMS plugin traits remain open. The generated crates.io README is refreshed.
These corrections change no library source or dependency requirement.

Before publication, the owner must review the corrected
[paired README](https://github.com/imazen/zenpixels/blob/33682afcfbc4a0192aa8143c0d6cb5334bcb0275/README.md)
and approve release. Publish and validate the paired dependency contents before
publishing zencodec. No crate was published or version requirement relaxed as
part of this audit.


## Package dependency audit (2026-10-29)

At `af1c9578`, `cargo package --list --allow-dirty --no-verify` succeeded
for all seven workspace packages in an isolated export. This checks inclusion,
not registry resolution or compilation. The separate renderer is not part of
that seven-package gate.

The versioned testkit has a normal dependency on `zencodec-io 0.1.0`.
I/O still has `publish = false`, and its
[sparse registry entry](https://index.crates.io/ze/nc/zencodec-io) returned HTTP
404 during this audit. Thus publishing the testkit requires a reviewed I/O
release first; core's successful workspace tests do not discharge that gate.
Coroutine, media, media-testkit and the separate renderer also retain their
explicit publication guards. Do not remove those guards as routine packaging
cleanup.

The dependency order, once each package receives release approval, is:

1. Paired zenpixels, then zenpixels-convert, with registry-only consumer checks.
2. zencodec core.
3. zenmeta and zencodec-io (each depends on core); coroutine can follow core.
4. zencodec-testkit after I/O, and media after I/O.
5. media-testkit after media; renderer after its own qualification.

This is dependency ordering, not a requirement to publish every companion in
one batch. Each selected package still needs the release gates above. Verify
normal, optional, target-specific and dev dependency resolution from the actual
normalized package; path patches must not conceal unavailable registry edges.

A focused I/O normalization probe used:

```sh
cargo package -p zencodec-io --allow-dirty --no-verify --exclude-lockfile
```

Cargo removed excluded test/example/benchmark targets and the unversioned
path-only coroutine dev dependency from the normalized manifest. There are no
dangling excluded targets in that manifest. This deliberately omitted lock
resolution and build verification: it is not a publication dry run. The
serialized wrapper exited 0 with peak RSS 0.03 GiB; that is packaging resource
usage, not codec memory. Full logs and file lists are retained under
`~/tmp/zencodec-package-audit-20261029` on the qualification host.
