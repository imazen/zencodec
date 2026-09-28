# Coordinated media implementation

Started 2026-09-27. This is an experimental implementation, with no publication
or stable API commitment. Work is delivered as coordinated PRs; the integration
manifest pins their tested revisions.

The implementation must preserve native code values, sample/color interpretation,
exact frame timing, ownership and errors. Unsupported backend/container features
must be reported explicitly. A bounded pipeline includes codec references,
lookahead, retained frames, parser buffers and conversion scratch in its budget.

Work order:

1. Reproducible synthetic corpus and independent scalar color references.
2. Backend metadata, packet lifecycle and frame-ownership fixes.
3. Checked integer sample vocabulary, borrowed native planes and color conversion.
4. Exact timeline, sequential decoding, random access and image extraction.
5. Incremental encoding/muxing, bounded source/sink adapters and cancellation.
6. Animation composition, supported-format transcodes and metrics integration.
7. Pinned multi-repository checkout/test runner and published PR evidence.

## Verified foundation

- 48 original synthetic AV1 files, 192 presentations, natively encoded by
  zenrav1e and decoded by rav1d-safe, compared byte-for-byte to independent
  Python source samples. Checked mapped planes preserve every native code.
- Every unsigned U8/U16 depth/shift combination checked in zenpixels. Raw planes
  preserve code depth and bit placement without changing image U16 semantics.
- 219 Python Decimal reference cases for full/narrow YCbCr and sRGB/PQ/HLG.
- Production native YUV/GBR/monochrome reconstruction, explicit siting, preserved
  odd-crop phase, and unclipped source-encoded RGB rows. All 192 matrix vectors
  and independent spatial/crop references pass. See [color contracts](COLOR.md).
- Explicit sRGB/PQ/HLG/BT.1886/display-linear interpretation in absolute cd/m²,
  with existing production transfer kernels and primary matrices checked against
  independent Decimal references. HLG includes black lift and coupled OOTF.
- Packed RGB/gray → native 8–16-bit components, explicit matte policy, source
  color validation, phase-aware chroma decimation and bounded allocation.
  Converted 8/10/12-bit codes survive native lossless AV1 exactly.
- Whole-file byte-array adaptation with a hard byte budget and immutable seekable
  snapshots that spill to explicit disk storage above a memory threshold.
- Exact signed rational time, explicit rounding, checked integer extrema.
- Animation cumulative-endpoint quantization with one million frames per clock,
  explicit zero/halfway policy and checked overflow without state corruption.
- Bounded IVF presentation index, exact nearest/directional/keyframe extraction,
  fresh decoder state, sequence initialization, and actual returned PTS.
- HTTP(S) Read/Seek with one-chunk cache, strong ETag pinning, checked ranges,
  explicit unknown length, failed-fetch poisoning and request/body timeouts.
- Sequential bounded IVF reading and writing with no Seek requirement, fragmented
  reads, every-byte truncation checks, extended headers, both unknown count
  conventions, signed timestamps, output errors and poisoned partial I/O.
- AV1 owned frames and guard-owned mappings survive later decoding and decoder
  drop; raw color codes and chroma position remain available.
- Native AV1 encoding emits packets before end-of-input, preserves variable and
  negative presentation timestamps through lookahead/reordering, accepts native
  8/10/12-bit code values in either word packing, and writes to non-seekable sinks.
  Queue exhaustion, cancellation, disconnects and final flush errors are explicit.
- Backend packet backpressure, timestamp/offset passthrough and incremental
  drain/reset tested in checked and actual frame-threaded configurations.

## Animation and video integration

- Thirty original native animation files: five formats × three dimensions
  (1×1, 17×13, 257×259) × opaque/binary-alpha content. Five composited frames
  include repeated canvases, erasure, zero delay, unequal delays, and finite loops.
- All 150 directed format/fixture transcodes preserve the representable visible
  pixels, exact durations, canvas size, frame count, and total plays. Hidden RGB
  at alpha zero is not a universal lossless guarantee for 8-bit codecs.
- All 72 APNG/JXL/AVIF high-precision routes preserve every 10/12-bit source code
  expanded to full-domain U16, partial alpha, native linear/sRGB/PQ/HLG color,
  three frames including an identical repeat, and 1001/30000-second timing.
  Equivalent authoritative ICC and CICP signaling are both accepted and checked.
- Explicit frame transforms run between composited decode and encode. A real
  Adobe RGB ICC animation is converted through MoxCMS to sRGB and checked through
  APNG/WebP/JXL/AVIF encoders with partial alpha and exact timing. Current pixel
  context, rather than stale original metadata, drives conversion.
- GIF/WebP reject unrepresentable fractional timing by default. Explicit
  cumulative-endpoint quantization keeps sixty 1/60-second frames at one second
  without accumulating independent per-frame rounding error. Unrepresentable
  loop counts are errors.
- Animation ↔ native AV1 IVF has actual APNG roundtrip tests with exact variable
  frame timing and an explicit alpha matte. IVF's absent loop/final-duration
  fields require explicit caller policy; they are not guessed.
- Selected AV1 presentations can be encoded through an ordinary still encoder;
  nearest/before/after and all/keyframe selection tests check actual returned
  frames, timestamps and every packed sample.

## Explicit limits and remaining work

The production video container is AV1 IVF. MP4/WebM demuxing/muxing, audio,
live discontinuities and hardware decode are not implemented here. Animation
codec adapters still accept complete compressed byte arrays and several buffer
encoded output; the bounded source adapter does not turn them into incremental
network decoders. Native AV1 packet encoding/IVF writing is incremental.

The conversion entry points keep source interpretation, display rendering, and
encoding quantization separate. Perceptual tone/gamut mapping needs a chosen
operator and target; clipping is not tone mapping. Unknown chroma siting, unknown
color interpretation, unsupported precision and missing timing are explicit
errors or caller policy, never guesses. Metrics must use explicit input domains;
see the source audit for the current zenmetrics CPU ingress limitation.

The Linux checks do not claim runtime validation of Apple, Windows, Android,
GPU, or ARM backends. Backend-specific release gates and measured pre-existing
failures are recorded in their owning PRs.
