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

## Open-ended extension

Source `18fdd6faa9b6ad77febd2b346e759bd7796738db`, archive SHA-256
`f44c54bee57f645dc2a7007fb99f91f7cc5192c1323fcd5ec68c895ced3eaf18`;
dependency lock unchanged. The same locked test/Clippy commands passed 9/9
tests, zero ignored/filtered, with strict all-target Clippy on the isolated
ARM64 export. Source paths and terminal supervisor/Rust exits were verified.
Receipt: `rc=0 6s | peak-RSS 0.31GiB | min-avail 14617MiB | peak-load 0.09`.
Full local log: `~/tmp/presentation-open-20261030/full.log`.

`Extent::Remaining` preserves an unknown eventual endpoint and produces an
explicit `Acquisition::From` request. New controls exercise an offset across
future fragments, clipping before the selected source start, and refusal of a
following segment or an unbounded gap/dwell. Every previous finite assertion
remains. This is still interval arithmetic, not native fragmented playback.

An initial export failed because the local disk filled; the partial archive was
retained as `source.incomplete.tar` locally and remotely. No build ran from that
archive. After reclaiming inactive Rust intermediates, the complete replacement
archive was checked locally, hash-matched remotely and extracted successfully
before qualification. No source/test expectation changed to repair staging.


## Native frame coverage cursor

Source `8e97531e2c21e3a2f5f621046d1e6fb27275106f2` adds a per-segment
execution cursor that refuses missing/overlapping coverage and becomes terminal
on failure or completion. The cursor retains timing state, not decoded frames.
Gaps require an explicit external decision; no implicit fill is implemented.

All 11 model tests pass (no ignored or filtered tests), plus strict all-target
Clippy. The native focused control passes (1 selected, 50 filtered), plus native
harness strict all-target Clippy. It uses three real lossless AV1 frames with
exact sample comparisons, double-rate clipping, replay and dwell. A deliberately
omitted middle frame refuses instead of completing a shortened presentation.
The fixture explicitly supplies its known 40 ms source durations and retains the
three decoded frames; it does not qualify final-duration discovery, low-memory
replay, MP4 edit parsing or independent player parity. No public API changed.

The frozen archive SHA256 is
`a7525548a5519a7d35eea5a6832a62e4706a8a6bf04d9ee3db9e57d29c854172`.
The isolated lock adds only the private path experiment and harness edge; graph
validation reports 13 interface consumers and zero mismatches. Tests used the
resolved lock with no later resolution, a 3 GiB cap, two jobs and 900-second
batch timeout. The recorded run-heavy receipt is rc0, 35s, peak RSS 0.40 GiB,
minimum available 14326 MiB, peak load 1.34. This is test/build resource use,
not a codec performance or retained-memory measurement. Recorded wrapper and
Rust processes exited. Reproduce with `just presentation-native-check`.
