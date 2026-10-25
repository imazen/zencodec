# Encoder output stalls and retained raw input

Library baseline: `1d7e54bbcdeb45ce961fb68df018bcbb6100052c`.
Native codec: zenbitmaps `11fbd887985e59769d2aeb48295767b7d04209b9` from the
existing pinned experiment export. Rust 1.99.1, r7900x.

`just io-encoder-retention-check` tests the existing async drivers with native
QOI and a deliberately retaining wrapper. The wrapper can retain copies of
every decoded row until `finish`, while delegating encoding and output timing
unchanged to QOI. It represents a raw ownership lifetime; it is not a claim
about native QOI's working memory.

The 512x512 opaque RGBA fixture is encoded before the observed drive and the
original raw allocation is dropped. QOI decode selects RGB rows, so retaining
every decoded row holds 786432 logical raw bytes. A gated writer first returns
Pending, then accepts at most 97 bytes per write once released. No wall-clock
sleep or timing model determines the observations.

| Raw retention control | Driver schedule | Raw bytes at blocked write | Encoder finalized there |
|---|---|---:|---|
| Retain every row | Current `transcode_async` | 786432 | false |
| Retain every row | `decode_async` into `EncoderSink`, finish, then write | 0 | true |
| Retain no rows | Current `transcode_async` | 0 | false |

All three report streaming output and preserve all 4255 encoded bytes exactly.
The current-driver observation is recorded without requiring that retention
to remain forever. The finish-first assertion requires release before waiting.

The control stages complete encoded output in memory. It does not implement
a bounded generic spool, admit arbitrary jobs, measure a heap/RSS reduction,
or calibrate a production threshold. Raw lengths are logical ownership counts.
Finite jobs that retain raw data need separately admitted encoded storage;
sequential operations need output backpressure that stops upstream production.
CPU fairness must not imply waiting on a slow client in every configuration.

[Full output](io_encoder_retention_2026-10-25.log). Tests and standalone
all-target clippy pass:

```text
run-heavy: done rc=0 0s | peak-RSS 0.19GiB | min-avail 16150MiB | peak-load 0.15
```

This refines the [IO proposal](../docs/io-contract-proposal.md#5-bound-compute-batches-and-preserve-media-order)
and [issue 160](https://github.com/imazen/zencodec/issues/160). The next simulator
experiment should measure raw/intermediate allocation lifetimes, aggregate
concurrent-job occupancy, encoded staging and client delivery separately.
