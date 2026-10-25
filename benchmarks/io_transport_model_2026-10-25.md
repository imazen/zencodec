# Read API choices through the reused IMI transport model

Model source: imazen/zqi `b3856304ed60ee232b792a18460aadaf3886a63e`,
`imij-set/examples/access_model/transport.rs`. The format-independent kernel is
copied into `perf/io-validation/src/transport_model.rs`; only trait/error
adaptation changes it. No private format grammar or input data is included.

Run: `just io-transport-model` after `just io-validation-setup`, in an export
outside workspace path overrides. zencodec base: `b82a9c72`. Pins and lockfiles
are in `perf/io-validation/`. Rust 1.99.1; host r7900x; 2026-10-25 UTC.
[All 32 results](io_transport_model_2026-10-25.csv) are calculated network costs,
not measured WAN latency, decoder throughput or process memory. Two policy
regressions and all-target clippy pass. Every returned byte matches the source.

Input: an authored 131072-byte immutable object; 16 independent 4096-byte
requests spaced 8192 bytes apart; then the identical viewport again. Aggregate
bandwidth is 12500000 bytes/second, shared across requests. Each admission wave
pays one RTT. A batch exposes independent requests together. Cache is 0 or
131072 logical encoded bytes; request concurrency is 1 or 8; RTT is 0 or 160 ms.

At 160 ms RTT, concurrency 8 and no cache:

| Policy | Simulated ms | Transferred bytes | Dependency rounds |
|---|---:|---:|---:|
| serial read_exact_at | 5130.48576 | 131072 | 32 |
| batched read_many_at | 650.48576 | 131072 | 2 |
| merge 4096-byte gaps | 340.31616 | 253952 | 2 |
| checked 65536-byte plan | 650.48576 | 131072 | 2 |

With zero RTT and no cache, the gap-merging policy takes 20.31616 simulated ms;
the other policies take 10.48576. With a full cache, the second viewport incurs
no transfer. The checked planner refuses to spend more than its 65536-byte
per-view budget on gaps, so it retains the original 16 ranges. That is a
concrete cost of the limit, not a reason to bypass it.

This supports keeping `read_many_at` at the adapter boundary and making
coalescing a transport policy with an explicit byte budget. It does not justify
a new core seek trait: positional reads already describe the useful operation.
`IncrementalDecode::requested()` still means alternatives; an all-required
batch must come from an explicit dependency plan, never by reinterpreting it.

The reused model does not charge DNS/TLS/HEAD, retries, CPU, cancellation,
concurrent views or output scheduling. It tracks cached extents, not allocated
buffers. The real HTTP benchmark retains a source raster in server state, so it
cannot alone establish raw-storage release under client backpressure. See the
[ownership control](io_encoder_retention_2026-10-25.md) for that separate test.

```
run-heavy: done rc=0 0s | peak-RSS 0.17GiB | min-avail 16055MiB | peak-load 1.10
```
