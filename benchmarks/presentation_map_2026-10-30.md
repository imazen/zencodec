# Exact presentation mapping experiment

Private source `a7a6ee73ec611652dd564bf15b77711101c0f5a2` in
`perf/presentation-map`, not a public API or integrated playback implementation.
Archive SHA-256 `d158e3a69a84eb6ca3faac23cf2f722ce72f9ec6afb0835e5aaef04083c18f2c`;
dependency lock SHA-256 `4df91e3c0425c8c4c0f1c022785bbeae8612da58816d54de78517539666ca985`.

`just presentation-map-check` reproduces the two locked Cargo commands:
tests followed by strict all-target Clippy. The isolated ARM64 run used a
3 GiB/two-job cap, 300-second timeout, debug info disabled and incremental
compilation disabled. Seven tests passed, none ignored or filtered; strict
Clippy passed. Compiler output names the frozen export. Hashes were verified
after execution; supervisor and Rust processes exited. Full local logs:
`~/tmp/presentation-map-20261030/final.log`.

Receipt: `rc=0 6s | peak-RSS 0.31GiB | min-avail 14583MiB | peak-load 0.12`.
The earlier version without exposed acquisition requests also passed seven
tests and Clippy: `rc=0 14s | peak-RSS 0.31GiB | min-avail 14501MiB | peak-load 0.24`.
These are qualification resources, not performance or workspace-bound claims.

Controls cover repeated media on distinct clocks, clipped crossing frames,
negative output origin, double/fractional playback rates, explicit gaps,
half-open dwell selection, reverse-rate/invalid-duration refusal, exact
destination-clock conversion and an integer-grid rational oracle. Acquisition
requests explicitly distinguish source spans, one frame and no decoding.

No native frames, seek, reorder, cancellation, delay/discard integration,
independent playback, or caller resource policy was exercised. No API migration
is justified by these tests alone. Next decisive gate: retain a native MP4 edit
program without scalar loss, drive acquisition in segment order, and compare
actual presentation against an independent player while preserving sample bits.
