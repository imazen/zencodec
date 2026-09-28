# Executable media workflows

Run commands from `zencodec/media`. Dependencies are pinned in `Cargo.lock` and
`integration/repos.lock.json`; the full codec test/example workspace needs Rust
1.93. The core contract crate has a smaller Rust 1.89 feature baseline.

```sh
# Exact transcode; refuses timing/color/precision that cannot be represented.
cargo run --release -p zencodec-media-codec-tests --example media -- \
  transcode apng jxl input.apng output.jxl

# Explicit ICC/CICP conversion and quantization to straight RGBA8 sRGB.
cargo run --release -p zencodec-media-codec-tests --example media -- \
  transcode-srgb apng webp input.apng output.webp

# Request t=1001/30000 seconds, choose nearest presentation, return actual PTS.
cargo run --release -p zencodec-media-codec-tests --example media -- \
  extract input.ivf 1001 1 30000 nearest all center clamp output.png
```

`INPUT` also accepts HTTP(S) with byte ranges, a stable strong ETag, and a known
resource length. Index construction is a bounded full scan; subsequent reads
seek to the required initialization/keyframe region and decode dependencies.
`keyframes` selects presentation frames classified by AV1 as keyframes; the
index does not claim every selected frame is an independently decodable access
point. Returned PTS can differ from the request; ties choose the earlier PTS.

The example writes through a temporary file in the output directory and refuses
to overwrite an existing destination. Encoded animation input/output is buffered
by current codec APIs; size/frame/work budgets are explicit. IVF packet writing
supports non-seekable sinks and reports partial-write/flush failures.

Native U16 is preserved by `transcode`; AVIF 10/12-bit quantization requires the
library API with an explicit encoder depth. `transcode-srgb` intentionally
reduces precision. HDR-to-SDR requires an explicit luminance/tone-map policy
through the library; no peak or operator is invented by the example. GIF's
palette and binary-alpha limits are not lossless for arbitrary RGBA content.
Use `transcode_dyn_with` to place a caller-controlled conversion/filter stage
between composited decoding and encoding. The callback owns its allocations
and cancellation inside its kernels.

The library's `animation_to_ivf`, `ivf_to_animation` and `encode_extracted` paths
are exercised by `tests/video.rs` in the codec integration package.
Animation→IVF requires explicit alpha and loop handling. IVF→animation requires
an explicit final frame duration and rejects duplicate/decreasing timestamps.
