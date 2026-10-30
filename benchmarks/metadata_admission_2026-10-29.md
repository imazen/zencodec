# Checked metadata admission candidate

Source tested: `3795b19c139972997b1b86084d143c4afa534641`, based on
main `529b57975b8e8e7d7e7d62a49bbfc315f2367afe`. Later spec/recipe edits
change no tested Rust source. Native adoption remains unqualified.

`just metadata-admission-check`, isolated ARM64 export, run-heavy 3G/jobs2,
900-second timeout:

- Core all-feature library tests: 686 passed, none failed or ignored.
- Testkit operation-policy tests: 13 passed, none failed or ignored.
- Core all-feature/all-target strict Clippy: passed.
- Wrapper: rc0, 67s, peak RSS 0.51 GiB, minimum available 14224 MiB,
  peak load 1.50. These describe the validation process, not codec memory/performance.

New controls cover default refusal of nonempty metadata and empty clearing,
retained job identity/no native startup after rejection, checked builder/dyn
forwarding, concrete native error preservation and atomic metadata state,
ordinary/Send request refusal before execution, and continued operation when
no metadata requirement was requested. The positive shim fixture qualifies
forwarding only: it is not an encoder or native retention oracle.

Preserved failed attempts: the first export omitted the ignored workspace
Cargo.lock and refused `--locked` before compilation (rc101, 3s, 0.06 GiB).
The next used the prior qualified isolated workspace lock, SHA256
`cceafcb184efae78e61716d4a31166d257e0beba84a6a81e5463a4af23b1a9a3`,
and failed on two incorrectly named HDR fields in the new test (rc101, 65s,
0.44 GiB). Correcting that new fixture did not change existing assertions.
All attempts remain in the validation run directory `metadata-admission-20261029`.

Required next qualification: actual JPEG/PNG rejection or retention, unchanged
owner after native rejection, later setter/declaration conflict before output,
independent positive metadata readback, and reusable native testkit coverage.
The best-effort setter is not deprecated until the replacement is qualified.

All eight compiler-derived catalogs regenerated. Strict eight-crate Rustdoc and
26 site source tests passed with no skips (rc0, 29s, peak RSS 0.35 GiB,
minimum available 14122 MiB, peak load 1.07). The catalog run had first stopped
after generation because the archive lacked Git metadata and the runner omitted
`GITHUB_SHA` (rc1, 52s, 0.35 GiB). Supplying the exact exported source revision
and the existing locked site dependencies repaired the runner; no assertion was
changed. Browser interaction was not rerun: this changes signatures/catalogs,
not layout or navigation. Native retention remains the blocking acceptance gate.

## Shared readback oracle

`check_metadata_retained` compares the resolved expected output record with
completed native/independent readback. It never consults capability flags.
Opaque bytes, CICP, CLL, mastering display, diffuse white and orientation are
checked; extra metadata and privacy require separate conformance. Explicitly
requested baking may normalize orientation/EXIF only with transformed pixels.

