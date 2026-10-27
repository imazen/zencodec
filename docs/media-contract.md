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

## Checked output ownership (`std`)

`zencodec_media::output::PreparedOutput::new` retains the actual muxer and
`Forward`/`SeekBack` destination without writing. Preparation requires the native
`begin_exact_timing` contract; unsupported timing fails rather than silently
falling back. `publish` consumes that preparation. `Output::new` is the immediate
prepare-and-publish convenience; `with_container_timing` is the explicit weaker
alternative. These are synchronous adapters over the sans-I/O `Muxer`, not new
requirements on the no_std codec interfaces.

`send` checks track, clock and epoch before native packet effects. A failure or
unwind leaves the operation terminal. `finish` validates the complete tail/patch
plan before writing, applies offsets relative to the captured output origin,
restores the final position and flushes. I/O failure can leave partial writes or
patches; `Failure` preserves stage, confirmed appended bytes and original cause.
Dropping a prepared or active output never implicitly finishes it.

A slow writer may block these methods. Callers must finish the encoder or use a
separately qualified configuration before allowing writer pressure to retain
raw/reference buffers. `MuxTiming::streams` alone is not such qualification.

### Retained destination evidence

Frame-route callbacks can inspect `FrameRouteOffer::destination`, populated from
`PreparedOutput::facts()` before publication or `Output::facts()` for the same
continuing destination. The selected record retains this bounded value. It
reports actual seek-back access, byte availability before finish, required final
patches and whether exact packet timing was successfully required. Unqualified
routes use `None`; ordinary container timing reports `exact_timing = false`.

This avoids asking application code to coordinate capability queries. Preparation
still refuses unsupported layout/timing before writes. Facts never authorize an
unsupported mechanism, and streaming bytes do not prove safe backpressure on a
live encoder. A full owned frame-to-destination facade remains open; the native
witness still supplies and retains these objects explicitly.

Native validation on 2026-10-27 exercises these facts with real AV1 encoding and
MP4/WebM/IVF output: forward versus prefixed seekable destinations, exact versus
ordinary timing, prepared-policy refusal, and selected-report retention. All 40
native cases, strict lint and the independent gamma verifier passed. Artifacts
on r7900x:
`/mnt/v/output/zencodec/platform-regions-20261026-run/destination-facts-20261027-full-2`;
log `/home/lilith/tmp/zencodec-destination-facts-full-2-20261027.log`.
Run-heavy: `rc=0 42s | peak-RSS 0.57GiB | min-avail 11580MiB | peak-load 1.21`.
This change requires no zenpixels changes. Earlier policy-cost measurements
predate the added destination field and are not measurements of this revision.

### Owned finite encoder-to-destination operation

`staged::StagedEncode` owns the encoder and the exact prepared muxer/writer.
It offers the held frame and actual destination facts to the shared resolver,
retains conversion until selection, checks its returned interpretation/timing,
and stages native packet payloads under `StorageBudget`. Packet additions are
also charged; copying changes storage ownership without rebuilding or dropping
packet metadata. Failed partial admission releases all reservations.

Only `finish` writes the final destination: it drains and drops the encoder before publishing the
prepared header, packets and finish patches. This makes the lifetime boundary
structural instead of relying on the caller remembering to drop an encoder
before a slow writer. Source pictures retained by the caller are outside this
ownership guarantee. The native workspace is not budgeted by this API.

Every failure/unwind is terminal, and conversion/policy refusal writes nothing.
The error retains stage, bounded decisions and its concrete cause, including
partial destination effects. Success returns output stats and the decision
report. Explicit configuration changes fail; the operation handles one finite
fixed-configuration video sequence, not implicit segmentation or live streaming.
It stages the entire compressed sequence in memory by default. Explicit disk
staging is described below; qualified live sequential backpressure remains open.

