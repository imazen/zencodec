# Bounded 4096² and 8192² comparison

`run.py NATIVE_PROBE SYNTHETIC_PROBE OUTPUT` performs exactly one diagnostic:
three native RGB8 witnesses and one synthetic RGBA8 witness at each size,
through every listed harness arm. The native unchanged A/A control and synthetic
`direct_AA` are retained. Each case uses `None` Pulse. The synthetic case uses
64-row strips and a 64-step quota at both sizes.

The controller first creates durable fixtures and checks exact outputs through
every route. Synthetic output includes 16-byte row-padding controls. It then
calibrates from one extra decode, doubling only until the paired adjusted loop
lasts at least 500 ms (8192-decode ceiling; unresolved calibration fails).
Calibration ends before the fixed five-block shuffled measurement plan. Every
measurement subtracts a zero-extra process with the same file load, runtime
startup and one warm decode. External `perf stat` records wall, user, kernel,
total CPU, task-clock, context switches and CPU migrations. No per-decode process
launches and no timers inside codecs. All raw controls and negative counters
survive. The controller fails on source drift and records binary hashes.

Run under the published zenbench exclusive lease, its performance placement
executor and `run-heavy`; also acquire the legacy benchmark flock. This is an
external profiler diagnostic, not zenbench Confirm or a latency acceptance
verdict. A/A spread is the maximum same-block absolute difference between the
unchanged reference and its identical control, reported separately for wall and
CPU. Smaller differences remain unresolved. Retain flagged or interrupted data;
do not rerun measured cells to obtain a passing result.

Streamed readers and positioned Cursor readers are memory-backed. These results
do not measure disk/network throughput, gigapixel browsing, concurrent request
fairness or worker replacement. Tokio uses two runtime workers, but calls are
made from `Runtime::block_on` outside a worker task. Consequently the InPlace
route measures its admission and inline execution here, not the scheduler cost
of replacing a blocked runtime worker.

The run directory records stage, source/binary identity, exact commands, child
PID, completed/required pairs, calibration counts, raw counters and fixtures.
Before reporting progress inspect this record and verify its live child PID.

`just large-report RUN` renders every cell and the native resident-path
contrasts into a bounded Markdown report and compact JSON receipt, then creates
a filterable HTML page in the run directory. It refuses incomplete studies and
checks the expected 670 pairs / 134 cells. The HTML states the distinction
between the minimal measured sink and the public collecting convenience call.
