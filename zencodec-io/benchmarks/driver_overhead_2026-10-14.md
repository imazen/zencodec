# Driver overhead, 2026-10-14

Host: `mac` (Apple M4 Pro, macOS, rustc stable 1.99). Tree: PR 147 branch
`feat/sansio-feed-step` at `27ed39ec` plus the follow-up change described
in this commit. zenbench run was `--cpu-policy=unrestricted`
(UNVERIFIED_UNRESTRICTED on the asymmetric M4; a verified run on `dev`
is recorded separately when available). Fixture:
`zencodec-io/benches/fixture.rs`, a memcpy-per-strip RGBA8 decode, so
every number is driver and coroutine overhead, not codec work.

Commands:

```
cargo run --release -p zencodec-io --features tokio --example alloc_audit
cargo bench -p zencodec-io --features tokio --bench drivers -- --cpu-policy=unrestricted
```

## Allocations per decode (counting global allocator, all threads)

1024x1024 RGBA8 (4 MiB), 64 strips of 16 rows.

| driver shape | allocs | per strip | bytes allocated |
|---|---|---|---|
| baseline: direct sink writes, no driver | 1 | 0.016 | 4,194,304 |
| hand-written feed/step via `drive_from_slice` (1 MiB chunks) | 13 | 0.203 | 8,389,048 |
| hand-written via `pump_read_to_sink` (64 KiB) | 4 | 0.062 | 4,456,448 |
| coroutine, sequential, via `drive_from_slice` | 23 | 0.359 | 8,521,680 |
| coroutine, ranges, via `drive_from_read_at` | 84 | 1.312 | 4,459,920 |
| hand-written via tokio `decode_on_thread` | 41 | 0.641 | 11,538,966 |
| same, strips streamed back via `ChannelRowSink` | 106 | 1.656 | 11,544,358 |

Before the fixes in this change the sequential coroutine was 159 allocs
and 45,550,528 bytes (the input store copied the rest of a segment on
every front release) and the range coroutine 276 allocs.

## Time per decode (zenbench, mean, 30 rounds)

| group | direct | hand-written / drive | hand-written / pump | coroutine seq | coroutine ranges | tokio bridge |
|---|---|---|---|---|---|---|
| 1024x1024, 16-row strips | 73.4 µs | 266.6 µs | 319.9 µs | 439.1 µs | 344.1 µs | 303.7 µs |
| 1024x1024, 1-row strips | 112.2 µs | 310.2 µs | 318.3 µs | 481.1 µs | 449.6 µs | 347.9 µs |
| 256x256, 4-row strips | 7.6 µs | 19.8 µs | 19.8 µs | 31.2 µs | 30.0 µs | 50.4 µs |

Reading: the cost is memcpy count, not machinery. Direct is one copy
(into the sink); hand-written through a driver is three (fetch buffer,
decoder buffer, sink); the coroutine is four (store, strip, sink). The
per-step machinery is the difference between the 1-row and 16-row rows
divided by the 960 extra steps: about 45 ns per step for the hand-written
decoder and about 40 ns for the coroutine. The tokio bridge adds one
thread spawn and two channel hops per request; on the small image that
fixed cost dominates (50 µs for a 256 KiB decode).

Before this change the hand-written template (bench fixture and testkit
reference) released input with `Vec::drain`, shifting the whole remaining
buffer each strip: 1,044 µs at 16-row strips and 7,947 µs at 1-row strips
for the same image, versus 266 µs and 310 µs after switching to a head
index with amortised compaction. The first bench run caught it.

## dev (AMD Ryzen 9 9950X3D, Linux), same tree at `1053613c`

Throwaway clone at `~/tmp/zencodec-bench` on dev, built with
`run-heavy` (peak RSS 0.61 GiB). zenbench refused verified placement on
the 9950X3D's two L3 classes; this run is `--cpu-policy=unrestricted`
(UNVERIFIED_UNRESTRICTED, exploratory).

Allocation audit: identical counts to the mac run except the tokio
bridge (32 allocs, 0.50 per strip; 98 and 1.53 with `ChannelRowSink`),
the difference being tokio's thread-local setup.

heaptrack over the whole audit binary (all seven decodes):
298 calls to allocation functions, 117 temporary, peak heap 15.81 MB,
peak RSS including heaptrack 23.09 MB, 948 B reported leaked (runtime
statics).

| group | direct | hand-written / drive | hand-written / pump | coroutine seq | coroutine ranges | tokio bridge |
|---|---|---|---|---|---|---|
| 1024x1024, 16-row strips | 93.1 µs | 289.0 µs | 203.6 µs | 381.2 µs | 264.1 µs | 597.9 µs |
| 1024x1024, 1-row strips | 106.7 µs | 288.7 µs | 212.9 µs | 403.2 µs | 337.6 µs | 650.9 µs |
| 256x256, 4-row strips | 4.7 µs | 12.2 µs | 10.1 µs | 21.9 µs | 20.3 µs | 61.7 µs |

Reading: the stream pump beats the 1 MiB-chunk drive loop here because
its 64 KiB chunks stay in L2 while a 1 MiB fetch buffer does not
(`DriveOptions::sequential_chunk` is the knob). The tokio bridge's fixed
cost on Linux is about 50 to 60 µs per decode (thread spawn, two futex
hand-offs per fetch round, and the fetcher's chunk copy), which is why
`ThreadOptions::with_spawner` exists for a pool and why small images
should not cross a thread at all. Per-step machinery stays in the tens of
nanoseconds: 1-row versus 16-row differs by about 0 to 60 µs over 960
extra steps for every in-thread shape.
