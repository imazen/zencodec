# zencodec-media-testkit

Reusable conformance checks for native sessions, containers and frame owners.
This crate is a test dependency; it does not wrap a production codec.

Start with `check_video_decoder_contract` / `check_video_encoder_contract` and
`check_demuxer_contract` / `check_muxer_output_honesty`. Run
`check_frame_interchange(&frame)` on native outputs to compare direct mapping,
selected-image mapping and canonical planar lending. The check retains full
current interpretation, timing and presentation across paths, and compares
active U8/U16/F32 sample bits without treating row padding as pixels.

Factories should cover odd dimensions/crops, independent strides, 10/12-bit
shifts, mixed-depth alpha, retained old epochs and source/current color
separation. Add independent native-reader controls: two paths agreeing is not
proof of correct decoding. Mux presentation admission does not prove playback.

The executable migration guide is in
[frame_contracts.rs](../docs/examples/frame_contracts.rs); the full contract and
qualification scope are in [the implementation stack](../docs/core-contract-stack.md).
