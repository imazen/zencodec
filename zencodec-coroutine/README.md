# zencodec-coroutine

Write a [zencodec](https://github.com/imazen/zencodec) `IncrementalDecode` as
straight-line `async` code. `rustc` synthesises the resumable state machine;
`CoroutineDecoder` polls it behind the ordinary synchronous contract. No
runtime, no executor, no threads; `no_std + alloc`, `forbid(unsafe_code)`.

```rust,ignore
let dec = CoroutineDecoder::boxed(|mut ctx| async move {
    let mut header = [0u8; 16];
    ctx.read_exact_at(0, &mut header).await?;
    ctx.set_info(info);
    ctx.set_output_descriptor(desc);
    ctx.announce(MetadataUpdate::HeaderReady).await?;
    for strip in strips {
        ctx.read_many_at(&mut strip.ranges).await?;
        ctx.write_rows(BatchMeta::final_rows(strip.y), &strip.pixels()).await?;
    }
    Ok(OutputInfo::full_decode(w, h, desc))
});
```

Each suspending `.await` returns control to the driver. Row delivery can take
multiple steps: each step fills at most one buffer allowed by the sink's
`max_buffer_rows`, letting a pull consumer drain it before continuing. The
coroutine copies the offered bytes its reads need into a store (under a byte
cap, released once read; gap bytes are never stored) and each strip once more
into the sink. That suits container walks and headers; bulk pixel paths are
better hand-written, decoding from the driver's bytes straight into the sink.
Measured cost: `zencodec-io/benchmarks/driver_overhead_v2_2026-10-14.md`.

License: Apache-2.0 OR MIT.

## Owned completion details

A producer can move its final metadata, source details and typed extras into
`ctx.set_details(details)` before returning `Ok(OutputInfo)`. The adapter exposes
these through `IncrementalDecode::take_details` only after all pending rows and
`sink.finish()` succeed. The final details replace earlier metadata snapshots;
after taking them, use `details.info` instead of the decoder's borrowed `info()`.

A failure, cancellation or caught producer/sink panic discards pending details.
The coroutine remains terminal. Details cannot change an adopted interpretation
or disagree with the final output dimensions. This channel does not certify
complete container discovery, privacy, or native workspace limits. Codecs that
have not supplied all configured result details must leave it unavailable.

`tests/details.rs` covers successful row delivery, ownership release while the
decoder remains alive, failed/panicking finish, inconsistent dimensions, producer
failure/unwind, cancellation during input and resume, and post-adoption changes.
All 42 coroutine tests and strict all-target Clippy pass with the no-default-
features check (2026-10-29; rc0, 6s, peak RSS 0.25 GiB, including API regeneration).
One existing ignored documentation sketch is unchanged. The initial finish-panic
failure was retained and fixed by poisoning state before final sink callbacks.
