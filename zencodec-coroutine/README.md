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

Each suspending `.await` is one poll round trip through the driver. Rows are
copied once into the sink. Input is kept only until read, under a byte cap.

License: Apache-2.0 OR MIT.
