# zencodec-io

Drivers that run a [zencodec](https://github.com/imazen/zencodec) decoder over
real input: a slice, a random-access source, a `std::io::Read` stream, or an
async runtime. Codecs do no I/O; this crate owns the loops.

| input | resumable decoder (`IncrementalDecode`) | pull-native decoder (`push_decoder_from_source`) |
|---|---|---|
| slice | `drive_from_slice` | `SliceReadAt` |
| `ReadAt` (file, object store, FFI callback) | `drive_from_read_at` | direct |
| `std::io::Read` stream | `pump_read_to_sink` | `BufferedReadAt` |
| async runtime (feature `tokio`) | `tokio_io::decode_async`, `transcode_async` | `tokio_io::pull_decode_on_thread` |

The drivers own the input bytes and offer them to the decoder in place; the
slice driver copies nothing. Every driver reports a decoder that spins or asks
past a known end (`ProtocolViolation`) instead of looping, and takes
`Option<&dyn how_far::Pulse>` for cancellation and progress (`Pulse: Stop`; a
plain stop passes as `how_far::StopOnly::borrowed`).

Use `PolicyDecoder` for automatic resolution of mandatory interpretation
boundaries. Adapters that need to prepare conversion and encoding while decode
is paused use `ControlledStepper` with the existing byte cursors. A
`ControlledEvent::Suspended` keeps the native session held; inspect its facts,
configure the destination through `sink_mut`, then resume that same boundary.
See the [controlled protocol](../docs/controlled-decoding.md).

`FileReadAt` and `BufferedReadAt` return `StdSourceError`: structural bounds and
window failures retain their `SourceError`, and native I/O failures retain the
original `std::io::Error`, including its kind, OS code and custom payload. Typed
causes survive the normal erased source and driver paths. Retry still requires
checking whether input was consumed and whether output has been committed.

For an existing synchronous `Read + Seek` parser, use
`ReadAtReader::new(FileReadAt::open(path)?)` or supply another `ReadAt` source.
The bridge has its own cursor and adds no buffering. End-relative seeks can
consume a sequential source to find its length. A `BufReader` is optional:
[native TIFF measurements](../perf/spatial-validation/README.md) show fewer
source calls but more transferred bytes, so its capacity belongs to the
transport plan. This bridge does not turn a synchronous parser into a resumable
decoder or combine individual reads into concurrent range batches.

Use `BufferPolicy::try_for_pattern(pattern, max_bytes)` to reject insufficient
lookback before reading. Windows retain at most their configured history even
when a forward read is larger. Spools reject impossible exact ranges before I/O;
finding EOF at the exact cap uses one stack byte of lookahead. An excess byte
makes the spool terminal. These are retained-payload limits, not whole-operation
memory or total-input-byte limits; `retained_bytes()` and `watermark()` expose
those distinct counts.

`DriveOptions::with_limits(IoLimits::new(input_bytes, ranges, in_flight))`
applies input admission to `ReadAtCursor`, `AsyncCursor` and the incremental
image drivers. It bounds retained input capacity, planned ranges and ranges
passed to a source per batch. Requests are alternatives: the driver chooses an
affordable one and reduces optional prefetch/gap merging to fit. Impossible
contiguous requests return `InputError` before reading. Resident borrowed
slices need no input allocation allowance. Defaults preserve existing behavior.

This allowance excludes source caches, codec/output allocations, allocator
bookkeeping and transient reallocation. `retained_input_capacity()` exposes
the cursor's charged capacities. `ReadCursor` applies its existing `max_window`
to prefetch and skipping as well as the decoder's minimum request.

The async drivers keep the decoder in the task between batches and run each
batch on a `Compute` (`spawn_blocking`, a rayon pool, or `Inline`), so no
thread is held while bytes are in flight and codecs stay synchronous.
`DriveOptions::with_work_quota(WorkQuota::new(64).unwrap())` limits each async
compute turn to 64 complete codec steps. It also yields between ready packet
outputs in `AsyncCursor`. A quota yield does not drain encoder output; it gives
other tasks a turn without tying CPU fairness to a slow destination. One codec
step remains indivisible, so this is neither a wall-time nor a memory bound.
When both codec paths stream, `transcode_async` streams rows into an encoder
and its output to an `AsyncWrite`. A whole-file decoder or an `AT_FINISH`
encoder can retain a complete input or output. Driver measurements:
[benchmarks](https://github.com/imazen/zencodec/tree/main/zencodec-io/benchmarks).

Output backpressure is configuration-specific. Pausing a truly sequential
pipeline can bound both upstream production and encoded queues. Pausing an
encoder that retains a complete raster can instead keep that raster alive
for a slow client. Admit buffered jobs and their encoded staging separately;
`OutputTiming` describes byte availability, not raw-storage release. See the
[ownership experiment](https://github.com/imazen/zencodec/blob/main/benchmarks/io_encoder_retention_2026-10-25.md).

`no_std + alloc` by default. `std` adds the stream pump, `FileReadAt` and
`BufferedReadAt`; `tokio` adds `tokio_io` (it uses `tokio::sync` and
`tokio::io` only and never starts a runtime). Design:
[range-decoding contract](https://github.com/imazen/zencodec/blob/main/docs/range-decoding-contract.md).

License: Apache-2.0 OR MIT.
