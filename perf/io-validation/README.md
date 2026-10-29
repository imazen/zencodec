# I/O validation experiments

Current policy, disclosure, execution and stream harness boundaries accept
optional borrowed Pulse controls. They preserve absence through public
forwarders and resolve a no-op only for local checks/private helpers. The source
migration has not been compiled or run; existing receipts retain their original
revision scope.

Private harnesses for the [2026-10-24 investigation](../../benchmarks/io_validation_2026-10-24.md).
They add no library API. Run builds and benchmarks serially under your host's
resource limiter. The disk probe uses Linux/Unix positional file I/O; the
batching script uses Linux `taskset`. These are experiments outside the normal workspace build. The Documentation
workflow runs the guide examples, encoder retention control, and transport model;
the larger performance and container sweeps remain opt-in.

`setup.py` fetches text sources from pinned public commits, checks previously
downloaded files before writing, and records each source URL and SHA-256 in
`vendor/sources.json`. It does not read or modify sibling checkouts. The
vendor tree is ignored. The committed lock snapshots preserve the resolved
dependencies used for the measurements (including distinct Tokio patch
versions for the root workspace and this standalone harness).

## Prepare an export

Run from the repository. Use a fresh directory outside any parent Cargo
configuration that adds local path patches. All generated media and outputs
belong in the experiment directory, never in git.

```sh
experiment="$HOME/tmp/zencodec-io-validation"
test ! -e "$experiment"
mkdir -p "$experiment/source" "$experiment/scratch"
git archive origin/main | tar -x -C "$experiment/source"
cd "$experiment/source"
export TMPDIR="$experiment/scratch"
export CARGO_INCREMENTAL=0 CARGO_TARGET_DIR="$experiment/source/target"
cp perf/io-validation/locks/workspace.lock Cargo.lock
cp perf/io-validation/locks/harness.lock perf/io-validation/Cargo.lock
just io-validation-setup
just io-validation-test
just io-validation-clippy
```

## Correctness and performance

```sh
just io-contract-probes
just io-validation-batching "$experiment/batching" 2,3 synthetic
just io-validation-batching "$experiment/batching-qoi" 2,3 qoi
just io-validation-disk "$experiment/disk-fixture.bin" 2,3
cargo build --locked --release --manifest-path perf/io-validation/Cargo.toml --bin disk_probe
strace -c -e read,pread64,lseek "$CARGO_TARGET_DIR/release/disk_probe" seek 100 "$experiment/disk-fixture.bin"
strace -c -e read,pread64,lseek "$CARGO_TARGET_DIR/release/disk_probe" positional 100 "$experiment/disk-fixture.bin"
cargo run --locked --manifest-path perf/io-validation/Cargo.toml --bin io-validation
```

Choose CPUs appropriate to your host. The reported disk measurements are
warm page-cache results, with both implementations interleaved in one
zenbench process. They do not measure cold storage. The batching script
builds and hashes three binaries, preserves them outside Cargo's cache,
runs exact-output/stalled-writer probes, and reverses binary order in the
second timing pass. Results across binaries are exploratory comparisons,
not paired confidence intervals. Timed QOI transcodes include the exact
encoded-byte comparison in every arm.

The QOI tests cover pixel-aligned strided input, small read boundaries,
async decoding, and acceptance/pixel parity for every truncated prefix of
an authored fixture. Both codec paths accept a missing QOI trailer; the
test compares their behavior rather than asserting every prefix is invalid.
It is a transport equivalence test, not an independent QOI conformance suite.

## Video containers

The pinned scripts require FFmpeg/ffprobe with libaom, libvpx, libopus and
libx264. The strict oracle requires the full fixture set; missing H.264
fixtures fail its expected-pair count. It checks all mapped streams,
including audio, and compares successful decodes rather than partial output.

```sh
bash perf/io-validation/vendor/zenmp4/scripts/ffmpeg-oracle.sh "$experiment/mp4-oracle"
bash perf/io-validation/vendor/zenwebm/scripts/ffmpeg-oracle.sh "$experiment/webm-oracle"
just io-validation-oracle "$experiment/mp4-oracle" "$experiment/webm-oracle" "$experiment/strict-oracle"
just io-validation-transport "$experiment/mp4-oracle" "$experiment/webm-oracle" "$experiment/ivf-oracle"
```

