# zencodec-io driver per-step overhead, 2026-10-15

Follow-up to the "zencodec-io drivers" section of
`streaming_regress_2026-10-14.md`: PR 152 (in `ecca0caa`) had rebuilt the
image drivers on the `ByteStepper` cursors, leaving ~+868 instructions per
decode on `drive_from_slice` and a coroutine wall-time regression of +6.3%
at 256x256/4-row and +3.1% at 1024x1024/1-row. This change fuses the
decode step loop back into the driver entry points — `decoder.step`
directly in `drive_from_slice`, `drive_from_read_at` and
`pump_read_to_sink` — keeps the cursors' `offer`/`take`/`serve` as
`pub(crate)` primitives (`next` still composes them for `ByteStepper`
users), and cuts the coroutine decoder's `spin::Mutex` acquisitions from
five to three per step (one absorb, one post-poll harvest that also
refreshes cached info, one spare write-back; `accepts_provisional` is
cached on the decoder at `begin`).

Host: dev (Ryzen 9 9950X3D), cores 2-7 for wall time (`taskset`,
`--cpu-policy=unrestricted`), core 4 for `perf stat`. Probe:
`zencodec-io/examples/drvprobe.rs` (256x256 RGBA8, 4-row strips; arms
`direct|slice|pump|at|coro|tokio`), instructions per decode =
(count at 60,000 − count at 20,000) / 40,000, `just io-instr-probe`.
Baselines: pre-refactor main `08e8f195` (`~/tmp/zc-drivers/main`) and
PR-152-as-fixed `a8b35aa0`≡`ecca0caa` for this crate
(`~/tmp/zc-drivers/pr3`), each built with the same probe.

## Instructions per decode (drvprobe, 256x256/4-row)

| shape | main `08e8f195` | PR 152 | this change | vs main | vs PR 152 |
|---|---:|---:|---:|---:|---:|
| direct (control) | 49,656 | 49,657 | 49,651 | −5 | −6 |
| `drive_from_slice` | 57,866 | 58,758 | 57,371 | **−495** | −1,387 |
| `pump_read_to_sink` | 74,166 | 74,275 | 73,897 | **−269** | −378 |
| `drive_from_read_at` (coroutine ranges) | 258,622 | 267,898 | 257,758 | **−864** | −10,140 |
| coroutine sequential via `drive_from_slice` | 754,429 | 756,310 | 748,469 | **−5,960** | −7,841 |
| tokio `decode_async` inline | 978,511 | 977,706 | 976,607 | **−1,905** | −1,099 |

Driver overhead (`drive_from_slice` − direct) is 7,720 instructions, vs
8,210 on main and 9,101 on PR 152 in this build — the +868 left after
`a8b35aa0` is gone and the loop is cheaper than either reference.

## Wall time (zenbench `drivers` bench, 3 alternating runs per arm)

Medians of the per-run medians, µs; `m/main` and `m/pr3` are this change
against each reference. The `direct` control moves ~8% between the
main-built and newer binaries on the 1-row group, so that group's raw
deltas are code placement; net of the control every arm is flat or faster.

| group | driver | main | PR 152 | this | m/main | m/pr3 |
|---|---|---:|---:|---:|---:|---:|
| 256x256/4-row | coroutine sequential | 19.1 | 19.9 | 18.3 | **−4.5%** | −8.4% |
| 256x256/4-row | coroutine ranges | 18.9 | 19.3 | 17.6 | **−6.6%** | −8.6% |
| 256x256/4-row | drive_from_slice | 5.5 | 5.5 | 5.4 | −0.7% | −0.7% |
| 256x256/4-row | pump | 7.3 | 7.6 | 7.4 | +0.9% | −3.0% |
| 256x256/4-row | tokio inline | 8.6 | 8.4 | 8.5 | −0.7% | +1.8% |
| 1024x1024/16-row | coroutine sequential | 321.8 | 319.3 | 308.8 | **−4.1%** | −3.3% |
| 1024x1024/16-row | coroutine ranges | 248.1 | 247.7 | 239.7 | **−3.4%** | −3.2% |
| 1024x1024/16-row | drive_from_slice | 107.4 | 105.6 | 106.5 | −0.8% | +0.9% |
| 1024x1024/16-row | pump | 151.4 | 149.6 | 151.4 | +0.0% | +1.2% |
| 1024x1024/16-row | tokio inline | 159.2 | 157.6 | 159.2 | +0.0% | +1.0% |
| 1024x1024/1-row | coroutine sequential | 366.4 | 383.2 | 360.3 | **−1.7%** | −6.0% |
| 1024x1024/1-row | coroutine ranges | 315.8 | 343.5 | 301.3 | **−4.6%** | −12.3% |
| 1024x1024/1-row | drive_from_slice | 127.5 | 137.5 | 137.6 | +7.9%* | +0.1% |
| 1024x1024/1-row | pump | 169.4 | 180.1 | 179.7 | +6.1%* | −0.2% |
| 1024x1024/1-row | tokio inline | 179.0 | 186.1 | 188.8 | +5.5%* | +1.5% |
| 1024x1024/1-row | direct (control) | 120.7 | 130.1 | 130.6 | +8.2%* | +0.4% |

*same shift as the untouched control arm — code placement between
separately linked binaries, not the changed path (all these shapes run
fewer instructions than main, above).

The coroutine wall-time regression noted on 2026-10-14 is reversed:
sequential is −4.5%/−1.7% vs main where it was +6.3%/+3.1%, and ranges is
−3.4% to −6.6% vs main. Five `spin::Mutex` acquisitions per step were the
serial cost; three remain (absorb, harvest, spare write-back).

Commands:

```
cargo build --release -p zencodec-io --features tokio --example drvprobe
just io-instr-probe            # or the loop in the justfile
cargo build --release -p zencodec-io --features tokio --benches
taskset -c 2-7 target/release/deps/drivers-* --cpu-policy=unrestricted
```

Raw logs: `~/tmp/zio-perf/` (`instr-*.txt`, `ab/*.llm` + `ab/*.log`).
