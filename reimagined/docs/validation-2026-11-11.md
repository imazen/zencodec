# Document candidate validation — 2026-11-11

This is an isolated, unpublished mechanism experiment. Missing native EXR/JXL/IMIJ,
async document driving, complete metadata and operation/storage qualification
are listed in [the design ledger](document-design.md). No release or performance
claim follows from these checks.

## Revisions and inputs

- Original reimagined capture: `7a87fc87`, from parent `1027e9f5` plus the
  verified working-tree capture in [source-capture.json](source-capture.json).
- Mechanical formatting: `aec82016`; baseline lint repairs: `96864b18`.
- Contract: `ed892eae`; driver: `e8570af8`; synthetic controls: `f72a3d51`.
- Native TIFF witness: `fd98c4ea`; exact graphs are in the workspace/tool locks.
- Native parser: image-tiff `10a3b6b2fa88eb5943525066a989ea9e64e644d5`.
- zenpixels: `3286169c7ef5b1e66ac841c3d8204eb54e390f8c`.
- Rust/Cargo: 1.99.1. Native oracle: ImageMagick 7.1.2-18, TIFF delegate.

The TIFF is authored in test source, not copied from an encoder: classic
little-endian, uncompressed Gray8, a 32×32 tiled root with 16×16 tiles, a reduced
16×16 SubIFD and a second 16×16 root. Discovery reads directory metadata but no
sample tile. Repeated root ROIs target separate sample tiles; reduced-level ROI
checks its own coordinate/sample identity. The independent oracle checks all
1,024 first-root samples. It does not qualify arbitrary TIFFs or the entire
native library.

## Accepted checks

Run from `reimagined/` using [justfile](../justfile). Builds used separate output
directories for the workspace and native tool, with a 4G hard cgroup limit and
two build jobs. Peak lines below describe the validation processes, not the
codec's memory consumption or a usable codec memory bound.

| Command | Result | Resource-wrapper receipt |
|---|---|---|
| `just test` | Existing 21/21 blocking controls; new 11/11 document controls; original doctest passes | `rc=0 1s; peak-RSS 0.19GiB; min-avail 10965MiB; peak-load 0.09` |
| `just clippy` | Strict all-target/all-feature workspace Clippy passes | `rc=0 1s; peak-RSS 0.15GiB; min-avail 10943MiB; peak-load 0.23` |
| `just check-core` | Core without default features passes | `rc=0 0s; peak-RSS 0.16GiB; min-avail 10939MiB; peak-load 0.17` |
| `just docs` — Rustdoc | All-feature workspace docs pass with warnings denied | `rc=0 1s; peak-RSS 0.18GiB; min-avail 10954MiB; peak-load 0.13` |
| `just docs` — doctests | Original runtime doctest and new consumer example compile | `rc=0 0s; peak-RSS 0.16GiB; min-avail 10957MiB; peak-load 0.06` |
| `just tiff-test` | 3/3 native controls, including independent libtiff samples | `rc=0 1s; peak-RSS 0.22GiB; min-avail 10771MiB; peak-load 0.09` |
| `just tiff-clippy` | Strict all-target native-tool Clippy passes | `rc=0 1s; peak-RSS 0.16GiB; min-avail 10953MiB; peak-load 0.16` |

The final TIFF Clippy applied one mechanical collapsed-if cleanup after the
passing behavioral run; no pixel path or expectation changed. The consumer
doctest was added after the full workspace run and checked separately. Earlier
successful evidence is reused for unchanged behavior rather than repeatedly
running the same campaign after documentation/commit changes.

## Failures retained

The candidate initially accepted Done without complete rows/finish and swallowed
sink failures. Both adversarial controls failed before the guard repair. Later,
pre-cancelled discovery and selection controls caught retained native owners;
both failed first, then passed after terminal cleanup. Existing assertions were
not weakened. Strict lint attempts also found mechanical fixture/native lints,
which were corrected without changing expected behavior.

The native transport teardown returned Interrupted on closure; `read_exact`
retried indefinitely. The owned failing test was terminated after 109 seconds.
The [full output](../benchmarks/native_tiff_2026-11-11.log),
[original bridge](../benchmarks/native_bridge_before_2026-11-11.rs.txt), and
[command/source metadata](../benchmarks/native_tiff_2026-11-11.meta.json)
preserve the failure. Closure now returns BrokenPipe, and the passing native
control retains the original PermissionDenied source payload and completes
worker teardown. The bridge still cannot preempt an active native call or a
blocked transport.

FFmpeg 8.0.1's pipe demux rejected the tiled fixture. File input exited successfully
but produced zero samples: 1,020 of 1,024 differed from the expected image. This
is retained as an unsuitable-oracle finding, not accepted as pixel validation.
Inspection of [FFmpeg's TIFF decoder](https://github.com/FFmpeg/FFmpeg/blob/n8.0/libavcodec/tiff.c#L2050)
shows the tiled decode branch handling DNG/JPEG while this plain uncompressed
TIFF reaches the strip path. ImageMagick/libtiff read the same authored bytes
and matched all 1,024 samples. No sample expectation, source tile or native pixel
path was changed to accommodate FFmpeg.

One attempted exact test filter ran zero tests; that dispatch is not evidence.
All final receipts above name runs that actually executed the intended checks.
Full attempt logs and SHA-256 inventory are preserved at
`/mnt/v/output/zencodec/reimagined-documents-2026-11-11/`.

## Limits

Only local Linux qualification was run for the standalone experiment. Main's
CI targets do not by themselves execute this nested workspace. Native codec
cohorts, other platforms, freestanding targets, async document controls and
performance are unqualified. No benchmark speedup or reduced application memory
is claimed.
