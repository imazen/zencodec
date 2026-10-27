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

### Error inspection across pumps and mappings

`PumpError::source()` exposes the selected upstream or downstream error. A pump
must not end a generic error-chain walk before the codec, policy or destination
cause. Type erasure retains that cause and any `At<E>` location trace.

`MapError::Backend` owns `Box<dyn Error + Send + Sync>` on the unpublished media
API. Implementors pass the original backend error, including its trace, instead
of formatting it to a string. `Unavailable` remains the cause-free refusal for
storage without a CPU mapping. Mapping causes stay inspectable through
`AdaptError::Map` and erasure. `just media-errors-check` exercises these paths and
the fused-session terminal rules.

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

### Exact supplied timing

`Muxer::begin_exact_timing(tracks, hint)` prepares an actual instance that
preserves supplied PTS, DTS and explicit durations without rounding or rebasing.
The default refuses with `UnsupportedMedia::ExactTiming`, before calling legacy
`begin` or exposing a header. `DynMuxer` and `MuxerShim` retain that boundary.
The caller requests the behavior directly; no capability query is required.

The implementation must reject an unrepresentable later packet before that
packet changes pending output. A missing DTS may use PTS as decode time when
the format needs one. A missing duration may be refused; this entry point must
not guess it. Drivers retain the exact prepared instance, report prior output
on failure, and cannot retry a different layout after publication. Preparation
is not a guarantee about unknown future input. This timing contract does not
establish metadata privacy, side-data retention or a workspace bound.

The native MP4 implementation prepares fragmented output on both destination
types because its progressive sample tables cannot represent an unknown
nonzero decode origin. Its separate inherent entry point is bridged to this
review trait in the native harness while native dependencies retain their
earlier shared revision. No application chooses a layout from codec capability
flags. `begin` remains the lower-level entry point for deliberately inferred
timing; it is not an equivalent implementation of this stronger request.

Use `finish.apply_to(&mut bytes)` to finalize an in-memory stream; the free
`apply_finish` helper is deprecated. Validation or allocation failure leaves
its contents and length unchanged. All patch ranges are checked before any
mutation, including when a valid first patch precedes an invalid later one.

External writers call `finish.validate(emitted_bytes)` before writing the tail
or patches. The returned length includes the tail. Offsets are relative to this
muxed stream; a writer embedding it in a larger file adds its captured start
position and checks that arithmetic separately. Overlapping patches apply in
listed order. Destination patch support, output allowance and actual I/O remain
the driver's responsibility; an I/O error may leave a partially written file.

The shared media testkit regression failed before the fix and passes with the
replacement and deprecated compatibility path. `just mux-finish-check` passes
media tests, strict lint and the no-default-features WASM check (2026-10-27):
run-heavy rc=0, 3 s, peak RSS 0.30 GiB, minimum available 11336 MiB, peak load
0.19. These are validation-process observations, not muxer memory bounds.

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

Immutable interpretation, timing, frame owners and borrowed plane mappings live
in `zencodec::frame`; the existing `zencodec_media::frame` import paths name the
same types. Media retains codec sessions and `OwnedPlanes`, whose accounted
allocations depend on I/O storage. Core mapping contracts add no driver or
media dependency. Selected-image adapters and native AV1 now use this shared
ownership boundary; packed pixels still use `zenpixels`. The existing
media `SampleEncoding` compatibility type still needs migration to
`zenpixels::sample::SampleEncoding`; this move does not create a second new
sample interpretation implementation.

- A zero-width mapped plane returns empty rows within its declared height,
  without indexing unused stride or overflowing a row offset. A zero-height
  plane has no rows. Non-empty planes validate their full active extent.
- A frame is valid after its decoder resets or drops. Two frames may share a
  picture (show-existing); `shares_picture` tells.
- A frame's interpretation is immutable: a configuration change makes a new
  one, and frames already decoded keep theirs.
- Siting is never defaulted: `ChromaSiting::Unknown` stays unknown. A crop
  keeps absolute coded coordinates, so an odd origin keeps its chroma phase.
  Render size is separate from coded size; pixels are not resized.
- Sample codes are never widened implicitly: a 10-bit code in a 16-bit word is
  `SampleEncoding::new(U16, 10, shift)`, not a full-range 16-bit value.

### Policy before native publication

After an explicit planar-to-packed conversion, adapters can adopt its exact
visible-crop owner through `ImageOutput::from_frame_pixels`. Current packed
descriptor/context stays separate from `source_frame()` component provenance;
timing and presentation survive. This consumes the native frame without keeping
its pixel allocation alive. The ordinary `resolve_image` publication boundary
then sees both current samples and bounded source facts. This constructor does
not convert samples, infer orientation or execute native render-size scaling.

`zencodec_media::policy::PolicyDecoder::new(decoder, resolver)` wraps an existing
`VideoDecoder`. Before returning each native frame it invokes the same
`zencodec::operation::PolicyResolver` used by still-image operations, with a
typed `Question::Frame`. The question borrows the held frame's full planar
interpretation and includes its presentation timing. A successful choice returns
the original frame; it does not convert, copy or relabel its samples.

The callback runs for every presentation. Equal configuration epochs alone do
not justify reusing a decision, and application policy can depend on timestamps.
The bounded report retains recent frame facts and decisions. A native error,
policy refusal, stale token, cancellation or caught unwind makes this operation
terminal and drops its decoder and unpublished frames. Healthy reset remains
available; recovery after a failed operation uses a fresh decoder. Previously
published frames retain their native owners. Native and traced callback causes
remain inspectable; default error formatting excludes their payload text.

This is a publication boundary after native reconstruction. It does not admit
native allocations before decoding, establish a whole-container privacy promise
or implement planar conversion. Presets requiring metadata treatment, rejection
of opaque profiles or a workspace bound refuse because this native publication
offer lacks that evidence. A default preset preserves the native presentation,
including unknown declarations. Codec/container interpretation rules remain the
format adapter's responsibility.

A separate `Question::ImageConversion` chooses a retained conversion before
its destination is initialized. The exact mapping and storage reservations
survive the callback; declared assumptions and integer clipping remain visible.
The private AV1 witness executes it through existing strip kernels and
`ImageOutput::from_frame_pixels`, preserving timing and original signal facts.
This does not turn the native publication gate into a converter. See
[prepared image conversion](spec.md#prepared-image-conversion).

The authored ownership controls in `zencodec-media-testkit/tests/frame_policy.rs`
verify exact strided 10-bit samples and picture identity, cancellation after
callback, stale choices, typed causes, unwind release, terminal work methods and
bounded reports. `just frame-policy-integration` passes those controls, strict
lint, no_std builds, the full workspace suite and API snapshots. Native codec
qualification is recorded separately; these authored controls alone do not
establish a native codec memory bound.

The standalone [native AV1 qualification](../perf/av1-policy-validation/README.md)
executes this guard with pinned native decoder/encoder PRs. It checks original
picture identity, queued configuration facts, typed native causes and failure
cleanup. Policy-approved 8/10/12-bit mono, subsampled YUV and identity GBR frames
re-encode sample-exact under independent FFmpeg/libdav1d decoding. The
[recorded run](../benchmarks/native_frame_policy_2026-10-25.txt) distinguishes
these publication checks from native allocation or conversion guarantees.

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

### Error-chain qualification, 2026-10-25

`just media-errors-check` passed in an export outside workspace patches: media
conformance and error-chain controls, strict Clippy, no_std WASM, and API capture.
run-heavy rc=0, 10 s, peak RSS 0.33 GiB, minimum available 15907 MiB, peak load
0.63. This is test/build-process usage, not a codec memory bound.