`media_transport` compares full track descriptions, packet bytes, timing,
side data and other events against slice input. Its async sources alternate
`Pending` and short reads. Random-access differences fail. Forward-source
refusals are printed as observations, not silently skipped or enshrined as
permanent expectations. These are simulated transports, not HTTP tests.

## Disposable private prototypes

The prototype scripts refuse a git/jj checkout. Run them only in the export.
`prototype.py` saves the original Tokio driver once and restores that exact
source on `baseline`; `run-batching.sh` also restores it on exit.

```sh
python3 perf/io-validation/prototype.py . bounded
cargo test --locked -p zencodec-io --all-features
python3 perf/io-validation/prototype.py . baseline
python3 perf/io-validation/window-prototype.py .
cargo run --locked --manifest-path perf/io-validation/Cargo.toml --bin media_transport -- \
  "$experiment/mp4-oracle/av1_opus_faststart.mp4" "$experiment/mp4-oracle/av1_opus_frag.mp4"
python3 perf/io-validation/prototype.py . baseline
```

The batching thresholds are experimental, not selected defaults or hard
memory caps. A decoder step is indivisible, and an encoder may buffer a
whole frame. The window prototype handles only ranges fully contained in
the active window; partial overlaps, suffix retention and the synchronous
driver still need implementation and adversarial checks before landing.

## Qualification after the initial prototypes

The [implementation qualification report](../../benchmarks/io_contracts_2026-10-24.md)
records the subsequently landed synchronous/async overlap handling and actual
`FileReadAt` implementation. Use `disk_probe file` for that implementation;
`seek` remains the frozen baseline and `positional` the original experiment.
The executable storage/ordering candidate is checked with
`just io-admission-check` and measured with `just io-admission-bench 2,3`.

`just io-validation-disk-cold <file> 2,3` adds a Linux cache-miss experiment.
It advises eviction of that file before each sample, without changing the
system-wide cache. Every sample must charge at least the requested data bytes
to `/proc/self/io`'s `read_bytes`. Eviction and verification run outside the
timer. This verifies kernel block-read accounting, not a cold device/controller
cache. Keep these results separate from the warm benchmark.

`just io-validation-disk-profile <new-output> <file> 2,3` records fixed-work
`perf stat` counters and heaptrack traces for both disk adapters. It compares
256 and 1280128 reads, including initial verification, with the same reusable
4096-byte output. Preserve the complete output directory outside git; the
qualification report links the compact counters and heap summary.

The [complete evidence pointer](../../benchmarks/io_evidence_2026-10-24.md)
records the archived logs, fixtures, binaries, counters and traces with a
manifest and verified archive hash. `just io-validation-pack-evidence <dir>
<archive>` packages an explicitly selected evidence tree and verifies every
decompressed file. Keep large results and binaries outside git.

## Situational policy resolution

`just io-policy-resolution-check` retains the initial authored-choice control.
The current private typed/fallible resolver is under `src/policy/`:

- `just io-policy-engine-check`: requirements, scoped choices, traces, joint
  plans, concurrent admission, real payload reservations and lifetime checks.
- `just io-policy-native-check`: stored profiles through native QOI/BMP and
  CodecSet, independently read EXIF, exact strided pixels, failure causes and
  accepted-byte progress. Also executes `docs/examples/policy.rs`.
- `just io-policy-media-check <mp4> <webm> <ivf>`: native container packets over
  resident/disk/reused transport-model sources, track omission and epoch controls.
  Fixture arguments are required; use the video-container preparation above.
- `just io-policy-portability-check`: WASM plus an isolated freestanding core.
- `just io-policy-cost-build`, `just io-policy-bench 2,3`, and
  `just io-policy-allocations <new-directory>`: common dispatch/error workloads
  and serial heaptrack captures. The allocation script requires rustfilt.

See the [scenario matrix](../../docs/policy-scenarios.md),
[measured costs](../../benchmarks/policy_cost_2026-10-25.md), and
[proposed contract](../../docs/policy-resolution.md). All types remain private
experiments. The harness does not implement a generic disk spool or establish
native whole-operation memory bounds. Logical fixture costs are not measured RSS.
