# Media design: video, packets and containers next to the image traits

Decided 2026-10-14 after an independent design review of the media experiments
in PR 129 (`zencodec-media` workspace under `media/`) and PR 130 (metadata
privacy). This page records what was decided and why; the rules every
implementation follows are in [media-contract.md](media-contract.md).

## The architecture in one paragraph

`zencodec` stays the small `no_std + alloc` image-contract crate and gains one
thing: a time vocabulary (`zencodec::time`: `TimeBase`, signed `Timestamp`,
`Rounding`) next to the existing `FrameDuration`. A new workspace member,
`zencodec-media` (`no_std + alloc`, MSRV 1.88, unpublished until a real codec
and container exercise it), holds the media contracts: packets, tracks,
refcounted frames with an immutable per-frame interpretation, codec sessions
with explicit backpressure, a sans-I/O demuxer that speaks the image
decoders' `Input`/`Request` dialogue, a muxer whose bytes are pulled like a
streaming image encoder's, and the animation traits as a layer over video.
`zencodec-media-testkit` holds the conformance checks and the reference
implementations they are validated against. `zencodec-io`'s drivers run any
byte-stepping stage, image decoder or demuxer alike.

Implementations live elsewhere, so zencodec's build and CI never carry a
codec:

| What | Where |
|---|---|
| Contracts, conformance checks, reference toy codec and container | `zencodec-media`, `zencodec-media-testkit` (this repo) |
| Drivers (slice, `ReadAt`, `Read`, async) | `zencodec-io` (this repo) |
| AV1 sessions over the AV1 backends (rav1d-safe decode; zenrav1e, zenav1-svt, zenav1-aom encode) | zenavif |
| MP4 / ISOBMFF demux and mux | zenavif, next to zenavif-parse and zenavif-serialize |
| WebM / EBML and IVF demux and mux | zenextras (for now) |
| Colour conversion: YUV reconstruction, display interpretation, RGB to YUV | zenpixels-convert |
| Wiring: the session pump with per-track routing, HTTP sources, the registry, the `Video::open` facade, CLI | zenpipe (zencodecs) |

### Backends beyond AV1

Status on 2026-10-15:
- **AV1** sessions are in zenavif PR 69. Decode uses rav1d-safe. Encode uses
  zenrav1e, or zenav1-svt all-intra while its inter frames are corrupt
  (imazen/zenav1-svt issue 26).
- **MP4** demux and mux are in zenavif's `zenmp4` (PR 70).
- **WebM** and **IVF** are in zenextras: `zenwebm`, `zenivf`.

Other codecs are being qualified on zenextras branch
`research/media-codec-qualification`, in `research/media-codecs/README.md`.
Packet adapters wait for that qualification:
- H.264: rusty_h264, Hibernia, rust_h264, oxideav-h264 and wedeo against
  FFmpeg. Not one is qualified yet; rust_h264 is exact on four of nine
  probe cases.
- Audio: Symphonia, ruopus, opus-rs, Rusopus, oxideav-aac, flacenc and
  Rubato.
- VP8: `zenvp8`, a safe-Rust decoder that is byte-exact against libvpx.

## The five calls that matter

```rust
// 1. Codec sessions: explicit backpressure, drain, reset. One shape both ways.
fn send(&mut self, packet: &mut Packet, stop: &dyn Stop) -> Result<SendStatus, SessionError<E>>;
fn receive(&mut self, stop: &dyn Stop) -> Result<Receive<Frame>, SessionError<E>>;
// 2. Containers in: the image decoders' dialogue, reused unchanged.
fn step(&mut self, input: Input<'_>, stop: &dyn Stop) -> Result<DemuxProgress, SessionError<E>>;
// 3. Containers out: the streaming encoder's pull shape plus the writer-capability answer.
fn begin(&mut self, tracks: &[TrackInfo], hint: MuxHint) -> Result<MuxTiming, SessionError<E>>;
// 4. Frames: owned, refcounted, mapped through a guard; two frames may share a picture.
fn map(&self) -> Result<Box<dyn MappedPlanes + '_>, MapError>;
// 5. Time: one rounding rule (ties toward +infinity, the TimingQuantizer rule).
fn rescale(self, target: TimeBase, rounding: Rounding) -> Result<Timestamp, TimingError>;
```

## Decisions

**Time.** One model, two types, one rule. `FrameDuration` measures lengths;
`Timestamp` names instants on a stream clock, signed (pre-roll and edit lists
are negative). Every lossy conversion names its `Rounding`; `Nearest` sends
exact halves toward +infinity, which is what `TimingQuantizer` already does,
so quantized durations and rescaled instants agree (tested over 300 NTSC
frames). Missing timestamps are `Option`, never sentinels, and nothing derives
a timestamp from a nominal frame rate. Delays, edit lists and padding are
carried untouched on tracks and packets and applied once, by the decoder
adapter. PR 129's own clock (ties to the earlier endpoint) is dropped.

**Animation and video.** The animation traits stay the fused container-plus-
codec path for GIF, APNG, WebP and the rest; they are published and
implemented by every animation codec, and GIF has no useful packet model.
Two generic adapters (`VideoAnimationDecoder`, `AnimationVideoEncoder`) make
the same traits work over demuxer + decoder and encoder + muxer. Durations
come from adjacent timestamps; the last one is an explicit `final_duration`.
The streaming animation output (`AnimationFrameEncoder::begin`,
`pending_output`, `take_output`) maps directly onto a muxer's pull output, so
it ships unchanged. Colour conversion between planes and packed canvases is a
caller-supplied `RenderFrame` / `CaptureFrame` (8-bit grey identity included);
the YUV implementations come from zenpixels-convert.

