# Driver overhead on the sans-I/O boundary v2, 2026-10-14

Host: `mac` (Apple M4 Pro, macOS, rustc stable). Tree: PR 147 branch
`feat/sansio-feed-step`, parent `e54b4175` plus the change that adds this
file (read sizing in whole requests, no per-batch allocation for inline
compute). zenbench `--cpu-policy=unrestricted` (UNVERIFIED_UNRESTRICTED on
the asymmetric M4). Fixture: `zencodec-io/benches/fixture.rs`, a
memcpy-per-strip RGBA8 decode, so every number is framework overhead, not
codec work. Allocation counts are zenbench's per-iteration profiler on the
bench thread; the spawn_blocking arm's pool threads are not counted.

```
cargo bench -p zencodec-io --features tokio --bench drivers -- --cpu-policy=unrestricted --format=llm
```

| group | arm | mean | vs direct writes (95% CI) | allocs/decode |
|---|---|---|---|---|
| 1024x1024/16-row strips | direct sink writes | 67.23µs | baseline | 1.0 |
| 1024x1024/16-row strips | hand-written via drive_from_slice | 68.37µs | +0.28% to +2.12% | 1.0 |
| 1024x1024/16-row strips | hand-written via pump_read_to_sink 64K | 141.47µs | +108.27% to +113.86% | 2.0 |
| 1024x1024/16-row strips | coroutine sequential via drive_from_slice | 320.66µs | +372.49% to +378.11% | 12.0 |
| 1024x1024/16-row strips | coroutine ranges via drive_from_read_at | 243.25µs | +259.13% to +263.40% | 12.0 |
| 1024x1024/16-row strips | hand-written via tokio decode_async, inline | 156.96µs | +130.19% to +134.59% | 3.0 |
| 1024x1024/16-row strips | hand-written via tokio decode_async, spawn_blocking | 573.48µs | +753.13% to +760.69% | 200.2 |
| 1024x1024/1-row strips | direct sink writes | 102.50µs | baseline | 1.0 |
| 1024x1024/1-row strips | hand-written via drive_from_slice | 106.95µs | +3.88% to +5.27% | 1.0 |
| 1024x1024/1-row strips | hand-written via pump_read_to_sink 64K | 203.73µs | +96.94% to +100.69% | 2.0 |
| 1024x1024/1-row strips | coroutine sequential via drive_from_slice | 377.42µs | +265.66% to +270.78% | 12.0 |
| 1024x1024/1-row strips | coroutine ranges via drive_from_read_at | 339.53µs | +228.47% to +232.81% | 12.0 |
| 1024x1024/1-row strips | hand-written via tokio decode_async, inline | 218.01µs | +111.05% to +114.35% | 3.0 |
| 1024x1024/1-row strips | hand-written via tokio decode_async, spawn_blocking | 594.92µs | +475.84% to +485.12% | 200.2 |
| 256x256/4-row strips | direct sink writes | 7.57µs | baseline | 1.0 |
| 256x256/4-row strips | hand-written via drive_from_slice | 7.91µs | +1.52% to +5.59% | 1.0 |
| 256x256/4-row strips | hand-written via pump_read_to_sink 64K | 14.51µs | +88.57% to +94.58% | 2.0 |
| 256x256/4-row strips | coroutine sequential via drive_from_slice | 26.47µs | +244.05% to +255.02% | 12.0 |
| 256x256/4-row strips | coroutine ranges via drive_from_read_at | 24.46µs | +217.12% to +228.72% | 12.0 |
| 256x256/4-row strips | hand-written via tokio decode_async, inline | 15.18µs | +96.77% to +102.51% | 3.0 |
| 256x256/4-row strips | hand-written via tokio decode_async, spawn_blocking | 48.22µs | +532.06% to +540.47% | 20.1 |

## Reading the table

- The slice driver offers the decoder sub-slices of the caller's bytes:
  +0.3 to +5.6% over writing the sink directly, and no allocation of its
  own.
- The stream pump's cost over the slice driver is the copy out of
  `std::io::Read` into its window, one more pass over the input. Before
  this change, a 16-byte header in front of 64 KiB strips misaligned every
  64 KiB read and the leftover was moved on every strip: +257.7 to +270.3%
  on the 16-row group. Reads are now sized to a whole number of the
  decoder's `min_len`, so the window drains to empty and restarts.
- `decode_async` with `Inline` copies from the `AsyncSource` into the
  window like the pump. With a thread pool, each batch is one handoff and
  costs three allocations (the boxed job, the result channel, the pool's
  task); a batch here is one 64 KiB strip, so a real codec's work per batch
  dwarfs the handoff. `DriveOptions::read_size` sets the batch size.
- The coroutine arms copy input into the coroutine's store and each strip
  once more on its way to the sink. That is the price of writing a decoder
  as straight-line `async` code in safe Rust; it suits container walks and
  headers, not bulk pixel data.
