# Streaming-output rollout: main vs PR heads, 2026-10-14

Interleaved paired benchmarks of each codec's main branch against its
streaming-output PR head, both compiled into one binary on zencodec
`5ed2dade` (PR 149). Harness: `perf/streaming-regress` (run `./setup.sh`,
then `cargo run --release`; `summarize.py` makes the table below).

| codec | main | PR head |
|---|---|---|
| zenjpeg | `48f83c40` | `cb449477` (PR 231) |
| zenpng | `52acca5a` | run 1 `a8d13778`, run 2 `ae40a86c`, run 3 `c8f24a26` (PR 45) |
| zenbitmaps | `7617e352` (+ return-type shim, `shim.py`) | `0aac3a5a` (PR 33) |
| zengif | `6e1bb218` | `174484bc` (PR 31) |
| IMIJ (zqi) | `57af1aa9` (+ return-type shim) | `09e44f1a` (PR 59) |

Host: dev (Ryzen 9 9950X3D), pinned to cores 2-7 (`taskset`, the 96 MiB-L3
CCD), zenbench 0.1.11 with `--cpu-policy=unrestricted` (dev's user systemd
does not delegate the cpuset controller, so zenbench marks the placement
unverified; both sides of every pair share it). Run 3: 95 s,
`run-heavy` peak RSS 0.53 GiB. Source photo: imazen-26
`1009_general_aerial-cityscape_tokyo-skytree-sumida_s23u_iso50-f2p4_20230706-144950_4000x3000.jpg`,
top-left crops; animations are 8 shifted 320x240 crops (RGB8 for APNG,
opaque RGBA8 for GIF). Raw logs: `streaming_regress_2026-10-14.pointer.md`.

## Findings

- **Outputs.** In run 3 every like-for-like encode (one-shot, rows without
  `begin`, animation without `begin`) is byte-identical between main and
  the PR. `begin` changes PNG framing only (per-strip IDATs: +36 bytes at
  1024x768, efforts 1 and 5).
- **zenpng APNG, fixed in PR 45.** Run 1 found effort 7 at +94% and effort
  2 at +15%. Honouring `rgba_to_rgb = false` had also widened RGB8 frames
  to RGBA (bigger files). Fixed in `ae40a86c`: frames with no alpha channel
  stay RGB. Run 2 then showed the one-shot low-effort path at +25% (effort
  0) and +7.5% (effort 2) with identical bytes: every frame was converted
  to RGB twice and copied once more. Fixed in `c8f24a26`. Run 3: effort 7
  at parity, efforts 0 and 2 at +1% to +3%, `begin` at parity.
- **zenbitmaps PNM one-shot, +19 ns per call (64x64, all runs).** The PR
  enforces `max_output_bytes` on the one-shot path (a review fix). Kept.
- **Noise floor.** QOI and IMIJ decode did not change in their PRs, yet the
  two compiled copies differ by up to 7.6% (QOI decode, 1024x768) and their
  sign flips between runs: code placement in one binary. Differences inside
  that range without a source change behind them are not regressions.
  zenjpeg's fixed-table encode at 64x64 (+0.1% to +2.5%, 0.4 us) is in it.
- **What streaming costs and saves** (`pr+begin` vs main, run 3): PNG
  effort 0 rows 13% to 32% faster, effort 5 rows 19% faster at 1024x768,
  effort 1 rows +7% at 64x64; PNM rows 1% to 8% faster; JPEG within 2%.

## zencodec-io drivers: main vs PR 152 (same day)

PR 152 rebuilt the image drivers on the `ByteStepper` cursors. The
`zencodec-io` drivers bench (unchanged between the revisions) was run on
main `08e8f195` and on PR 152, alternating three times per side, pinned to
cores 2-7. Medians in microseconds; the before-fix column is `ca9a4a72`,
the after-fix column `a8b35aa0`, each against its own main runs.