Source `55baffbaf705ae58522b32a0e75f4ffa2c8d5d9b` passed the two focused controls
(including each field's omission/substitution and payload-free diagnostics),
testkit lib/tests strict Clippy, eight catalog generations and strict Rustdoc,
and all 26 site source tests. Wrapper rc0, 71s, peak RSS 0.34 GiB,
minimum available 14079 MiB, peak load 1.02. No native result is implied.

Preserved fixture compilation failures: nonexistent root DiffuseWhite re-export
(rc101, 25s, 0.36 GiB), corrected to zenpixels::hdr; incorrect CICP fixture field
(rc101, 9s, 0.33 GiB), corrected to color_primaries. The intermediate helper also
passed before stronger per-field substitution controls were added (rc0, 107s,
0.35 GiB). No pre-existing expectation or native output was changed.

## First native JPEG qualification

JPEG source `92ce25dd362a9d705e0745c180ca0bd67af988af`, core/testkit
`5493eaaad039a3f6e07f1cc37082f8e7c067caee`, paired pixels `f23ac753`:
13 checked-metadata controls, 2 cancellation controls, 19 native hardening
controls (including all 47 verified native seeds), and 9 Pulse controls passed.
The root independently checked the remote logs and absence of Rust processes.
Targeted clean plus verbose compilation verified the new source path. The lock
delta changes only five core-family revisions; this remains a locked graph,
not proof of portable unlocked dependency selection.

Test wrapper rc0, 195s, peak RSS 0.83 GiB, minimum available 13855 MiB,
peak load 1.76. Strict native library/four-test-target Clippy rc0, 22s,
peak RSS 0.59 GiB, minimum available 14010 MiB, peak load 1.43.
The original attempt's fixture import/type errors are preserved separately;
its correction changed no library source or behavioral assertion.

These controls include native EXIF/XMP/ICC readback and independent EXIF/ICC
inspection. Independent XMP inspection and decoded-sample parity against the
existing encode path remain acceptance gates. The independent JPEG decoder's
successful length check is not a pixel-correctness oracle. No native landing,
external fuzz-corpus coverage, reporting-overhead or full codec-matrix claim.

The guide's existing metadata chapter now exposes checked typed/dyn signatures
and labels the QOI example best-effort. All 26 site source controls pass. A
first local preview refused a stale Rustdoc revision; the old generated preview
was preserved and a fresh source-only preview built, without weakening the
revision guard. Matching full Rustdoc had already passed above.

## Deprecation and compatibility qualification

After native independent readback passed, the old best-effort typed/dyn setters
were deprecated. Compatibility controls retain their behavior with narrowly
scoped allowances; checked calls are not suppressed. The orientation example
still qualifies spatial representation, explicitly labeling metadata embedding
as compatibility behavior until that adapter's checked retention is available.

Source `80c3cf823b18e3817453340a995827eadf05528f` passed workspace all-feature
all-target tests (1415 passed, 0 failed/ignored) and doctests (31 passed,
39 existing ignores, 0 failed). Strict Clippy then rejected the orientation
example's unsuppressed compatibility call: rc101, 195s, peak RSS 0.48 GiB,
minimum available 14168 MiB, peak load 1.80. No runtime assertion changed.

Source `3a29aa9527fc3117c9e1ab719d97bbc9b89159d3` adds scoped compatibility
annotations to that example and the standalone historical/native witnesses;
one existing expression was rustfmt-reflowed without semantic changes. Workspace
all-feature/all-target strict Clippy, all eight catalogs, strict Rustdoc and all
26 site source controls pass: rc0, 99s, peak RSS 0.39 GiB, minimum available
14213 MiB, peak load 1.41. Debug symbols and incremental compilation were disabled
for these capped validation runs; these are not compile-performance measurements.
Native follow-through must use the final pinned core revision before landing.

Guide CI on `61fb96f1` passed its runtime examples but rejected two remaining
legacy metadata calls in `docs/examples/journeys.rs` under strict Clippy.
The two QOI compatibility examples now have function-scoped deprecation
allowances, matching their explicitly best-effort guide text. No checked call,
assertion or behavior changed. Rustfmt and all 26 site source controls pass;
the native guide Clippy rerun remains a CI gate. Original failed job:
https://github.com/imazen/zencodec/actions/runs/40992267193/job/123857227055

## Final core landing

PR186 merged as `2f8720de6a26d9c68b0dceafd5b8e910c09c37c3`; remote main
ancestry was verified. On `0705b277`, all reported CI checks passed, including
the corrected guide examples, browser guide, native JPEG/AV1/TIFF/JXL,
Linux ARM64/x64, Windows ARM64/x64, macOS ARM64, i686 and WASM.
[Guide run](https://github.com/imazen/zencodec/actions/runs/40992767973) and
[core run](https://github.com/imazen/zencodec/actions/runs/40992767828) retain
the results. This is a source landing, not a crate publication.

Native JPEG `67d5c27b880ceed2cc4e47d866eab09e2e86f011`, pinned to core
`61fb96f1`, passed all 13 checked-metadata controls after a package clean and
verified source compilation. ARM64 run-heavy receipt: rc0, 168s, peak RSS
0.77 GiB, minimum available 13698 MiB, peak load 1.93. Strict all-target
Clippy subsequently failed on the existing `examples/decode_wall.rs`
`ptr_arg` lint: rc101, 22s, peak RSS 0.59 GiB, minimum available 14121 MiB,
peak load 1.82. Logs are retained under `terra-jpeg-progress-67d5c27b`.
Native landing remains gated on that repair; this does not qualify missing
parallel worker progress or other codecs' checked metadata adoption.

JPEG PR252 merged as `13a8a23bb0062fe08f5c55dd4abe36aab9acc546`; GitHub
compare confirms its reviewed head `9dcd37a4` is an ancestor of main.
Final strict all-target Clippy (`inventory,parallel`) passed at `faa3d261`: rc0,
46s, peak RSS 0.83 GiB, minimum available 13692 MiB, peak load 1.07.
Archive comparison with `67d5c27b` confirms native library, manifests and lock
identity; test differences are scoped legacy deprecation allowances. Two
example lint repairs and newer viewer changes are separate. No runtime tests
were rerun for this lint-only gate. Full log: `terra-jpeg-progress-faa3d261`.
The final child updates only changelog and qualification documentation.

## Caller guide integration

The native harness now pins JPEG main `13a8a23b`. Isolated resolution changes
only the JPEG and companion zenyuv source revisions; the graph checker reports
10 interface consumers and zero identity mismatches. The caller recipe registers
JPEG and uses `EncodeRequest::with_metadata_policy(PreserveExact)`. Its control
requires XMP retention through the shared testkit and independently checks the
standard APP1 payload, then requires a typed MetadataRetention refusal for CLL.

The final source (`679dd338`, before this receipt) passes all 67 native harness
library tests and strict all-target Clippy. ARM64 wrapper: rc0, 9s, peak RSS
0.34 GiB, minimum available 14356 MiB, peak load 1.05. Full logs and encoded
artifacts are retained in `checked-metadata-guide-20261030`; no performance or
whole-codec memory bound is inferred. The first focused build passed the new
control (58s, peak RSS 0.69 GiB); the final run also uses the displayed registry
helper. No existing assertions changed.

Site generation and all 26 source tests pass. The first site build correctly
rejected a missing graduated explanation for the new card; the explanation was
added without relaxing the guard. Browser qualification remains CI's separate
gate. Reproduce the focused native gate with `just checked-metadata-guide-check`.

## Native PNG landing — 2026-10-30

PNG PR50 merged as `a5eeaddb3c5efc92c9ff6b3e08e3124b463f0c14`;
GitHub comparison verifies reviewed head `117718bd` is its ancestor.
Checked retention covers still, declared rows, shared pull-driver, APNG and
ordinary/Send erasure paths. Native callback pull remains explicitly unsupported.

Focused controls passed 12/12 on `f1f2cd62`. Equivalent lint-repaired `59c377f3`
passed strict all-target Clippy and five complete serial test targets: integration
66, orientation 3, row planning 7, shared contracts 4 and source color binding 5
(85/85, no ignored or filtered tests). These include the focused controls; do not
add the two counts. Selected-run receipt:
`rc=0 7s | peak-RSS 0.12GiB | min-avail 14666MiB | peak-load 0.18`.
The same-source artifacts were Fresh; their original compilation is recorded
in the interrupted broad log. Standalone fuzz graph `ee36012e` subsequently
compiled all five targets with one core/pixel interface graph:
`rc=0 31s | peak-RSS 0.35GiB | min-avail 14166MiB | peak-load 0.79`.
No fuzz campaign or speed benchmark is claimed.

The broad library run was OOM-killed in its 3 GiB scope and has no final test
result. Its zero peak-RSS report is invalid. Existing APNG tests compare raw
reference subframes to native composed canvases and tolerate mismatch counts;
missing-input returns also prevent a full corpus claim. These remain open in
[PNG issue 51](https://github.com/imazen/zenpng/issues/51). The selected tests do
not discharge those gates or publication requirements. Shared guide repinning
to this landed PNG implementation remains a separate integration check.
