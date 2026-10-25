# IO implementation qualification, 2026-10-24

This continues [the transport investigation](io_validation_2026-10-24.md).
The baseline is `9ca5fd1fded1b8d7edd29f616e9f2361d319ed2a`;
codec/container pins, fixtures and dependency locks are unchanged.
Tests run in an export outside workspace path patches, using Rust 1.99.1
on r7900x. No `target-cpu=native`. Builds run serially through the resource
wrapper with 12 GiB / four jobs. Wrapper RSS includes compiler processes;
logical buffer counters are not heap measurements.

## One-shot writer fallback

Implemented in `648dee88a71756b0e8e03bd89f9642f78e726236`.
Two regressions failed before their fixes:

- Inherited `Encoder::begin` reported success although inherited `push_rows`
  rejects rows. It now refuses at begin, preserving the encoder for one-shot use.
- `encode_to_writer` checked only typed causes. An envelope whose category is
  `Request(Unsupported(RowLevelEncode))` also selects the fallback now. A different
  category remains an error. No fallback occurs after output or rows are accepted.

The strided fixture spans multiple driver strips. The actual pinned BMP encoder
writes exactly the same 70 bytes as direct encoding (previously zero bytes and
an error). Full workspace all-feature tests and all-target/all-feature clippy
pass. Existing ignored documentation examples were not changed.

```
run-heavy: done rc=0 7s | peak-RSS 0.30GiB | min-avail 15853MiB | peak-load 1.23
```

## Reusing held ranges

The implementation covers synchronous `ReadAtCursor` and the async fill shared
by image and media drivers. Fully held requests keep the remaining suffix.
Partially held requests read only their missing tail. Batched range allocations
can become the sequential window without copying. Sequential cache hits return
without filling a speculative prefetch target. This does not add a general
range cache or manufacture random access for forward streams.

Forward-only regression scripts fail on the baseline at offset 4 after a
16-byte prefetch, and at offset 16 after an earlier batch already fetched
16..32. They pass after the change. The overlap script reads `(0,16)`, `(16,4)`,
`(20,18)`; 18 reflects the existing rounding to three-byte requests. The batch
script reads `(0,4)`, `(16,16)`, `(32,4)`. Async cases include Pending and chunks
1, 3, 7, 16 and 64. A genuine backwards request still returns BeyondWindow.

