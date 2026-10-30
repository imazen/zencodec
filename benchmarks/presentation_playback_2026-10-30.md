# Independent MP4 edit playback — 2026-10-30

**Unqualified: 7 of 12 cases disagree with FFmpeg 8.0.1 playback.** This is an
interoperability finding, not proof that valid edit-list records are wrong or
that every player behaves the same. No mismatch was excluded or accepted.

The native writer emits progressive and fragmented MP4 for identity, clipping,
repetition, leading gap, dwell and double rate. Independent FFprobe timestamps,
final frame end and FFmpeg/libdav1d GBR samples are checked against explicit
expected frame selections and previously verified native lossless samples.

| Edit | Progressive | Fragmented |
|---|---|---|
| Identity | exact samples/timing | exact samples/timing |
| Clip | exact samples/timing | 3 frames instead of 1; PTS -40, 0, 40 ms |
| Repeat | exact samples/timing | original 3 frames instead of repeated frame 0 |
| Leading gap | exact samples/timing | extra frame at 120 ms; ends 160 instead of 120 |
| Dwell | 2 frames instead of held frame 1 | all 3 source frames |
| Double rate | 2 frames at 0, 40 ms instead of 3 at 0, 20, 40 | samples exact; original timing retained |

## Attribution and contract consequence

The pinned [FFmpeg 8.0.1 MOV reader](https://github.com/FFmpeg/FFmpeg/blob/n8.0.1/libavformat/mov.c)
disables advanced edit handling for fragmented inputs (lines 5173–5184).
Its edit-record reader stores rate, but the edit-index helper consumes only
media time and duration. These explain the observed limitations; they do not
establish general playback equivalence or validate every emitted box.

The [W3C MSE ISO BMFF profile](https://www.w3.org/TR/mse-byte-stream-format-isobmff/#initialization-segments)
requires support for one normal-rate edit, with finite or open duration. That
requirement does not establish support for arbitrary repetition, gaps or dwell,
and MSE is a specific consumer profile rather than all ISO BMFF playback.

The API review must distinguish preserving declared edits from rendering them
into samples/timestamps, and from compatibility with a selected playback target.
The retained edit-list API remains necessary for codec-agnostic inspection and
remuxing. A successful mux admission cannot certify an arbitrary player's
execution. Before freezing, qualify a route that bakes supported edits or
refuses unsupported target requirements through policy. Do not relax this oracle
or rename record retention as complete playback validation.

## Reproduction and provenance

Run `just presentation-files INPUT.ivf NEW_FILES`, then
`just presentation-playback NEW_FILES EXPECTED.raw NEW_RESULTS` through the
serialized resource wrapper. The Rust generator uses native IVF demux and MP4
mux; no external encoder or rewriting tool produces the tested containers.
The oracle retains source hashes, per-case stdout/stderr/raw outputs, full tool
versions and a cumulative results.json, returning failure for any mismatch.

Native fixture source: prior AV1 qualification's
`8-Yuv444-identity-true-bounded-false/approved.ivf`, 17x13, three frames at
0, 40, 80 ms. IVF SHA256:
`1eafbe848714a65b609c7316c905bbc7c3bcb062b72c3475462aa0792167a364`.
Reference GBR samples SHA256:
`7894797ccaac0d7c4fb904f4e4bd1f182ab09070b0e1adc8975eea57254e8c12`.
Original fixture provenance: `native_exact_mux_timing_2026-10-27.json`.

Generator source `70a3440e99b0e57f10c470338f0ef5d49e40e1d7`, native MP4
`0070de3f`, core source from the review. Native generation and strict example
Clippy pass: rc0, 14s, peak RSS 0.32GiB, minimum available 14415MiB, load 0.31.
Oracle: system FFmpeg 8.0.1-3ubuntu2, explicit libdav1d, one decoder thread;
rc1, 1s, peak RSS 0.05GiB, minimum available 13683MiB, load 0.23.
Both used 3G/jobs2 and timeouts; these are qualification receipts, not benchmarks.
An earlier oracle launch failed before comparisons because the new artifact
folder was not writable. A dedicated owned directory fixed the setup; the
failed log remains separate.

Artifacts on r7900x:
`/mnt/v/output/zencodec/presentation-playback-20261030/{playback-files,oracle}`.
Logs: `~/tmp/container-timeline-20261030/playback-oracle*.log` and the remote
native qualification's `playback.log`. No executables are archived or committed.
