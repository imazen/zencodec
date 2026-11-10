# Row driver and policy ablation spike

Baseline: PR208 `23ca16ff9af0a9fe92fb8c00853d998013f880eb`.
No main/PR208 source, dependency kernel, established assertion or threshold was
changed. Public signatures/snapshots remain unchanged. The proposed one-policy
signature and intersection helper are private test prototypes.

## Qualification

All commands run on r7900x through run-heavy, 4G hard cap, two Cargo jobs and
an outer timeout of 1200 seconds. Full logs, isolated source exports, original
failures, JSON receipts and heap captures are retained under the coordinating
checkout's external `zencodec-row-policy-spike` run directory. No executables or
heap captures are committed.

| Run | Source | Result | run-heavy peak RSS |
| --- | --- | --- | --- |
| 02-io | b55e43b8273d3c7504d503390d72d91ed1b3755a | All-feature I/O tests/doctests and strict all-target Clippy pass | 0.43 GiB |
| 04-native-row | Same b55 runtime | Existing six native JPEG row-driver controls pass | 0.04 GiB |
| 07-core | 321f5a239c95d60067677b727e171e35a382e565 | Workspace tests/docs, strict all-target Clippy, no-default check pass | 0.49 GiB |
| 08-multipass | d376edf3f1c1c7a27213b7e8141959c3efee0799 | FAIL before build: frozen IMI beta SIMD cap conflicts with stable pixel graph | 0.03 GiB |
| 11-native | dca21e11037a8909c1a61a005e21f0c39ff4fe91 | All 69 native harness tests, doctests and strict all-target Clippy pass | 0.72 GiB |
| 12-core | 881f7e54ca384b28733a70ca17bb2be643e44911 | 1,459 workspace tests and 33 doctests pass; 39 existing doc ignores; strict all-target Clippy and no-default check pass | 0.50 GiB |
| 14-allocations | a4a5da18528a8e79406d3eb2d824fad2660d9ef9 | Both arms and both repeats pass | 0.72 GiB |
| 15-api | Same a4a5 runtime | All eight zenutils API snapshot sets regenerate without a diff | 0.28 GiB |

Core runtime files and the intersection tests are byte-identical to 12-core;
the final native control is byte-identical to 11-native. Later script, lock and
documentation edits do not replace those qualification identities.

Reproduction entry points:

```sh
just row-policy-spike-check NEW_RUN OWN_TARGET CANDIDATE core
JPEG_POLICY_ARTIFACTS=OWN_OUTPUT python3 scripts/qualify-native-cohort.py NEW_NATIVE_RUN \
  --revision CANDIDATE --target OWN_TARGET --roots jpeg --execute --lint
just row-policy-spike-allocations NEW_PROFILE_RUN OWN_TARGET CANDIDATE 23ca16ff
just row-policy-spike-check NEW_MULTIPASS_RUN OWN_TARGET CANDIDATE multipass
just api-doc-check
```

Wrap heavy commands with run-heavy on a host with available resources. Native
JPEG controls require djpeg. The added independent PNG reader is png 0.18.1;
its lock adds only fdeflate 0.3.7, miniz_oxide 0.8.9, png 0.18.1 and zlib-rs 0.6.8.
No existing package identity was removed or advanced. Kernel pins remain fixed.

## Allocation observation, not a throughput benchmark

The same native test source and full dependency graph run against each core/IO
arm. The test contains 36 driver encodes: four sample descriptors, PNG plus two
JPEG modes, and raw/typed/erased entry points. It also runs independent readers.
Each arm is repeated twice under heaptrack, with all pixel and byte assertions.

| Arm/repeat | Allocation calls | Temporary allocations | Peak heap, as displayed |
| --- | ---: | ---: | ---: |
| PR208 / 0 | 9672 | 1620 | 13.10M |
| PR208 / 1 | 9672 | 1620 | 13.10M |
| Candidate / 0 | 9672 | 1620 | 13.10M |
| Candidate / 1 | 9672 | 1620 | 13.10M |

Peak RSS includes heaptrack overhead and differs; no RSS reduction is claimed.
The instrumented durations are not timing evidence. These observations do not
establish memory bounds, throughput parity, allocation counts for large inputs,
or isolated adapter cost. The raw path now uses the common fill guards and
progress callbacks; their CPU cost remains unmeasured.

## Preserved failed attempts

- 01: orchestration omitted root lock resolution before `--locked`; no tests ran.
- 03: five native row controls passed; disk control refused missing caller artifact
  environment. 04 supplies that environment and runs all six unchanged.
- 05/06: new test-oracle imports, constructors, closure annotation and a mutable
  local did not compile. No production failure or weakened expectation.
- 09: 68 old native controls passed. The new oracle compared PNG palette indices
  with RGB samples. EXPAND now compares color samples without stripping U16.
- 10: 68 old controls passed. The new JPEG oracle reused a P6-only helper on gray
  output. Its separate djpeg invocation now explicitly requests RGB; old helper
  assertions remain unchanged.
- 13: baseline profile and native assertions passed, analysis failed because this
  heaptrack appends `.zst`. 14 detects the generated filename and profiles both arms.

The existing IMI resolution failure is still a blocker, not skipped coverage.
One-shot whole-raster fallback, native video/mux requalification and large-input
throughput remain outside the demonstrated result. Decisions and scenario scope:
[the spike design](../docs/row-policy-spike.md).
