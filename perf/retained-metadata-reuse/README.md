# Retained preparation reuse experiment

This is a tested native candidate on PR183, **not adopted JPEG behavior**.
The preceding [ARM confirmation](../../benchmarks/retained_decode_arm_2026-10-29.json)
found a small progressive-image regression from repeated preparation work.

`jpeg.patch` retains the configured decoder's completed metadata inspection and
selected descriptor. Repeated preparation checks cancellation before returning
that descriptor. The full metadata scanner accepts the already-validated header;
execution reuses its structural facts instead of reparsing the header again.
The complete primary-image scan and late/split ICC checks remain. No new public
API or caller query is introduced beyond PR183's existing proposal.

After output selection, execution drops the scanned carrier buffers before
native reconstruction. Actual decode still owns output metadata and native extras;
prepared facts do not manufacture a replacement result. Preparation retains its
original dimension/progressive checks before the full scan. Unsupported native
configurations still return unknown and receive final output validation.

## Reproduce

First use PR183 `9efb21077dc446ba2285eb7d71efca80324130e1`'s
`perf/retained-decode-validation/prepare.py` to create an isolated prepared export
from the core and native repositories. That script checks the JPEG/YUV source
substitution against the frozen lock. Then, from current zencodec:

```sh
just retained-metadata-reuse-apply "$EXPORT"
```

The applicator verifies all three input source hashes, dry-runs both patches
before mutation, leaves the Cargo lock unchanged and records resulting identities.
An already-patched or different source refuses. Patch reproduction was checked
byte-for-byte against the tested source; repeated application refuses.

In the export's `core` directory, through the normal serialized resource wrapper:

```sh
JPEG_POLICY_ARTIFACTS="$ARTIFACTS" cargo test --manifest-path perf/jpeg-policy-validation/Cargo.toml --features retained-output --locked --lib
cargo clippy --manifest-path perf/jpeg-policy-validation/Cargo.toml --features retained-output --locked --all-targets -- -D warnings
```

## Qualified behavior and limits

All 62 native harness controls and strict all-target Clippy pass on ARM64.
The existing preparation matrix retains seven layouts, three ICC placements,
baseline/progressive input, repeated preparation, crop unknown fallback and exact
sample/metadata equality. Additional assertions compare native retained segment
bytes and types. Eight cancellation cases stop after successful preparation,
then require the original Cancelled/TimedOut category from cached preparation
or decode. Existing assertions and feature coverage were not relaxed.

The final wrapper reports rc0/9s/peakRSS0.32GiB/minavail14453MiB/load1.00 under
3GiB/two jobs/timeout900. Initial correctness passed in80s/peakRSS0.67GiB;
explicit carrier-release refinement passed in29s/peakRSS0.71GiB. These are build
and validation resources, not decoder memory measurements. Full logs remain
at `~/tmp/retained-reuse-20261029/{check,check2,check3}.log`.

**Runtime non-inferiority is not established. Matched allocation costs are below.** The inline cached
owner changes decoder layout; retaining full metadata between preparation and
execution can change peak memory. Early release is not a numeric bound. Before
native adoption, repeat matched heap profiles and the unchanged five-process,
30-round, 1% runtime protocol; compare ordinary decode too. Do not infer that
removing two parses eliminates the observed regression. Native main checkouts
and their unrelated work remain untouched.

## Runtime confirmation

The unchanged five-process/30-round/1% ARM confirmation completed with zero
gate waits and no unreliable/incomplete replicate. Aggregate rc2 is an
inconclusive result, not a successful optimization gate:

| Fixture | Preparation overhead | 95% family interval | Verdict |
|---|---:|---:|---|
| 97×65 baseline | +1.1634% | -0.4691% to +2.8227% | Inconclusive |
| 97×65 progressive | +0.8781% | +0.1834% to +1.5776% | Inconclusive |
| 1024×768 baseline | +0.6217% | -0.3190% to +1.5712% | Inconclusive |
| 1024×768 progressive | +0.2426% | +0.0390% to +0.4465% | Equivalent within 1% |

[Exact receipt](../../benchmarks/retained_reuse_runtime_2026-10-29.json).
The point estimates cannot establish improvement over the earlier experiment:
these are separate confirmations, and ordinary decode also changes carrier
lifetime in this candidate. Uniform ARM CPU placement is enforced; frequency
is unverified. No x86, whole-application or memory claim follows. Wrapper rc2,
25s, peak RSS0.02GiB, minimum available14823MiB, load1.74; build separately
passed in198s/peakRSS0.72GiB. Raw results remain in the run directory above.

## Matched ARM allocation comparison

The same release probe, compiler, native pins and fixtures ran against the
original PR183 preparation and the reuse candidate. Each profile replays every
heaptrack allocation/free event and checks its peak against heaptrack_print.
The executable pathname is identical; plain/required mode names and fixture
pathnames have equal lengths. Source hashes and all pixel checksums match.
The baseline package was explicitly cleaned and compilation logs identify its
original source paths; this is not a reused candidate executable.

| Fixture / call | Original peak heap bytes / allocations | Reuse peak heap bytes / allocations |
|---|---:|---:|
| Baseline ordinary | 2,820,122 / 73 | 2,820,226 / 73 |
| Baseline required | 2,820,127 / 78 | 2,820,231 / 74 |
| Progressive ordinary | 5,323,424 / 119 | 5,323,528 / 119 |
| Progressive required | 5,323,429 / 124 | 5,323,533 / 120 |

Reuse removes four successful-preparation allocations while increasing peak live
heap by 104 bytes on every measured path, including ordinary decode. Within the
reuse candidate, required output costs one allocation and five peak-live bytes
more than ordinary output on these fixtures. These are full-process measured
values, not a universal decoder bound. They do not cover large ICC/EXIF, every
native configuration or stack/compile-time cost. Do not attribute the 104 bytes
to a specific allocation site without further inspection.

[Exact receipt](../../benchmarks/retained_reuse_heap_2026-10-29.json) includes
binary/input hashes and all eight cells. `decode-required-output-profile` adds
a paired mode to the existing profiler without removing its ten-path defaults.
Both profile batches and strict binary Clippy pass. Reuse wrapper rc0/77s,
peakRSS0.56GiB/minavail14172MiB; original wrapper rc0/76s,
peakRSS0.73GiB/minavail13976MiB, each 3GiB/two jobs/timeout900. Those wrapper
figures are separate from the heaptrack allocation values above. Raw captures,
interpreter output and logs remain under `~/tmp/retained-reuse-20261029` in
`heap-profiles` and `base-profiles`. Regenerable executables are not archived.
