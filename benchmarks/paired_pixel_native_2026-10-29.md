# Paired pixel native qualification, 2026-10-29

This is integration qualification, not a codec throughput benchmark.

Pixel source: f23ac753. Core base: fc3724a8 plus the paired pixel manifest
rollout. Native IMI source: e3b14aa2 (only shared dependency ranges changed
from daa9c13d). JXL encoder: 3c1be817, replacing the older harness pin0255e5a9
whose converter requirement excluded the paired version.

- IMI: 25 native multipass/public-driver tests and strict all-target Clippy pass.
  Corrected run rc0,10s,peakRSS0.67GiB. First resolution retained an old compatible
  pixel lock entry; targeted update unified it. The first runtime runner omitted
  MULTIPASS_SCRATCH and failed five cases. Both failures are retained; supplying
  admitted disk scratch fixes the runner without changing assertions.
- JXL planned color:10 tests pass; strict all-target spatial-harness Clippy passes.
  rc0,68s,peakRSS0.74GiB,minAvailable10518MiB,peakLoad1.57.
  Two assertions that the now-removed OutputMetadata.hdr was None were retired;
  all sample, ICC/CICP, allocation and storage assertions remain unchanged.
  The paired API cannot carry that removed field.

Commands (isolated export, run-heavy3G/jobs2, timeout900):

```
MULTIPASS_SCRATCH=<admitted-disk-directory> cargo test --manifest-path perf/multipass-validation/native/Cargo.toml --locked -- --test-threads=1
cargo clippy --manifest-path perf/multipass-validation/native/Cargo.toml --all-targets --locked -- -D warnings
cargo test --manifest-path perf/spatial-validation/Cargo.toml --lib jxl_planned_color_tests --locked
cargo clippy --manifest-path perf/spatial-validation/Cargo.toml --all-targets --locked -- -D warnings
```

The IMI runtime trial used a local immutable export. The equivalent published
git source subsequently passes all-target compilation; all standalone lockfiles
now resolve to one pixel family. The video witness uses AVIF94116d26 and
UltraHDR7e9d296c, matching the AV1 harness; two runtime controls and strict
Clippy pass. Combined video/catalog wrapper rc0,64s,peakRSS1.00GiB,
minAvailable10345MiB,peakLoad1.52. All thirteen standalone graphs compile.
The new `just paired-harness-check` reproduces their target checks after setup.
No required platform or whole-corpus codec parity claim is made here.

## JXL intrinsic-size regression follow-through

The complete documentation CI on e1c0ca5b exposed one failure in 147 spatial
controls: libjxl rejected the generated auxiliary fixture before basic metadata.
The pinned encoder 3c1be817 wrote intrinsic dimensions as bare width/height
integers, omitting SizeHeader's small flag and ratio. Current encoder main
already fixes this in 5923e603; the current paired stack 55c59fe7 includes it.
No duplicate encoder patch or changed test expectation is needed.

The unchanged seven-role/HDR/intrinsic-size test passes with 55c59fe7 against
the harness's pinned libjxl 0.11.1 decoder, and all-target strict Clippy passes
(rc0, 51s, peak RSS 0.73GiB). The lock delta is the three encoder packages plus
hashbrown 0.16.1 to 0.17.1, explicitly required by the new encoder manifest.
No core/pixel/decoder source changed. The old failure is retained in
`core-ci-failed.log` and `jxl-aux-current2.log`; the first local attempt stopped
before encoding due to artifact-directory permissions (`jxl-aux-current.log`).
All logs live under `~/tmp/zenpixels-release-reconcile-20261029`.

The full unchanged `just spatial-ci-check <fresh-artifacts> cms_oracle,old_jpeg`
then passes 147/147 tests, zero ignored, plus strict all-target Clippy. Receipt:
`run-heavy: done rc=0 58s | peak-RSS 0.56GiB | min-avail 11143MiB | peak-load 1.91`.
The command retains the ten-minute test and five-minute lint timeouts. Full log:
`spatial-current-stack.log`; generated files are retained under
`/mnt/v/output/zencodec/spatial-current-stack-20261029`.