| group | driver | before fix: main → PR | after fix: main → PR |
|---|---|---|---|
| 1024x1024/16-row strips | direct sink writes | 87.0 → 91.0 (+4.6%) | 87.4 → 89.1 (+1.9%) |
| 1024x1024/16-row strips | hand-written via drive_fr | 91.4 → 93.1 (+1.9%) | 88.9 → 90.3 (+1.6%) |
| 1024x1024/16-row strips | hand-written via pump_rea | 138.4 → 139.8 (+1.0%) | 133.9 → 135.0 (+0.8%) |
| 1024x1024/16-row strips | coroutine sequential via  | 294.7 → 297.3 (+0.9%) | 296.0 → 292.9 (-1.0%) |
| 1024x1024/16-row strips | coroutine ranges via driv | 225.9 → 226.8 (+0.4%) | 229.0 → 231.4 (+1.0%) |
| 1024x1024/16-row strips | hand-written via tokio de | 352.7 → 353.6 (+0.3%) | 351.3 → 344.9 (-1.8%) |
| 1024x1024/1-row strips | direct sink writes | 116.3 → 111.7 (-4.0%) | 115.6 → 116.7 (+1.0%) |
| 1024x1024/1-row strips | hand-written via drive_fr | 122.6 → 127.8 (+4.2%) | 122.2 → 121.7 (-0.4%) |
| 1024x1024/1-row strips | hand-written via pump_rea | 163.4 → 169.8 (+3.9%) | 165.5 → 165.1 (-0.2%) |
| 1024x1024/1-row strips | coroutine sequential via  | 359.7 → 389.3 (+8.2%) | 360.4 → 371.6 (+3.1%) |
| 1024x1024/1-row strips | coroutine ranges via driv | 302.0 → 343.0 (+13.6%) | 310.6 → 312.6 (+0.6%) |
| 1024x1024/1-row strips | hand-written via tokio de | 389.6 → 400.0 (+2.7%) | 417.3 → 412.6 (-1.1%) |
| 256x256/4-row strips | direct sink writes | 4.9 → 5.0 (+2.0%) | 4.9 → 4.8 (-2.0%) |
| 256x256/4-row strips | hand-written via drive_fr | 5.3 → 5.9 (+11.3%) | 5.3 → 5.2 (-1.9%) |
| 256x256/4-row strips | hand-written via pump_rea | 7.3 → 8.0 (+9.6%) | 7.6 → 7.3 (-3.9%) |
| 256x256/4-row strips | coroutine sequential via  | 18.3 → 20.8 (+13.7%) | 18.9 → 20.1 (+6.3%) |
| 256x256/4-row strips | coroutine ranges via driv | 18.4 → 21.3 (+15.8%) | 19.3 → 19.8 (+2.6%) |
| 256x256/4-row strips | hand-written via tokio de | 28.3 → 29.0 (+2.5%) | 29.1 → 27.8 (-4.5%) |

Instructions per decode, 256x256 in 4-row strips (`perf stat` over 20,000
and 60,000 iterations of a probe that runs only that drive, difference
divided by 40,000):

| | main | PR before fix | PR after fix |
|---|---|---|---|
| direct sink writes | 48,398 | 48,405 | 48,409 |
| `drive_from_slice` | 57,859 | 61,597 | 58,733 |
| driver overhead | 9,461 | 13,192 | 10,324 |
| coroutine via `drive_from_slice` | 751,255 | | 747,350 |

The cursor's `next` was not inlined into `drive_from_slice`, so each
step's event went through `Progress`, `Stepped`, `Yield` and `Result` in
memory where the old loop matched it in place. `#[inline]` on the cursors'
`next`, `DecodeStepper::step`, `Yield` and the progress hook (PR 152,
`a8b35aa0`) removed most of it: +868 instructions per decode (+1.5% of the
whole decode) remain. The commit message's 10,352 came from a scratch
build of the same change; the pushed revision measures 10,324. The
coroutine sequential driver still times +6.3% at 256x256 while executing
0.5% fewer instructions than main: code placement, inside the floor above.

## Run table

