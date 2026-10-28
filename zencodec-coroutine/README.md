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
