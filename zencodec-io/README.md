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

The async drivers keep the decoder in the task between batches and run each
batch on a `Compute` (`spawn_blocking`, a rayon pool, or `Inline`), so no
thread is held while bytes are in flight and codecs stay synchronous.
`transcode_async` streams rows into an encoder and its output to an
`AsyncWrite`, keeping memory bounded end to end. Measured overhead per driver
shape: `benchmarks/`.

`no_std + alloc` by default. `std` adds the stream pump, `FileReadAt` and
`BufferedReadAt`; `tokio` adds `tokio_io` (it uses `tokio::sync` and
`tokio::io` only and never starts a runtime). Design:
`docs/range-decoding-contract.md` in the zencodec repo.

License: Apache-2.0 OR MIT.
