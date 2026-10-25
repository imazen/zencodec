# Media contract: rules for sessions, containers and frames

What every `zencodec-media` implementation does. The design behind these rules
is in [media-design.md](media-design.md). `zencodec-media-testkit` checks the
rules marked (checked); run those checks against your implementation.

## Codec sessions (`VideoDecoder`, `VideoEncoder`)

- `send` answers `Accepted` when it took the input: a packet's payload is
  taken, a frame handle is cloned. It answers `ReceivePending` when output
  must be received first: the input is left untouched, and the next `receive`
  returns `Ready` (checked).
- `receive` returns one output, `NeedInput`, or, after `end_input` and once
  everything is out, `EndOfStream`, which stays `EndOfStream` (checked).
- `end_input` is idempotent; `send` after it is
  `ProtocolViolation::SendAfterEnd` (checked).
- Receiving eagerly or only when pushed back gives the same output (checked).
- A decoder may produce zero, one or several frames per packet, and may
  produce them later than the packet that coded them.
- A decoder refuses a packet whose configuration epoch is not the configured
  one (`ProtocolViolation::EpochMismatch`, checked). `configure` starts an
  epoch: before the first packet, after a `ConfigChanged` event, after a
  `reset` for a seek.
- `reset` discards queued input, output and references and keeps the
  configuration. Frames already returned stay valid, and so do their
  mappings (checked). After `reset`, the same packets decode to the same
  frames (checked).
- Every call checks its `stop`. Behind `erase_video_decoder` (or
  `FusedDecoder`), a tripped stop or an error is terminal: every later call is
  `Finished` until a successful `reset` (checked).
- An encoder has its `output_config` ready before the first frame, gives
  every packet the presentation timestamp of the frame it codes (checked), and
  refuses a frame on another clock (`ClockMismatch`, checked). It copies or
  clones what it needs, so the caller may drop a frame after `Accepted`. A
  queue too short for the encoder's lookahead is an error at construction,
  not a stall.
- Sessions do no I/O, start no threads, and never block waiting for input.

`erase_video_encoder` now applies `FusedEncoder`: all errors, including native
validation errors, are terminal. Later send/receive/end-input calls return
`Finished` without re-entering the backend. Cancellation is checked before work.
An encoder has no reset, so recovery uses a fresh session. `VideoEncoderShim`
remains an error-only adapter for a backend with an explicitly documented
retryable validation contract. `output_config` remains readable after failure.
Both fused wrappers stay failed if a backend or stop unwinds and the caller
catches the panic; only a successful decoder reset reopens a fused decoder.
These properties are checked by `zencodec-media-testkit/tests/encoder_fusing.rs`.

## Demuxers

- `step` takes a prefix of the input it asked for and returns an event, or
  `Need` with a non-empty `requested()`. Input at an offset it did not ask
  for is not consumed (checked).
- The events do not depend on how the input arrives: windows of any size, the
  random-access driver, the stream driver, the async driver (checked).
- Truncated input fails as the demuxer's own error (`SessionError::Codec`) or
  ends with a prefix of the events; it never breaks the driver protocol
  (checked). Structural limits (`MediaLimits`) are checked before the
  allocation they guard and fail as `SessionError::Limit`.
- `tracks()` is filled at `Headers`. Every packet names its track.
- Timestamps are what the container records, on the track's clock; missing
  ones stay missing; nothing invents a timestamp from a frame rate or picture
  order count.
- `RandomAccess` is container evidence; `Unknown` when the container records
  nothing (IVF). It is never inferred from the codec's keyframe flag.
- A container that reads out of order (an MP4 with `moov` at the end) asks
  for ranges and reports `AccessPattern::RandomAccess`.
- `seek` positions at once (`Positioned`) or answers `Scanning` and reports
  `Seeked` from a later step. The reported position is the actual timestamp,
  ordinal and random-access evidence of the next packet of that track, never
  the request echoed back (checked); a random-access-scoped seek lands on a
  packet the container marks `Yes` (checked). Selection ties choose the
  earlier timestamp.

## Muxers

- `begin` once (`BeginTwice` otherwise, checked); its `MuxTiming` says whether
  bytes stream and whether `finish` patches earlier bytes. With
  `MuxHint::forward_only`, a layout that would patch refuses
  (`UnsupportedMedia::ForwardOnlyLayout`) or is replaced by one that does not
  (checked).
- Draining changes memory, never output: never, fully, in uneven prefixes
  (with `consume_output` clamping an over-large request) and through
  `take_output` give the same file (checked). Use
  `zencodec::encode::OutputBuffer` for the output.
- A muxer that reported `AT_FINISH` keeps `pending_output` empty; one that
  reported no patches returns none (checked).
- A muxer refuses what its container cannot carry (a codec, an epoch change,
  a timestamp it cannot represent) instead of dropping it; tick quantization,
  where a container's clock forces it, is reported.

## Frames

- A frame is valid after its decoder resets or drops. Two frames may share a
  picture (show-existing); `shares_picture` tells.
- A frame's interpretation is immutable: a configuration change makes a new
  one, and frames already decoded keep theirs.
- Siting is never defaulted: `ChromaSiting::Unknown` stays unknown. A crop
  keeps absolute coded coordinates, so an odd origin keeps its chroma phase.
  Render size is separate from coded size; pixels are not resized.
- Sample codes are never widened implicitly: a 10-bit code in a 16-bit word is
  `SampleEncoding::new(U16, 10, shift)`, not a full-range 16-bit value.

## Colour

- Colour claims are carried with provenance: what the bitstream says, what the
  container says, and what the adapter's format rule resolved. Unknown and
  reserved CICP codes are kept.
- Reconstruction (matrix and range), transfer decoding, primary conversion,
  display rendering and tone mapping are separate operations, each with
  explicit parameters, in zenpixels-convert. None of them is implied by a
  codec contract. There is no implicit HDR target and no implicit sRGB.
- A conversion that needs a fact the stream left unknown fails when it runs,
  not when the frame is decoded.
- Non-opaque pixels written to a codec without alpha need an explicit policy
  (refuse, or composite on a stated matte).

## Time

- Instants are `zencodec::time::Timestamp` on the track's `TimeBase`; lengths
  are `FrameDuration`. Conversions are exact or name their `Rounding`;
  `Nearest` rounds halves toward +infinity, like `TimingQuantizer`.
- Codec delay, seek pre-roll, edit-list offsets (on `TrackInfo`) and
  discard padding (`Packet::discard`) are carried untouched by demuxers,
  muxers and stream copies, and applied once, by the decoder adapter
  configured from the track. A container that cannot record one refuses the
  packet or track instead of dropping it.

## Animation over video

- `VideoAnimationDecoder` gives each frame the time until the next
  presentation, and the last one an explicit `final_duration`; frames carry
  their `pts`. `AnimationVideoEncoder` converts durations to timestamps on
  the encoder's clock exactly or refuses with
  `UnsupportedOperation::AnimationTiming`; a declared frame count binds
  `finish` (checked). Round trips keep pixels and durations; draining never
  changes the bytes; bytes become pending across every `lag + 1` frames when
  the muxer streams (checked).

## I/O and transports

- CPU stop tokens cannot interrupt a blocking read; transports carry their own
  timeouts.
- An HTTP range source pins the representation (strong `ETag` with
  `If-Match`), asks for identity coding, times out per request, and never
  falls back silently to a full download.
- Byte-array-only codecs say so (`AccessPattern`, capabilities); they are not
  dressed up as streaming.