**Frames.** A `Frame` is an owned handle over an `Arc<dyn Picture>`
(rav1d's refcounted picture, owned planes, a pool slot), cheap to clone,
valid after the decoder resets or drops, usable as encoder input without a
copy. Its `FrameInterpretation` (geometry with crop and render size,
subsampling, siting, sample encoding, plane roles, colour claims, epoch) is
immutable and shared, so a queued frame keeps the interpretation it was
decoded under. A frame is a presentation occurrence: AV1 show-existing is
`Frame::represent`, sharing the picture with other timing. The planar view
lives here, not in zenpixels, until a second consumer adopts it; depth and
placement use zenpixels' `SampleEncoding` once it publishes (a local mirror
until then).

**Packets and tracks.** `Packet` has private fields and an owned or shared
payload taken on accept; timestamps optional; container random-access
evidence (`RandomAccess::{Yes, No, Unknown}`), which is not a codec keyframe
flag; configuration epoch; ordinal; side data. A decoder yields 0..N frames
per packet, possibly later. `CodecId` is independent of the container.
`TrackInfo` is built and validated through a builder. Configuration changes
are epochs: `DemuxEvent::ConfigChanged`, `VideoDecoder::configure`, and a
decoder refuses a packet from another epoch.

**Flow control and I/O.** Sessions are synchronous and sans-I/O. `send`
answers `Accepted` or `ReceivePending` (receive first, then retry the
untouched input); `receive` answers `Ready`, `NeedInput` or a stable
`EndOfStream`; `end_input` is idempotent; `reset` discards and keeps the
configuration. Demuxers step over driver-owned input; muxers hand out bytes
through the image encoders' pull API (`OutputBuffer` shared). `MuxTiming`
says whether bytes stream and whether `finish` patches earlier bytes;
`MuxHint::forward_only` makes a patching layout refuse at `begin`. Async stays
out of codecs and containers: `zencodec_io::tokio_io::AsyncCursor` awaits the
source and steps the stage on the calling task.

**Seeking.** By time: `SeekTarget { track, time, Nearest | AtOrBefore |
AtOrAfter, Any | RandomAccessPoints }`. A container whose index needs bytes
answers `Scanning` and reports `Seeked` once positioned. The result is the
actual timestamp, ordinal and random-access evidence, never the request
echoed back. A keyframe flag is not a random-access point; the driver resets
and reconfigures the decoder (sequence-header injection), decodes from the
chosen packet and discards up to the target, and reports the work.
`with_start_frame_index` stays the animation-index hint.

**Cancellation, limits, errors.** A borrowed `&dyn Stop` on every call; a
tripped stop is terminal until `reset` (`FusedDecoder`, the `Fused` pattern).
Backends that poll inside kernels take an owned stop at construction; the
trait only knows the borrowed one. `MediaLimits` bounds container structure
and queues before allocation, reporting `zencodec::LimitKind`
(`PacketBytes`, `Tracks`, `HeaderBytes`, `TableEntries`, `QueueDepth`);
`ResourceLimits` keeps bounding pixels, frames, duration and memory. Errors
are `SessionError<E>`: the implementation's typed error, cancellation,
`Finished`, a `ProtocolViolation`, `UnsupportedMedia`, or a limit. No
`&'static str` errors.

**Colour, HDR, alpha.** Carried, not interpreted: raw CICP from bitstream and
container kept apart with the adapter's resolved claim; unknown siting kept;
conversion that needs an unknown fact fails at conversion time. No implicit
tone map, no sRGB guess. Alpha travels as a plane role or as packet side data.

**Audio and pages.** Audio is pass-through for now: tracks, packets, delays
and padding are in the contracts so remux and copy work and no track is
dropped silently; decoder and encoder traits wait for a qualified audio codec.
Multi-page images (TIFF, PDF) are a third, separate sequence kind.

**Dyn dispatch.** Every session and container trait is object-safe through
shims (`erase_video_decoder`, which also fuses; `erase_muxer`). Per-stream
facts come from the session (`stream_info`, `output_config`), never from
static capability flags.

## What happens to the experiments

- **PR 129** is closed and mined. Adopted with changes: the time code (tie rule
  flipped), the plane and frame vocabulary, the AV1 session shapes, the
  container parsing rules (rewritten sans-I/O), the HTTP rules, the colour and
  seeking rules, the AV1 corpus. Dropped: its own animation clock, the
  spill-to-disk source, the nested workspace with its own lockfile and MSRV,
  and the cross-repo pin runner. Its traits get the fixes an earlier review
  asked for: no `Vec`-only payloads or planes, optional timestamps, private
  track fields, owned codec identifiers, channel layouts, 0..N frames per
  packet. It also discarded the duration `push_frame_timed` returns, which
  would have under-reported rounding after the timing rework.
- **PR 130** is closed: its code reached main through zenmeta. Its privacy
  table and two retention sections are worth carrying into zenmeta's docs.

## Landing order

1. `zencodec::time` and `pts` on animation frames (additive; PR 150).
2. `zencodec-media`, `zencodec-media-testkit`, and the `zencodec-io` cursors
   (this change).
3. IVF and WebM in zenextras; AV1 sessions and MP4 in zenavif; the pump,
   HTTP and registry in zenpipe; the colour kernels in zenpixels-convert.
   Each runs the testkit checks; PR 129's AV1 corpus re-measures its
   sample-exact claim.

Nothing in `zencodec-media` publishes before step 3 proves it with a real
codec and container, and zenpixels publishes `SampleEncoding`.
