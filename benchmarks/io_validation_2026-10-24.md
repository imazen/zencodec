# I/O contract validation, 2026-10-24

Baseline: zencodec `87ec0d008cee0cae8ddc19de6b39f3df8018d022`.
These are diagnostic experiments, not a calibration of shipping defaults.
No library API or behavior changes are included in the probes.

## Reproduced contract gaps

Run `just io-contract-probes` with the `tokio` feature. The fixture expands
one input byte to a 256 × 1024 RGBA image. Its decoder emits either 1, 16,
or 1024 rows per step; its encoder exposes output immediately. The writer
alternates `Pending` with short writes and verifies that production stops
during every pending interval. Every output byte is checked against the
authored row values.

All twelve combinations of step size, read size (1 or 65536), and compute
placement (inline or blocking pool) produce all 1024 rows before the first
write and reach 1048576 pending encoded bytes. The pool uses two jobs.
Those are logical queue lengths, not process-memory measurements. The
writer correctly stops production once it is finally polled; the problem
is how much work the driver does before that point.

- [#160](https://github.com/imazen/zencodec/issues/160): input starvation
  controls compute batching, even when output needs draining.
- [#161](https://github.com/imazen/zencodec/issues/161): an 8-byte spool
  consumes 1024 bytes before returning `BeyondLimit { needed: 1024,
  limit: 8 }`. `for_pattern(Sequential, 8)` selects a 65536-byte window;
  a required 1024-byte lookback is silently reduced to 8.
- [#162](https://github.com/imazen/zencodec/issues/162): a one-shot-only
  encoder using the trait defaults succeeds directly but returns
  `RowLevelEncode` through `encode_to_writer`, writing zero bytes.

`zencodec-io/benches/transcode.rs` uses the same expansion fixture to measure
scheduling separately from codec work. Unlike the correctness probe, it
writes to a ready `Vec`, so it measures neither network latency nor disk I/O.

## Baseline checks and resource envelope

On r7900x, Rust 1.99 stable, exported outside workspace path patches:
workspace all-feature tests and Clippy with all targets/features pass.
Commands were serialized under `run-heavy --mem 12G --jobs 4` with a
disk-backed `TMPDIR`. Recorded wrapper lines:

```text
baseline tests: rc=0 16s | peak-RSS 0.41GiB | min-avail 15421MiB | peak-load 1.50
baseline clippy: rc=0 10s | peak-RSS 0.33GiB | min-avail 15269MiB | peak-load 0.62
contract probes: rc=0 0s | peak-RSS 0.18GiB | min-avail 16125MiB | peak-load 0.56
```

The probe wrapper includes Cargo/build work. It does not isolate decoder
heap usage. It establishes neither a memory bound for a codec nor a
network latency bound.

## Positional file reads

`perf/io-validation/benches/disk.rs` compares the current `FileReadAt`
(seek then read) with a private adapter using safe Unix `FileExt::read_exact_at`.
Both use the existing `ReadAt` trait, the same boxed dispatch, checked lengths,
and reusable output buffers. The test checks every returned byte against an
authored formula before timing. No new codec trait is needed for this adapter.

These are **warm page-cache** measurements on r7900x, Rust 1.99.1,
zenbench 0.1.11, CPUs 2,3 via `taskset`, without `target-cpu=native`.
A deterministic 64 MiB file supplies 128 sequential or dispersed reads per
iteration. Opening, initial generation and correctness checks are outside
the timed loop. Each row below is the median of three interleaved passes;
units are microseconds per 128 reads, not per read.

| Read pattern | Current seek/read | Positional read |
|---|---:|---:|
| Sequential, 256 bytes | 102.95 | 65.34 |
| Dispersed, 256 bytes | 102.96 | 65.29 |
| Sequential, 4096 bytes | 109.11 | 71.33 |
| Dispersed, 4096 bytes | 109.58 | 71.42 |
| Sequential, 65536 bytes | 280.98 | 242.40 |
| Dispersed, 65536 bytes | 317.96 | 280.23 |

The command was `IO_DISK_FILE=<fixture> taskset -c 2,3 cargo bench -p
io-validation --bench disk -- --cpu-policy=unrestricted --median-of-passes=3`
from the standalone harness. Its checked dependency graph is preserved in
`perf/io-validation/locks/harness.lock`; the root probe graph is in
`perf/io-validation/locks/workspace.lock`. Restore them in an export outside
workspace path patches. `setup.py` supplies the pinned external source trees.

A separate `strace -c -e read,pread64,lseek` run used 128 verification reads
and 100 repetitions of 128 timed-shape reads (12928 data reads):

| Adapter | read | lseek | pread64 |
|---|---:|---:|---:|
| Current | 12933 | 12931 | 2 |
| Positional | 5 | 2 | 12930 |

Counts include process startup. This confirms removal of one seek per data
read; strace time is not substituted for the zenbench timing. The tested
prototype is Unix-specific. Cold storage, queue depth, concurrent file reads,
Windows offset semantics and macOS performance are not measured here.

```text
disk comparison: rc=0 35s | peak-RSS 0.29GiB | min-avail 15910MiB | peak-load 1.58
seek syscall probe: rc=0 0s | peak-RSS 0.00GiB | min-avail 16141MiB | peak-load 0.39
positional syscall probe: rc=0 0s | peak-RSS 0.00GiB | min-avail 16137MiB | peak-load 0.36
```

The wrapper rounds the small syscall-probe RSS to 0.00 GiB; that does not
mean zero memory use. The disk comparison wrapper includes build work.

## Output batching: correctness versus scheduling cost

`perf/io-validation/prototype.py` applies private variants to an exported
source tree and restores the original afterward. It changes only transcode
batching, leaving `decode_async`'s policy alone. The variants are:

- Baseline: continue until input starvation or completion.
- Rows: yield after each step that reports rows.
- Bounded: yield after at least 64 produced rows or 65536 pending bytes.

The last policy is a trial threshold, **not a hard allocation bound**. A
decoder step cannot be interrupted at an arbitrary row; a whole-file step
still produces the whole frame. Encoders can also retain work internally.

For the 256 × 1024 fixture, both read sizes and both compute placements
preserve every output byte and stop production during a pending write.
Observed first-write and queue behavior:

| Variant | Rows/step | Rows produced before first write | Peak pending encoded bytes | Pool jobs |
|---|---:|---:|---:|---:|
| Baseline | 1 / 16 / 1024 | 1024 | 1048576 | 2 |
| Rows | 1 | 2 | 1024 | 1026 |
| Rows | 16 | 32 | 16384 | 66 |
| Rows | 1024 | 1024 | 1048576 | 3 |
| Bounded | 1 / 16 | 64 | 65536 | 18 |
| Bounded | 1024 | 1024 | 1048576 | 3 |

`EncoderSink` hands the previous strip to the encoder when asked for the
next buffer. That accounts for the first write occurring after two strips
in the per-step variant. Queue counters are logical bytes, not heap/RSS.

`run-batching.sh` builds and hashes each binary, then runs baseline/rows/
bounded and the reverse order. Inline and pool arms are interleaved inside
each binary. Values below are the means reported for the two separate
passes, in microseconds per image. They are exploratory comparisons across
binaries; no cross-binary confidence interval is claimed.

| Variant | Rows/step | Inline, pass 1 / 2 | Pool, pass 1 / 2 |
|---|---:|---:|---:|
| Baseline | 1 | 92.7 / 92.1 | 108.7 / 110.1 |
| Rows | 1 | 183.6 / 173.5 | 6966.2 / 6177.7 |
| Bounded | 1 | 109.4 / 108.5 | 268.7 / 260.9 |
| Baseline | 16 | 55.0 / 49.9 | 65.1 / 64.6 |
| Rows | 16 | 51.4 / 51.0 | 507.7 / 443.0 |
| Bounded | 16 | 59.8 / 55.2 | 207.4 / 192.4 |
| Baseline | 1024 | 53.3 / 55.3 | 79.0 / 74.3 |
| Rows | 1024 | 50.9 / 64.1 | 85.0 / 87.6 |
| Bounded | 1024 | 54.4 / 50.4 | 85.9 / 83.4 |

This rejects one worker-pool handoff per row as a general policy. Batching
materially reduces those handoffs while getting output to the writer earlier.
It does not make the added scheduling cost disappear.

### Actual QOI codec

The harness uses zenbitmaps
[`11fbd887985e59769d2aeb48295767b7d04209b9`](https://github.com/imazen/zenbitmaps/commit/11fbd887985e59769d2aeb48295767b7d04209b9).
The correctness fixture is authored 37 × 29 RGBA with a pixel-aligned
strided layout. Direct encode and drained encode have identical bytes;
one-shot, slice, stream-pump and async decode produce the authored pixels.
Slice/pump read sizes are 1, 2, 3, 7, 31, 64, 4096 and 65536 bytes; async
sizes are 1, 7, 64 and 65536. All 5387 truncated prefixes agree with the
one-shot decoder's acceptance and pixels. Both paths accept the final eight
prefixes missing some/all trailer bytes. This tests transport parity, not
an independent claim about strict QOI validity.

The timing fixture transcodes 512 × 512 RGBA QOI to QOI and checks exact
encoded bytes on every iteration. Solid pixels encode to 4255 input bytes;
changing pixels encode to 1048595. Codec/source/output setup and the byte
comparison are included equally in all timed arms; runtime construction is
outside the iteration. Read placement is in-memory, so these are scheduling
and codec measurements, not network results.

| Variant | Solid inline, µs | Solid pool, µs | Changing inline, ms | Changing pool, ms |
|---|---:|---:|---:|---:|
| Baseline | 285.8 / 286.5 | 312.4 / 305.9 | 1.2 / 1.2 | 1.4 / 1.5 |
| Rows | 287.6 / 285.9 | 368.3 / 359.6 | 1.3 / 1.3 | 1.8 / 1.8 |
| Bounded | 286.3 / 286.5 | 364.3 / 357.0 | 1.2 / 1.3 | 1.6 / 1.5 |

The printed millisecond values are rounded by zenbench; no finer precision
or significance is inferred. These two synthetic images do not select
production thresholds for other codecs. The existing driver tests also
pass with the bounded prototype, including cancellation, short reads,
write errors and stuck consumers.

The observational BMP probe confirms [#162](https://github.com/imazen/zencodec/issues/162)
with an actual codec: a 2 × 2 direct encode emits 70 bytes, while the writer
helper reports `RowLevelEncode` and emits none. A four-range coalescing
probe requests 32 bytes at offsets 0, 64, 8192 and 16384. Gaps of 0, 32,
4096 and 8192 yield respectively (4 requests, 128 bytes), (3, 160),
(3, 160) and (1, 16416). Thus a gap setting alone is neither an overfetch
budget nor a transport-independent optimization.

```text
synthetic batching: rc=0 34s | peak-RSS 0.21GiB | min-avail 15941MiB | peak-load 0.99
QOI batching: rc=0 46s | peak-RSS 0.29GiB | min-avail 15607MiB | peak-load 2.01
bounded driver tests: rc=0 1s | peak-RSS 0.28GiB | min-avail 16123MiB | peak-load 0.36
```

## Video, audio and container transport

Source pins are zenmp4 from zenavif
[`a7638f6e0655914fb15fe4633ac26425533756c8`](https://github.com/imazen/zenavif/commit/a7638f6e0655914fb15fe4633ac26425533756c8),
and zenivf/zenwebm from zenextras
[`ae920c1e2ac0fe0e6e2fcb709d99934ba36ea49c`](https://github.com/imazen/zenextras/commit/ae920c1e2ac0fe0e6e2fcb709d99934ba36ea49c).
Their zencodec dependencies are patched to the baseline export with one
shared zencodec type graph. The container contract suites pass, including
timing, malformed-input refusal, limits, seek results, forward-only mux
admission and output patch honesty.

The pinned FFmpeg oracles ran with FFmpeg 8.0.1-3ubuntu2. They generated
AV1/Opus, VP9/Opus, VP9 alpha, H.264/AAC, MPEG-4 Part 2/AAC and FLAC cases,
including faststart, tail-indexed, fragmented and live layouts:

- MP4: nine originals and eighteen progressive/fragmented copies; packet
  comparisons, eighteen decoded-content comparisons and 162 seek checks pass.
- WebM/Matroska: five originals and five copies; packet and decoded-content
  comparisons and thirty seek checks pass.
- `strict-oracle.py` independently requires successful FFmpeg exits with
  `-xerror -err_detect explode`, maps all streams, and compares ordered
  decoded size/hash pairs per stream. All 23 copies match their originals,
  with 37 complete successful decodes. This closes the partial-output risk
  in the MP4 script's `|| true` pipeline. Packet oracles separately check
  timing; this stricter content comparison deliberately ignores timestamps.
- The AV1/Opus and VP9/Opus WebM originals and copies emit an Opus-header
  diagnostic even though FFmpeg exits successfully and their full decoded
  outputs match. These four files are not claimed to have diagnostic-free
  FFmpeg runs. No native zenavif video encode/decode qualification is implied.

`media_transport` then drives all 27 MP4 files, ten WebM/Matroska files and
two IVF extractions through slice input, short async random reads and a
forward async body. It compares complete track/packet/event representations,
including payload bytes, timing and side data. The async sources suspend
between reads. Chunk sizes are 1, 7, 1024 and 65536 bytes.

| Container | Random source, equal to slice | Forward source, equal to slice | Forward refusals |
|---|---:|---:|---:|
| MP4 | 108 / 108 | 3 / 108 | 105 / 108 |
| WebM/Matroska | 40 / 40 | 40 / 40 | 0 / 40 |
| IVF | 8 / 8 | 8 / 8 | 0 / 8 |

The [per-file CSV](io_transport_2026-10-24.csv) preserves every cell's outcome
and the 65536-byte random-source transfer counts. Refusals are observations,
not desirable behavior asserted by the test. No real HTTP server, request
latency or network-throughput measurement is involved.

### Two independent obstacles to forward MP4

1. **Discarded prefetch, [zencodec #163](https://github.com/imazen/zencodec/issues/163).**
   The 26435-byte faststart AV1/Opus file works at chunks 1, 7 and 1024,
   but the default 65536-byte chunk fails with `BeyondWindow` after the
   driver replaces prefetched bytes when switching from `AtLeast` to `Range`.
   Its random-source equivalent performs 122 reads and transfers 50666 bytes.
   The private active-window reuse prototype makes the forward case pass at
   all four sizes; the large-chunk random case uses one read and transfers
   exactly 26435 bytes, with all 123 events unchanged. The prototype only
   handles wholly contained ranges. Partial overlap, suffix retention,
   synchronous drivers and adversarial bounds still need a production fix.
2. **Demuxer backtracking, [zenavif #72](https://github.com/imazen/zenavif/issues/72).**
   Fragmented MP4 still refuses forward input with the window prototype,
   including one-byte reads. After a fragment's packets, the demuxer returns
   to the earlier `mdat` header because its scan position remains at the end
   of `moof`. This is independent of read-ahead. Its current `RandomAccess`
   declaration is honest; enabling supported monotone layouts requires a
   demuxer change. Tail-indexed progressive MP4 still intrinsically needs
   ranges or an explicit spool/replay policy.

### Relevant review queue, checked 2026-10-24

- [zencodec PR152](https://github.com/imazen/zencodec/pull/152),
  [zenavif PR69](https://github.com/imazen/zenavif/pull/69),
  [PR70](https://github.com/imazen/zenavif/pull/70),
  [PR71](https://github.com/imazen/zenavif/pull/71), and
  [zenextras PR50](https://github.com/imazen/zenextras/pull/50) are merged:
  media sessions, native AV1 sessions/cancellation and these containers are
  current code, not merely pending proposals.
- [zenrav1e PR44](https://github.com/imazen/zenrav1e/pull/44) remains open at
  `9057c4bacf1d8a23974e32de09fd4446a1cc525a` (small frames and lossless inter
  samples). The inspected zenavif `media-encode` dependency already pins
  that exact head. An open PR can therefore be in the active dependency
  graph. Its native sequence corpus and performance claims were **not
  rerun** by this I/O investigation.
- [zenpipe PR86](https://github.com/imazen/zenpipe/pull/86) remains a draft at
  `330eea4bee0b17567c80730edbea3feef742f860`, stacked on PR85, for media pins
  and explicit SDR conversion. This experiment does not qualify that
  pipeline's color conversion or timing contracts.
- [zenjxl-decoder PR60](https://github.com/imazen/zenjxl-decoder/pull/60) and
  [zenjxl PR20](https://github.com/imazen/zenjxl/pull/20) concern movable native
  decoder state and frame-at-a-time animation; that is distinct from feeding
  network bytes incrementally. [WebP PR105](https://github.com/imazen/zenwebp/pull/105)
  has newer cancellation work than its recorded benchmark head. None of
  those PRs is performance-qualified by the QOI or container results here.

```text
MP4 oracle: rc=0 14s | peak-RSS 0.25GiB | min-avail 15879MiB | peak-load 0.63
WebM oracle: rc=0 5s | peak-RSS 0.20GiB | min-avail 15881MiB | peak-load 0.38
strict decode oracle: rc=0 2s | peak-RSS 0.07GiB | min-avail 16167MiB | peak-load 0.22
full async container matrix: rc=0 2s | peak-RSS 0.21GiB | min-avail 16026MiB | peak-load 0.45
final harness/container tests: rc=0 1s | peak-RSS 0.19GiB | min-avail 15806MiB | peak-load 12.72
final harness clippy: rc=0 0s | peak-RSS 0.16GiB | min-avail 15611MiB | peak-load 12.72
```

The final correctness/Clippy runs occurred under higher host load; no timing
comparison is drawn from them. These are wrapper RSS measurements, not
isolated codec working-set bounds.

## Scope justified by these experiments

Implement the range-window repair, checked input/output budgets and bounded
transcode scheduling in drivers; add a positional file adapter under the
existing `ReadAt` contract. Select batching policy using actual codecs and
natural strip sizes. A row count is only a diagnostic threshold here; video
needs packet/frame-aware work limits and byte limits across all tracks.

Retain the media API's existing `MuxTiming`, `MuxHint::forward_only` and
`MuxFinish::patches`. A socket requires forward emission; a file may apply
absolute patches; replayable input pixels are a separate requirement for a
multi-pass encoder. Patchability cannot eliminate reference frames, reorder
queues or whole-frame processing inside a video codec.

No experiment here calls for a new universal seek abstraction or a core
[nostdio](https://docs.rs/nostdio/0.2.0/nostdio/) dependency. Its synchronous
Read/Write/Seek compatibility can be adapted when a concrete parser needs
it. The measured blockers are already expressible by the existing sans-I/O,
positional-read and media-output contracts.

Before selecting production defaults, the remaining qualification gates are:

1. Real HTTP range and streaming-body fixtures: unknown length, ignored or
   malformed ranges, changing object identity, cancellation/timeouts,
   out-of-order responses, bounded in-flight requests and overfetch. Measure
   first usable output, throughput, request count and bytes at controlled RTTs.
2. Cold and warm local files, concurrent readers and multiple platforms.
   Measure allocations/copies and queue occupancy, and use heap profiling or
   process peak RSS for actual working-memory claims. Do not derive a codec
   memory bound from the logical queues measured here.
3. Native JPEG/PNG/WebP/JXL and AV1 session/PR heads: natural strip/frame
   geometry, strided pixels, high expansion, 8/10/12-bit content, B-frame
   reordering, drain/reset/seek and all-track backpressure. Compare pixels,
   metadata and timestamps with independent decoders. Keep native streaming
   distinct from whole-file compatibility adapters.
4. Fault injection at budget and arithmetic boundaries, short/zero writes,
   interrupted offset reads, cancellation during blocking work and partial
   finalization. Admit patch/replay/spool needs before emitting output.

Reproduction commands and prototype limitations are in the
[harness README](../perf/io-validation/README.md). Follow-up implementation
is tracked in [#159](https://github.com/imazen/zencodec/issues/159),
[#160](https://github.com/imazen/zencodec/issues/160),
[#161](https://github.com/imazen/zencodec/issues/161),
[#162](https://github.com/imazen/zencodec/issues/162),
[#163](https://github.com/imazen/zencodec/issues/163) and
[zenavif #72](https://github.com/imazen/zenavif/issues/72).
