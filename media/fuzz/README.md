# Media contract fuzzing

Targets cover bounded IVF parsing/writing assumptions, rational time, plane
geometry and native AV1 decode/map/owner lifetime, indexed timestamp extraction,
and native lossless encode/decode roundtrips. Decoder frame area and packet
size are bounded; each input produces at most 32 presentation frames.

Build with an explicit native target if cargo-fuzz was installed for another
target (for example musl):

```sh
cargo +nightly fuzz build --target x86_64-unknown-linux-gnu
```

Seed the parser/decoder targets' ignored `fuzz/corpus/<target>/` with copies of
`corpus/av1/*`. The `native_encode` target consumes parameter bytes followed by
original synthetic sample bytes: first eight bytes select dimensions, depth,
layout, range/packing, count, preset and reordering; at least one payload byte
follows. It checks every native decoded sample, including padding-bit isolation.
Run targets sequentially with memory and time limits, storing logs outside the
repository. Example, from the repository root:

```sh
ASAN_OPTIONS=detect_odr_violation=0 \
  fuzz/target/x86_64-unknown-linux-gnu/release/native_av1 \
  fuzz/corpus/native_av1 -max_total_time=3600 -max_len=65536 \
  -timeout=10 -rss_limit_mb=2048 -print_final_stats=1 \
  -artifact_prefix=fuzz/artifacts/native_av1/
```

Create the ignored corpus/artifact directories first. Use the workspace's
resource wrapper when running on a shared workstation. Preserve source revision,
toolchain, binary SHA-256, options, initial corpus hashes, elapsed time and final
statistics for each epoch. A requested duration is not a completed duration.
Working corpora and raw artifacts stay outside git and should be backed up to
the workspace fuzz store. A minimized fixed regression belongs with its test.

Initial ASan smoke, 2026-09-27, foundation revision `e806fac`: each target ran
approximately 624 seconds before an explicit interrupt for the next build.
Container contracts executed 100,001,982 inputs; native AV1 executed 598,350.
No failure was reported. These counts establish only the exercised smoke run,
not exhaustive correctness or a completed one-hour epoch.

Completed ASan `native_encode` epoch at revision `f7b48029c1c2c22c907544d1c3fc0fba2c0c3780`:
3,601 seconds, 21,642 executions, 2,931 new corpus units, 579 MiB peak RSS,
no reported failure. Seed 20260929; `max_len=8192`, `timeout=10`, `rss_limit_mb=2048`.
The binary SHA-256 is
`58aa4a6926e295caa8ad9bdb84b74ea7153b3009ed4517be178bd27195340e86`.
This validates the recorded native encode/decode target only; later display and
animation additions were not part of that binary.
