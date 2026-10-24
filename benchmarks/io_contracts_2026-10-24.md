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