| group | run 1 pr vs main | run 2 pr vs main | run 3 pr vs main | last run main → pr | last run pr+begin vs main |
|---|---|---|---|---|---|
| encode/jpeg_default/64x64 | -0.6%–-0.2% | -1.1%–-0.7% | +0.0%–+0.4% | 80.4µs → 80.6µs |  |
| rows/jpeg_default/64x64 | -1.0%–+0.3% | -1.2%–-0.7% | -0.5%–+0.1% | 81.1µs → 80.7µs | -0.3%–+0.5% |
| encode/jpeg_fixed/64x64 | +1.5%–+2.0% | +0.1%–+0.3% | +1.7%–+2.5% | 20.3µs → 20.7µs |  |
| rows/jpeg_fixed/64x64 | +0.5%–+0.9% | -0.4%–+0.4% | +1.0%–+1.7% | 20.4µs → 20.7µs | +1.8%–+2.4% |
| encode/png_e0/64x64 | -0.4%–-0.1% | -0.9%–-0.6% | +0.1%–+0.4% | 3.2µs → 3.2µs |  |
| rows/png_e0/64x64 | -0.3%–-0.0% | -0.7%–-0.4% | +0.1%–+0.4% | 3.6µs → 3.6µs | -12.9%–-12.6% |
| encode/png_e1/64x64 | -1.2%–-1.0% | +0.0%–+0.2% | -0.1%–+0.3% | 19.5µs → 19.5µs |  |
| rows/png_e1/64x64 | -1.2%–-0.8% | -0.1%–+0.3% | +0.0%–+0.3% | 19.7µs → 19.7µs | +7.2%–+7.5% |
| encode/png_e5/64x64 | -2.8%–-2.5% | +0.8%–+1.2% | -0.2%–+0.2% | 100.1µs → 100.7µs |  |
| rows/png_e5/64x64 | -3.4%–-3.1% | -0.1%–+0.2% | -0.5%–-0.1% | 100.4µs → 100.1µs | -0.6%–-0.1% |
| encode/qoi/64x64 | +0.4%–+0.7% | +0.6%–+0.8% | +0.5%–+0.8% | 6.0µs → 6.0µs |  |
| rows/qoi/64x64 | +0.6%–+0.9% | +0.3%–+0.5% | -0.2%–+0.1% | 6.2µs → 6.1µs | +0.3%–+0.6% |
| encode/pnm/64x64 | +3.8%–+5.2% | +5.4%–+6.7% | +5.2%–+6.1% | 351ns → 370ns |  |
| rows/pnm/64x64 | +0.5%–+1.2% | +0.2%–+1.1% | +1.3%–+2.3% | 1.3µs → 1.3µs | -2.1%–-0.6% |
| encode/imij_quick/64x64 | -14.5%–-14.1% | -10.5%–-10.1% | -16.0%–-14.2% | 10.7µs → 9.1µs |  |
| encode/imij_group/64x64 | -2.0%–-1.6% | -3.9%–-3.5% | -1.8%–-1.0% | 295.7µs → 295.9µs |  |
| encode/jpeg_default/1024x768 | +0.4%–+1.1% | -1.5%–-0.9% | -0.6%–+0.2% | 16.2ms → 16.2ms |  |
| rows/jpeg_default/1024x768 | +0.2%–+0.8% | -1.2%–-0.8% | -0.5%–+0.2% | 16.2ms → 16.2ms | -0.7%–+0.0% |
| encode/jpeg_fixed/1024x768 | -0.8%–-0.5% | +0.4%–+0.7% | +0.3%–+0.8% | 2.6ms → 2.6ms |  |
| rows/jpeg_fixed/1024x768 | -0.7%–-0.1% | -0.3%–+0.2% | +0.2%–+0.6% | 2.6ms → 2.6ms | -0.8%–-0.3% |
| encode/png_e0/1024x768 | -0.0%–+0.2% | +0.0%–+0.3% | -0.1%–+0.1% | 104.0µs → 103.4µs |  |
| rows/png_e0/1024x768 | -0.2%–+0.3% | +0.2%–+0.5% | -0.1%–+0.1% | 152.6µs → 152.7µs | -32.5%–-32.4% |
| encode/png_e1/1024x768 | -0.8%–-0.2% | -0.4%–+0.0% | -0.5%–+0.0% | 1.5ms → 1.5ms |  |
| rows/png_e1/1024x768 | -0.6%–-0.0% | -0.6%–-0.3% | +0.1%–+0.4% | 1.5ms → 1.5ms | +0.6%–+1.0% |
| encode/png_e5/1024x768 | +0.2%–+0.4% | -0.3%–-0.2% | -0.2%–-0.1% | 22.9ms → 22.8ms |  |
| rows/png_e5/1024x768 | +0.2%–+0.4% | -0.4%–-0.2% | -0.3%–-0.2% | 22.9ms → 22.8ms | -18.9%–-18.8% |
| encode/qoi/1024x768 | +0.1%–+0.9% | -3.0%–-2.3% | -0.2%–+0.4% | 3.5ms → 3.5ms |  |
| rows/qoi/1024x768 | +0.5%–+1.1% | -2.3%–-1.5% | -0.7%–+0.2% | 3.5ms → 3.5ms | -0.8%–-0.3% |
| encode/pnm/1024x768 | +0.2%–+0.4% | -0.2%–+0.1% | +0.0%–+0.2% | 70.5µs → 70.8µs |  |
| rows/pnm/1024x768 | -0.4%–-0.1% | +0.1%–+0.4% | +0.2%–+0.4% | 169.5µs → 169.9µs | -8.6%–-7.5% |
| encode/imij_quick/1024x768 | -1.6%–-1.2% | +1.0%–+1.4% | -1.9%–-1.6% | 3.4ms → 3.4ms |  |
| encode/imij_group/1024x768 | +1.4%–+1.8% | -0.7%–-0.0% | +1.1%–+1.5% | 38.4ms → 39.0ms |  |
| animation/apng_e0/320x240x8 | -16.0%–-15.3% | +24.4%–+25.4% | +2.1%–+3.1% | 3.3ms → 3.4ms | -1.0%–-0.1% |
| animation/apng_e2/320x240x8 | +14.9%–+15.1% | +7.5%–+7.6% | +1.0%–+1.3% | 11.8ms → 11.9ms | +0.1%–+0.4% |
| animation/apng_e7/320x240x8 | +93.5%–+93.8% | -0.2%–+0.1% | -0.2%–+0.1% | 76.3ms → 76.1ms | -0.1%–+0.1% |
| animation/gif/320x240x8 | -0.1%–+0.1% | -0.2%–+0.0% | -0.2%–+0.0% | 64.6ms → 64.9ms | -0.1%–-0.0% |
| decode/jpeg/64x64 | +0.3%–+0.5% | -0.8%–-0.5% | +1.0%–+1.8% | 13.1µs → 13.3µs |  |
| decode/png/64x64 | -0.2%–+0.0% | -0.2%–+0.0% | -0.2%–+0.1% | 17.7µs → 17.7µs |  |
| decode/qoi/64x64 | -0.1%–+0.3% | -5.4%–-5.2% | -1.1%–-0.3% | 4.3µs → 4.3µs |  |
| decode/imij/64x64 | +1.5%–+2.2% | +2.4%–+3.2% | +1.2%–+2.8% | 59.3µs → 59.3µs |  |
| decode/jpeg/1024x768 | -1.2%–-0.8% | +0.5%–+0.9% | -0.1%–+0.6% | 1.2ms → 1.2ms |  |
| decode/png/1024x768 | -2.6%–+2.7% | -2.4%–+3.3% | -0.2%–+0.2% | 3.2ms → 3.2ms |  |
| decode/qoi/1024x768 | -2.8%–-2.4% | +7.3%–+7.6% | +5.5%–+5.9% | 2.2ms → 2.3ms |  |
| decode/imij/1024x768 | +0.3%–+0.7% | +0.7%–+1.2% | +0.2%–+0.7% | 3.2ms → 3.2ms |  |
