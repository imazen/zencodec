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