The executable usage is the module doctest in `zencodec-media/src/staged.rs`.
Native AV1/MP4 validation lives in
`perf/av1-policy-validation/src/staged_encode_tests.rs`, including an independent
pixel reader and a writer that asserts the native encoder has already dropped.
No zenpixels change or dependency addition was required.

Validation on 2026-10-27: all 42 native AV1 cases, strict lint and the independent
gamma verifier passed. The owned-path cases check identity/conversion execution,
independently decoded MP4 pixels, encoder-drop-before-write, policy refusal,
conversion target drift/unwind, cancellation, exhausted compressed storage,
wrong native packet epoch before publication, and a 13-byte partial writer
failure. Packet accounting separately checks side-data retention, shared charge
lifetime and cleanup after partial admission. These are correctness/lifetime
checks, not throughput or whole-pipeline memory measurements.
Artifacts on r7900x:
`/mnt/v/output/zencodec/platform-regions-20261026-run/owned-staged-20261027-complete`;
log `/home/lilith/tmp/zencodec-owned-staged-complete-20261027.log`.
Run-heavy: `rc=0 45s | peak-RSS 0.57GiB | min-avail 11569MiB | peak-load 0.64`.
Reproduce with `just staged-encode-check <fresh-artifact-directory>` or the full
`just av1-policy-check <fresh-artifact-directory>`.

The public owned path now also executes the existing real YCbCr conversion
plan over native-decoded 8/10/12-bit, 4:2:0/4:2:2/4:4:4, full/narrow inputs into
one fixed RGB8 AV1/MP4 output. The conversion implementation is shared with the
prototype witness, not copied. Policy observes the actual source and target,
reserved conversion storage and non-preservation of samples/extended range;
requiring extended range refuses before conversion or writer I/O. Independent
container decoding matches the converted RGB samples. All 43 native cases and
strict lint passed on 2026-10-27. Evidence on r7900x:
`/mnt/v/output/zencodec/platform-regions-20261026-run/public-conversion-20261027`;
log `/home/lilith/tmp/zencodec-public-conversion-20261027.log`.
Run-heavy: `rc=0 45s | peak-RSS 0.58GiB | min-avail 12772MiB | peak-load 0.65`.
No production API or zenpixels change was needed for this integration.

At that revision this exposed a contract gap: generic `VideoEncoder` reported container
configuration, but no guarantee of the prepared encoding fidelity. The public
operation therefore reported unknown encoder fidelity even when this native
witness uses lossless AV1. Required/preferred fidelity must eventually be derived
from the actual retained encoder, without caller-supplied capability coordination
or inferring guarantees from codec identity. Conversion fidelity remains separate.

The replacement contract adds `VideoEncoder::output_fidelity()`, with unknown
(`None`) as the default. It guarantees encoding fidelity for all accepted frames
of this actual configured instance; it cannot be inferred from requested flags
or codec identity. Boxed, fused and error-erased encoders forward the evidence.
`StagedEncode` binds it together with the prepared configuration, offers it to
policy, and rejects a change before accepting another frame or publishing the
staged sequence. Conversion preservation remains independent: lossless encoding
does not certify an earlier YCbCr conversion, quantization or clipping step.

The native witness supplies evidence only for its quantizer-zero configurations,
qualified by independent decoded samples. The pinned native crate itself still
uses the unknown default until its adapter adopts the method; this is a measured
adapter gap, not a claim that every video codec is now qualified. Native tests
exercise the evidence through Box, FusedEncoder and VideoEncoderShim together,
and inject a changed guarantee without changed container configuration bytes.

Prepared-fidelity validation: all 43 native cases and strict lint passed with
Box/FusedEncoder/VideoEncoderShim forwarding and a changed-fidelity refusal.
Full evidence on r7900x:
`/mnt/v/output/zencodec/platform-regions-20261026-run/encoder-fidelity-20261027`;
log `/home/lilith/tmp/zencodec-encoder-fidelity-20261027.log`.
Run-heavy: `rc=0 45s | peak-RSS 0.58GiB | min-avail 12886MiB | peak-load 0.61`.
This additive default method leaves unadapted native implementations unknown;
it does not assume their requested quantizer establishes a generic guarantee.