[All per-case results](io_transport_reuse_2026-10-24.csv) record the real-container
rerun. All **156/156 random-source cases** preserve complete event equality with
slice input, including tracks, packet bytes, timing and side data. Forward
WebM/Matroska/IVF remains **48/48**. Forward MP4 is **4/108**, all four cases being
the faststart AV1/Opus fixture. At 64 KiB it uses one 26435-byte read for all 123
events, versus baseline 122 reads / 50666 bytes. Other layouts still need random
access or demuxer work; fragmented AV1/Opus still requests offset 1534 after
reading to 7694 with one-byte chunks. See
[zenavif issue 72](https://github.com/imazen/zenavif/issues/72).

Driver regressions, native QOI strided/truncation parity, and workspace all-target
all-feature clippy pass. These are in-process transport tests, not HTTP speed
measurements. The existing independent FFmpeg fixture oracle is described in
the baseline report; no new encoded fixture was generated in this rerun.

```
run-heavy: done rc=0 4s | peak-RSS 0.21GiB | min-avail 15860MiB | peak-load 0.57
```

Reproduce with the baseline report's export and lock setup, the current source,
`just io-contract-check`, `just io-validation-test`, and
`just io-validation-media <mp4-dir> <webm-dir> <ivf-dir>`.

## Positional files

`FileReadAt` now uses safe standard-library positional operations on Unix and
Windows, with the old seek/read fallback on other platforms. Length comes from
metadata instead of a seek to end. Windows `seek_read` is allowed to update the
shared file position; Unix `read_exact_at` does not. Short reads and Interrupted
are handled by the standard Unix helper / explicit Windows loop.

Tests cover reversed reads, short EOF, overflow, truncation after opening, and
(on Unix) a sparse read above 4 GiB and two concurrent sources sharing a file
handle. Every returned byte is compared with authored values. The Unix test
also verifies construction and reads preserve an independent shared cursor.
The benchmark retains a frozen seek/read control from the baseline so its label
cannot accidentally start measuring the improved implementation.

Warm page-cache results, 128 reads per sample, 64 MiB authored file, median of
three zenbench passes on r7900x (microseconds):

| Pattern | Bytes/read | Frozen seek/read | Current FileReadAt |
|---|---:|---:|---:|
| Sequential | 256 | 101.80 | 64.53 |
| Dispersed | 256 | 102.75 | 65.28 |
| Sequential | 4096 | 108.93 | 71.45 |
| Dispersed | 4096 | 109.44 | 71.65 |
| Sequential | 65536 | 284.04 | 246.13 |
| Dispersed | 65536 | 326.18 | 287.54 |

Syscall probe: 12928 verified data reads with current FileReadAt produce 12930
pread64, five read and two lseek calls including process startup. The baseline
had 12931 lseek calls for the same workload. Disk cache misses and Windows/macOS
performance are not measured by this table. Zenbench marks the CPU placement
unverified under the explicitly unrestricted policy; these are exploratory
within-process comparisons on a shared host.

```
file tests/build/syscalls: rc=0 7s | peak-RSS 0.30GiB | min-avail 15727MiB | peak-load 0.34
warm file comparison: rc=0 42s | peak-RSS 0.23GiB | min-avail 15682MiB | peak-load 1.15
```

Commands: `just io-validation-disk <file> 2,3`, then
`strace -c -e read,pread64,lseek target/release/disk_probe file 100 <file>`.

## Native AV1 sessions and pending PR support

The current zenavif main is still `a7638f6e`, with media encoding pinned to the
still-open [zenrav1e PR 44](https://github.com/imazen/zenrav1e/pull/44) head
`9057c4bacf1d8a23974e32de09fd4446a1cc525a`. With export-only patches to current
zencodec `c3a5ddd1`, `cargo test --no-default-features --features media-encode
--test media_session` passes 14/14 tests, with no skips. [Test output](io_av1_sessions_2026-10-24.log).

The sample-exact matrix is seven frames in each of these six geometries:
8-bit 420 33x17, 8-bit mono 16x16, 10-bit 422 40x9, 10-bit 444 7x13,
12-bit 420 24x24, 12-bit 444 18x30. Other tests exercise queues 1/3/64,
the default-speed short queue, owned backend cancellation, av1C/sequence-header
agreement, reset with previously returned frames still valid, and a 30-frame
10-bit 420 lossy sequence whose output is in presentation order. No SVT feature
or full media corpus was enabled. These are native session correctness results,
not AV1 speed, HDR conversion or integrated network playback measurements.
The zenpipe media draft remains open at `330eea4b`; its full pipeline was not run.

```
run-heavy: done rc=0 104s | peak-RSS 0.87GiB | min-avail 14475MiB | peak-load 2.91
```

## Executable admission and ownership candidate

`perf/io-validation/src/admission.rs` is private experiment code, with tests in
`tests/admission.rs`. It does not add a zencodec library type. It implements:

- Checked all-dependency plans: overflow/extent/descriptor admission; union
  overlaps; merge gaps only within the byte allowance; split large spans before
  allocating payload storage. This is explicitly not the existing any-request
  `requested()` contract.
- Shared payload-byte and allocation-count reservations. A transfer allocates
  after reservation; a published lease retains the allocation charge through
  cache/decoder clones until its final owner drops. Partial, oversized and
  wrong-identity responses do not publish.
- A bounded ordered completion queue, distinct from transport completion order;
  stale seek generations and duplicate ordinals are refused. To prevent a full
  queue behind a missing head, callers must admit the head before later work.

Tests cover arithmetic/split limits, concurrent admission, cancellation release,
clone ownership, stale results and actual strided QOI decoded from fragmented
leased input through the current driver. Limits account payload capacity and
object count, not total RSS, codec heap, network-library buffers or arbitrary
caller allocations. HTTP validation must establish the opaque identity token;
the token itself is not an HTTP implementation. A scheduler, waiter fairness,
cache eviction and fallible metadata-node allocation are not implemented here.
The ordered queue establishes decode ordinal order; presentation reordering
remains the video session's job.

[Raw zenbench output](io_admission_2026-10-24.log), median of three passes:

| Input bytes | Vec allocation/copy + Arc | Reservation/transfer/copy + lease |
|---:|---:|---:|
| 1024 | 17.8 ns | 53.4 ns |
| 65536 | 805 ns | 1220 ns |

The 1024-byte lease cell has 25% CV; do not use it as a stable overhead ratio.
Both arms allocate/copy and clone/drop one shared owner; the candidate also zeros
transfer storage and checks identity/completion. This measures that whole path,
not mutex cost alone. It does not measure a contended HTTP scheduler. Preserve
borrowed resident-slice paths; ownership is for bytes that must outlive an I/O
call or cross a task boundary. The correctness tests and standalone all-target
clippy pass.

```
run-heavy: done rc=0 18s | peak-RSS 0.27GiB | min-avail 15778MiB | peak-load 0.85
```

Reproduce with `just io-admission-check` and `just io-admission-bench 2,3`.

## File-cache misses

The additional Linux benchmark uses `rustix` 1.1.5 `fadvise(DontNeed)` on the
individual 64 MiB fixture before every timed sample. It does not clear global
caches. Setup and teardown are outside timing; teardown asserts that the
process's kernel `read_bytes` increased by at least the requested payload.
This is evidence of block-read accounting, not proof of a cold device cache.
Both arms verify authored fixture bytes before measuring.

[Raw output](io_disk_cold_2026-10-24.log), median of three passes with nine
single-iteration rounds per pass, 128 reads per sample, milliseconds:

| Pattern | Bytes/read | Frozen seek/read | Current FileReadAt |
|---|---:|---:|---:|
| Sequential | 4096 | 1.03 | 0.94646 |
| Dispersed | 4096 | 9.04 | 8.65 |
| Sequential | 65536 | 2.16 | 2.36 |
| Dispersed | 65536 | 18.08 | 18.23 |

The positional 4096-byte sequential cell has 42% CV. CPU placement has the same
unrestricted/unverified qualification as the warm run. These measurements do
not establish a universal cache-miss speedup. The warm syscall reduction remains
useful, but device waiting and read-ahead policy matter much more here. Network
coalescing thresholds should not become unconditional disk read-ahead defaults.

```
cache-miss benchmark: rc=0 29s | peak-RSS 0.22GiB | min-avail 15683MiB | peak-load 0.79
standalone all-target clippy: rc=0 1s | peak-RSS 0.16GiB | min-avail 16096MiB | peak-load 0.31
```

Reproduce with `just io-validation-disk-cold <file> 2,3` under the resource
wrapper. The log names the underlying zenbench attempt and run artifacts.

## Allocation and instruction profiles

The [fixed-work profile summary](io_disk_profiles_2026-10-24.log) records
`perf stat -r 3` and heaptrack runs, using the existing verified disk probe and
the same 4096-byte reusable output. Both adapters make **20 allocation calls**
for both 256 and 1280128 reads. All four heaptrack reports show **79.67K peak
heap consumption** (heaptrack's units, whole process). Thus this workload does
not allocate per read in either implementation. It says nothing about a codec's
heap or a network client's buffers. The 548-byte exit residue includes runtime
state and is not attributed to the adapter.

For 1280128 reads, whole-process mean counters over three runs:

| Adapter | Instructions | Cycles | Cache misses |
|---|---:|---:|---:|
| Frozen seek/read | 3372209446 | 5712069839 | 4778149 |
| Current FileReadAt | 2769217018 | 3843638661 | 4337071 |

These profiles explain the warm-path improvement without assigning every cycle
to a source line. Perf/heaptrack elapsed times are not replacement throughput
benchmarks. The trace binaries and input hashes are retained with the raw data.

```
run-heavy: done rc=0 8s | peak-RSS 0.09GiB | min-avail 15922MiB | peak-load 0.23
```

Reproduce with `just io-validation-disk-profile <new-output> <file> 2,3`.
