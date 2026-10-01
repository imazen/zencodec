# Exact animation timing

`animation::FrameDuration` carries a reduced, nonnegative rational number of
seconds. It preserves source timing: zero, fractions of a millisecond and large
native JXL durations. It applies no player minimum or estimated frame rate.
`AnimationFrame::duration()` and `OwnedAnimationFrame::duration()` carry that
value through borrowing and ownership changes. The legacy `duration_ms()`
accessor truncates fractional milliseconds and saturates to `u32::MAX`.

Ordinary encoding uses `AnimationFrameEncoder::push_frame_timed()` (and the
same method on `DynAnimationFrameEncoder`). It returns the actual encoded
`FrameDuration`, making quantization observable. Adapters preserve representable
source timing where possible and otherwise round to their destination clock.
`TimingQuantizer` shares exact rational phase accounting: it rounds cumulative
presentation endpoints, not every frame independently. Zero is not changed to a
browser playback minimum. Adapters advance a copy and commit it only after the
frame has been accepted. Invalid pixels, cancellation and timing overflow must
not consume that frame's quantizer state.

An exact transcode explicitly calls `push_frame_exact()`. The adapter must encode
the duration exactly or reject it before accepting the frame. This returns an
ordinary error, never a timing-induced panic; earlier frames are not rolled back.
The default rejects even integer milliseconds because an unadapted legacy
`push_frame()` might clamp or narrow them. The normal method's compatibility
default delegates to the exact path; codecs override it to offer quantization.
`EncodeCapabilities::animation_timing()` advertises these implemented operations,
not representability of every possible rational.

Legacy millisecond constructors remain useful, exact conveniences. The legacy
`push_frame` entry point remains source-compatible. The lossy `duration_ms()`
projection is a compatibility path; migrate callers to `duration()` before
removing it in a future breaking release. No playback clamps or pixel conversions
are introduced by rational timing.

Native adapters validate destination fields: GIF centiseconds, WebP milliseconds,
APNG rational delays, and AVIF/JXL clocks. Long durations still cannot wrap or
saturate. A buffered encoder can preflight a complete timeline; streaming cannot
promise that a later frame will be representable. The shared quantizer bounds
its exact rational phase to u128 and rejects arithmetic overflow without mutation.
Video transport, DTS/PTS and playback scheduling remain outside these contracts.

## Main integration validation

The timing API is combined with current metadata retention and inventory support
at `f6175c37`. `just test-animation-hdr` passes workspace tests (including exact
rational timing and diffuse-white retention), strict workspace Clippy, no-default
WASM compilation, public docs and regenerated API snapshots. Existing ignored
examples remain unchanged. Validation uses zenpixels/zenpixels-convert
`789d69db`, matching the consuming codec integration; no crate is published.
The r7900x capped build/test wrapper reports 12 s and 0.33 GiB peak RSS; these
are whole-job observations, not codec memory or speed measurements.