### Explicit disk staging and replay failures

`staging::Staging::disk(File, DiskLimits, StorageBudget)` owns a private scratch
file positioned at its end. `StagedEncode::with_staging` accepts that store.
No implicit file creation or memory-to-disk fallback occurs. The caller owns file
privacy and deletion. External aliases/mutation are unsupported.

Disk writes happen while the encoder may retain raw buffers. Frame-route policy
sees the actual store kind, accumulated payload bytes and payload allowance.
This is explicit finite staging, not a qualification for live backpressure.

Separate limits cover disk bytes, indexed packets and additions per packet.
Replay admits each packet's payloads into one shared charged owner, preserving
metadata, timing and additions. Muxer retention keeps charges alive. First-packet
admission/read precedes header publication; later replay failures may leave
output. `staged::Failure::emitted` reports confirmed final-destination appends
for replay and writer failures; the cause retains the concrete error. Failed
patches can also leave partial overwrites. Spool bytes are not destination bytes.

Validation exercises native AV1 to MP4 with byte-identical memory/disk output,
independent decoded pixels, refusal before destination effects, separate disk
and replay caps, read-only scratch, metadata/addition retention, prefix
preservation and truncated replay. The late-replay fault deliberately mutates
scratch after header publication to verify the diagnostic boundary; this is not
support for externally mutable scratch. No zenpixels change is required.

The complete native suite passed on 2026-10-27 with 45 cases, strict lint and
the independent gamma verifier. Focused media tests/doctests and all five owned
native cases passed. Resource-wrapper receipt for the full suite:
`rc=0 39s | peak-RSS 0.05GiB | min-avail 13102MiB | peak-load 0.56`.
This is a validation-run resource observation, not a staging memory comparison
or a throughput benchmark.

### Live writer scheduling still requires source evidence

