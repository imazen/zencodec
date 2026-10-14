# zencodec-io

Drivers that run a [zencodec](https://github.com/imazen/zencodec) decoder over
real input: a slice, a random-access source, a `std::io::Read` stream, or an
async runtime. Codecs do no I/O; this crate owns the loops.

| input | resumable decoder (`IncrementalDecode`) | pull-native decoder (`push_decoder_from_source`) |
|---|---|---|
| slice | `drive_from_slice` | `SliceReadAt` |
| `ReadAt` (file, object store, FFI callback) | `drive_from_read_at` | direct |
| `std::io::Read` stream | `pump_read_to_sink` | `BufferedReadAt` |
| async runtime (feature `tokio`) | `thread::decode_on_thread` | `thread::pull_decode_on_thread` |

Every driver takes `Option<&dyn how_far::Pulse>` for cancellation and progress
(`Pulse: Stop`; a plain stop passes as `how_far::StopOnly::borrowed`). The
bridge re-checks it inside its blocking waits. Measured overhead per driver
shape: `benchmarks/`.

`no_std + alloc` by default. `std` adds the stream pump, `FileReadAt` and
`BufferedReadAt`; `tokio` adds the dedicated-thread bridge (channels only, no
runtime). Design: `docs/range-decoding-contract.md` in the zencodec repo.

License: Apache-2.0 OR MIT.