The native JPEG scheduling matrix in
[the JPEG qualification](../perf/jpeg-policy-validation/README.md#native-output-scheduling-qualification)
preserves exact bytes across undrained finish, complete/partial drains and short
writes for fixed/optimized and sequential/progressive configurations. Tiny
images correctly report no streaming. This validates the byte protocol, not
retained raw/intermediate allocations.

A full-raster `PixelSlice` caller still owns all input during a writer wait.
Consequently an encoder-only retention declaration cannot authorize a
whole-operation streaming bound. A live route must bind the actual pausable
source, configured codec state, pending-output ownership and publication
effects together. Unknown source/native retention must stay unknown; neither
`OutputTiming` nor a bounded output queue is a substitute. These facts belong
to a retained route offered to the shared resolver, rather than another
mandatory caller capability-query sequence.

### One resolver for actual output schedules

`OutputScheduleOffer` distinguishes finish-first from live output and carries
an optional qualified wait-retention bound. The shared question records actual
byte timing and source mechanism. Required unknown bounds refuse; a pull source
does not imply bounded retained storage. This is a wait-time scope, not peak
operation memory.

The owned video operation supplies finish-first evidence automatically through
`FrameRouteOffer::output_schedule`. Native AV1 tests accept a required
finish-first schedule, reject required live mode, and reject an unknown required
wait bound before accepting frames or publishing bytes. JPEG executes the
standalone question with generated strided rows and short writes. The shared row-source driver described below now executes these choices;
qualified native wait-memory bounds remain open.

Validation on 2026-10-27: the JPEG schedule matrix and generated-row policy
witness passed with strict lint; all six owned AV1 cases passed with media
tests/doctests and native lint. Workspace all-feature tests and browser checks
passed. Core controls cover unknown-versus-zero retention, required schedule
refusal, contradictory live offers and stale tokens. No zenpixels change or
dependency addition was required. Validation resource receipt for core/JPEG:
`rc=0 18s | peak-RSS 0.70GiB | min-avail 12414MiB | peak-load 0.90`.
This includes compilation and is not a codec memory/performance comparison.

The generic synchronous row-source driver is now implemented in
`zencodec_io::row_encode`; the native JPEG scheduling witness calls it instead
of maintaining a private loop. It preserves source descriptor/context, stages
compressed payloads under admission, releases the source and encoder before
finish-first writer waits, and retains selected decisions plus confirmed partial
writes on runtime failures. Native JPEG tests cover both schedules, strided
rasters, context retention, fixed/optimized/progressive configurations, source
failure/drift/unwind, cancellation, output/storage refusal and partial writes.
The shared memory bound remains unknown; this driver does not qualify native
workspace. Raster disk staging is described below.

Row-driver validation on 2026-10-27: all 32 then-existing native JPEG cases
passed with strict lint; the added compressed-staging admission control and all
four row-driver cases subsequently passed. Workspace all-feature tests and the
new public doctest passed. Native full-suite resource receipt:
`rc=0 4s | peak-RSS 0.53GiB | min-avail 13023MiB | peak-load 0.21`.
This is a correctness-run observation, not a throughput or memory comparison.
No zenpixels change or dependency addition was required.

Callback-boundary regression (2026-10-27): native JPEG confirms that policy
changes to dimensions, color context or acquisition fail before header output.
The same changes during writer callbacks fail before source reads and preserve
confirmed appends, the selected decision and the original typed refusal.
All five row-driver tests and strict native/driver lint pass. Resource receipt:
`rc=0 3s | peak-RSS 0.53GiB | min-avail 12843MiB | peak-load 0.31`.
This correctness run does not establish a codec memory or performance bound.


Raster disk staging uses `Options::with_disk_staging(file, replay_bytes)` and
reports its actual store through `OutputScheduleOffer::staging`. It shares the
existing policy question rather than introducing another resolver. The file
must be private, exclusively owned and positioned at EOF; prefixes survive.
The output limit bounds spooled payload; replay allocates an admitted chunk
only after source/native release. Spool writes themselves may block with native
state alive. Final-destination appends and original causes survive replay/write
failure. Caller path privacy and cleanup remain explicit responsibilities.

This byte-stream replay does not need a per-chunk index. Video packets retain
metadata and require an index plus admitted packet owners; the two currently
have separate internal storage implementations. Unifying those internals does
not require callers to adopt a new policy mechanism.

Native raster-disk validation on 2026-10-27 covers 32 combinations: fixed/optimized
Huffman × sequential/progressive × success, read-only spool, replay admission,
output cap, destination failure, policy failure, source failure and cancellation.
Success preserves exact encoded bytes, independent JPEG pixels and scratch
prefixes; destination waits observe released source/encoder owners. All six
row-driver tests and strict lint pass. Resource receipt:
`rc=0 3s | peak-RSS 0.53GiB | min-avail 13068MiB | peak-load 0.59`.
Workspace all-feature tests/doctests pass. These are correctness runs, not
memory-bound or throughput measurements. No zenpixels change is required.


The patch representation and writer wrappers now live below media:
`zencodec::output_patch::OutputFinish` and `zencodec_io::destination`.
Media re-exports identical types under the existing names, including MuxFinish.
No native muxer migration is required. This prepares a common raster/container
patch contract without adding a second range validator or seek-origin policy.
Raster Encoder negotiation/execution remains a separate unimplemented step.

Shared-output relocation validation (2026-10-27): all workspace feature tests,
strict core/I/O/media lint, and all 13 native mux-output tests pass. The native
controls include forward and prefixed seek-back output in three containers,
independent AV1 sample/timing inspection, malformed patches before final effects,
partial writes and terminal cancellation. Receipt:
`rc=0 36s | peak-RSS 0.58GiB | min-avail 12542MiB | peak-load 0.93`.
Strict default Rustdoc and the API-documentation gate also pass. This qualifies
the common implementation through media; it is not native raster-patch support
or a new performance/memory measurement. No zenpixels changes were needed.
