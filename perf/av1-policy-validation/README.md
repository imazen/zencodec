# Native AV1 policy qualification

## Container destination and timing qualification

The shared `Muxer::begin_exact_timing` entry point is now exercised through
the native MP4 implementation at `11b2b1d6`. Default implementations return
`UnsupportedMedia::ExactTiming` before publishing a header. The private writer's
`new_checked` requests this behavior directly and fuses later failures.

The native MP4 exact route chooses fragmentation before publishing, keeps that
actual prepared instance and requires explicit packet durations. Sixteen native
cases cover 8/10-bit AV1, 1/1000 and 1/90000 clocks, zero and 40 ms decode origins,
and short-writing forward/prefixed-file destinations. All six variable durations,
distinct PTS/DTS and exact decoded components survive independent inspection.
Missing durations, negative decode time and overflow refuse without increasing
the writer's confirmed append count; the driver then refuses further packets.

The native repository still uses its earlier shared dependency revision. A thin
test adapter delegates the new trait hook to the native inherent method; it adds
no packet copies, buffers or timing decisions. That hook must move into the
native trait implementation when the shared review dependency graph is aligned.
IVF/WebM have no qualified exact-timing implementation in this trial and refuse
the stronger entry point. This is partial progress on issue 172, not closure:
coupled encoder/converter/container preparation, full metadata/side-data
requirements and bounded live output remain unimplemented. The exact MP4 disk
route currently uses fragmentation; progressive optimization requires known
decode-origin evidence. No new pixel APIs or kernels were required here.

The private `mux_output.rs` adapter runs the same `Muxer` protocol against IVF,
WebM and MP4. `Forward(writer)` derives a forward-only destination;
`SeekBack::new(writer)` captures the actual initial file position. The shared
write loop handles interrupted and short writes, validates packet clocks/epochs
before native effects, preflights the complete finish, applies relative patches
and restores the final position. It has no container-specific branch.

The 12-case matrix is three containers × two destinations × 8/10-bit AV1.
Six frames per case pass independent FFmpeg/libdav1d component readback and
FFprobe timestamp checks. Disk cases use real files with an existing prefix;
three-byte writes and an initial interruption preserve their contents. IVF's
count-patching mode writes one four-byte patch on disk; progressive MP4 writes
one eight-byte patch; WebM needs none. Forward-only cases use no patches.
These are correctness/data-movement observations, not throughput measurements.

Timing inspection disables FFprobe's codec parsing and timestamp filling
(`-fflags +noparse+nofillin`), so the assertions inspect demuxed container fields.
FFmpeg 6.1's [parser path](https://github.com/FFmpeg/FFmpeg/blob/n6.1/libavformat/demux.c#L1100)
can clear packet durations. CI exposed that difference from FFmpeg 8.0.1;
the exact-duration assertions remain unchanged. Pixel readback still uses the
full independent decoder. The focused native gate passed with these flags
on 2026-10-27 (run-heavy: 13 s, peak RSS 0.54 GiB).

The subsequent CI-version reproduction isolated a second difference:
FFprobe 6.1.1 omits explicit durations from these fragmented MP4 files even
with those flags. The same inspector reads progressive MP4 and WebM durations.
CI therefore builds FFprobe 8.0.1 for timing only, with the source checksum
recorded beside the build. `AV1_TIMING_FFPROBE` selects that inspector; ordinary
FFmpeg/libdav1d remains the independent pixel decoder. Every exact PTS/duration
assertion remains required. `just av1-timing-oracle-build <directory> 6.1.1`
reproduces the older reader; the recipe defaults to 8.0.1.

The builder also recovers a partially restored external source directory: when
`configure` is absent it preserves that directory and extracts the verified
archive again. A reproduced partial-cache build produced FFprobe 8.0.1 while
retaining its original marker file; this changes no timing expectations.

Timing is a separate contract. The native IVF outputs preserve PTS but report
no explicit packet durations; WebM and MP4 preserve all six authored durations.
A second matrix starts at 40 ms: IVF, WebM and fragmented MP4 preserve the shifted
PTS, while the current progressive MP4 implementation refuses at finish, after
1748 bytes have been appended in that fixture. `begin` therefore does not certify
all future timing requirements. The test authors explicit DTS for that origin
probe; it does not claim the native encoder supplied a DTS field.

The driver retains the original I/O/native cause and the stage plus confirmed
append count. A malformed final patch changes neither the pending tail nor
previously written bytes. Partial header failure leaves its confirmed 13-byte
prefix. Cancellation and timeout between packet writes keep their reason and
make the operation terminal. This witness uses a synchronous forward-only writer
and actual files; it is not an HTTP/async-backpressure qualification.

**Replacement now:** `MuxFinish::apply_to` replaces the immediately deprecated
`apply_finish`. It validates every patch and reserves fallibly before changing
an in-memory stream. External writers use `MuxFinish::validate(emitted_bytes)`
before tail/patch effects; real I/O can still fail after partial publication.

**Required before promoting the writer:** checked timing requirements and layout
admission must distinguish timestamp preservation, explicit durations and nonzero
decode origins. Native queue/workspace admission, caller packet staging, packet
side-data preservation and continuous track reconfiguration are not qualified.
This encoded-packet trial required zero zenpixels or zenpixels-convert API/kernel
changes. That count excludes the still-open planar conversion and rendering
work; it is not a claim that the overall migration needs no pixel API changes.

Run `just av1-mux-check <fresh-artifacts>` for the focused controls, or the full
native gate. [Measured byte/layout matrix](../../benchmarks/native_mux_output_2026-10-27.csv)
and [qualification receipt](../../benchmarks/native_mux_output_2026-10-27.json)
record the pinned native implementations and source identities.

## Prepared output transitions

`epoch_route.rs` is a private native adapter for the shared
`Question::FrameRoute` / `resolve_frame_route` boundary. It uses the existing
`VideoEncoder` methods: no codec-wide reconfiguration method is needed to finish
one independently configured segment and prepare another. The actual replacement
encoder stays alive during policy; a failed decision never retries another route.

The native stream changes source epoch, dimensions, CICP, bit depth/subsampling
and timestamp clock, with old pictures queued at one/four decoder threads.
Eighteen presentations become five independently configured output segments;
the source-epoch-only change continues the first segment. Each segment has
independent FFmpeg/libdav1d exact-component readback. Native configuration checks
verify color and clock retention; independent color-signaling accuracy is not
established by the raw-sample oracle.

A fixed destination publishes a real IVF header derived from its current native
encoder configuration to an external writer buffer. No frame body is published.
A later dimension change has no offered route and refuses before acceptance;
the published header remains unchanged. Partial header failure and caught unwind
leave those external bytes intact and make the operation terminal. This exercises
the publication boundary, not an HTTP server or a complete container transaction.
Cancellation and timeout before preparation, during initial/changed-configuration
policy and at finish preserve their exact reasons and release native encoders
and unpublished compressed payloads. Callback errors keep their original cause;
decline, stale tokens, caught unwind, unknown-workspace refusal and staging
exhaustion are terminal too.

The finite route drains/drops native encoders before handing out compressed
segments. Retained packet clones keep their payload admission until the final
clone drops. This proves encoder lifetime ordering, not a whole-operation RAM
bound: input owners, native references/workspace, allocation before packet
admission, codec configuration and list bookkeeping remain outside that budget.
Header publication can itself block and has no sequential-memory qualification.
Continuous muxer reconfiguration, planar conversion and live sequential output
are not implemented by this witness.

The fixed-header regression also exercises equal-size decoded frames that change
CICP, bit depth/subsampling, or timestamp clock. Each change offers no route,
terminates before accepting the new frame, releases the old encoder and staged
packets, and preserves both the published configuration and previously returned
frame samples/interpretation. This is refusal coverage, not conversion support:
a future normalize-to-fixed-output route must retain its actual converter and
encoder together and expose the conversion effects to policy before acceptance.
An encoder's lossless setting alone cannot certify that combined route.

Run `just av1-policy-check <fresh-artifacts>`; the
[qualification receipt](../../benchmarks/native_frame_route_2026-10-27.json)
records source identities and the native and shared API gates.

## Private planar conversion bridge

`just av1-planar-check <fresh-artifacts>` separately exercises the existing
zenavif strip kernels through `PlaneView::as_strided_samples`. This uses the
codec's `_dev` export in this unpublished harness, not a new public conversion
API. A complete run of `av1-policy-check` includes these controls.

The 2026-10-26 run covers 18 native combinations (8/10/12-bit, 4:2:0/4:2:2/4:4:4,
full/limited range), 54 presentations and 162 crop cases. Whole-frame, 1/2/4-row
strips and odd-origin crops agree exactly; destination padding stays intact.
Native component samples also match FFmpeg/libdav1d. That independent check
does **not** establish RGB conversion accuracy. The bridge checks the active
word encoding before writes, keeps coded coordinates through upsampling,
and normalizes native-depth RGB into full U16 using the codec's widening rule.

Unknown subsampled siting refuses unless the caller explicitly assumes center;
actual AV1 Left/TopLeft signaling cannot be overridden by that fallback. Source
timing, presentation and interpretation remain intact without retaining the raw
picture. Unsupported color, geometry, word placement and active values refuse
before destination writes. Row scratch is fallibly allocated but not admitted
through the shared pool. Full-frame partition controls intentionally allocate
larger scratch; the one-row route needs one coded RGB row.
The private bridge validates every active source word even for a small crop;
it does not qualify proportional ROI work or conversion throughput. Native
canonical-word evidence or kernels that honor SampleEncoding directly need
evaluation before choosing the production boundary.

The original partition control retains its simple mapping-refusal enum. The
prepared witness below preserves original mapping causes and adds shared policy,
managed scratch and cancellation. Public planar conversion in zenpixels-convert,
independent RGB accuracy, unclipped float output, other color matrices/transfers,
mono/GBR, conversion reconfiguration and complete native workspace admission
remain open.

[Receipt](../../benchmarks/native_planar_bridge_2026-10-26.json): the full native
suite and strict lint pass. Resource wrapper: rc=0, 18 s, peak RSS 0.51 GiB,
minimum available 11298 MiB, peak load 0.43. This covers the whole build/test
process, not isolated codec or conversion memory.

### Prepared conversion before allocation

`prepared_conversion.rs` retains the actual native mapping, kernel selection,
destination reservation and scratch reservation through `Question::ImageConversion`.
Only a selected route initializes either buffer. `convert` then executes one
coded RGB row at a time, crops after upsampling and publishes the completed
`ImageOutput` with original timing and interpretation separate from current RGB.
The policy record accompanies success or execution failure. No fallback runs.

`just conversion-check <fresh-artifacts>` runs the shared storage/policy controls
and the native matrix. Every one of the existing 162 crop cases also executes
the prepared route and agrees exactly with whole-picture conversion. FFmpeg
still qualifies native component samples, not the resulting RGB color accuracy.
The partition control retains 1/2/4/full-row comparisons and unchanged padding.

The default preset refuses unknown subsampled siting; explicit permission accepts
the disclosed center assumption. Native signaled Left/TopLeft refuses before
policy. Integer clipping is disclosed as `preserves_extended_range: false`.
The preset refuses a strict range-preservation requirement or an unknown
workspace bound. Metadata/privacy requirements also refuse because this route
has no evidence for them. A test-only synthetic F32 offer exercises selection;
it does not claim an implemented F32 converter.

Controls exercise traced callback/backend failures with safe default formatting,
stale tokens, callback panic, cancellation before policy, after selection and
during row execution, partial storage admission and final-owner release.
A wrapped native picture confirms one mapping for the whole route; output
retains source interpretation without retaining its consumed raw picture.
Deferred-storage controls separately reserve and decline 10 GiB without creating
that pixel array. This establishes admission order, not product RSS or throughput.

This route needs **zero zenpixels API or kernel changes**: it borrows existing
native planes, calls existing zenavif kernels and adopts a strided RGB16 owner.
That is a qualification bridge, not the desired public planar implementation.
Preparation still examines every active source word before asking policy;
small-crop work is not proportional to returned pixels. Mapping, profile data,
source storage and native kernel workspace are outside this payload allowance.

### Independent RGB inspection

`just av1-planar-rgb-inspect <native-artifacts> <fresh-output>` compares the
retained RGB16 samples with FFmpeg/zimg float output, using explicit matrix,
range, transfer, siting and bilinear filtering. It records every command,
binary/input/output hash, float output and per-presentation difference. It is
an inspection, **not an accepted accuracy tolerance**. The existing exact native
sample and cross-path tests remain unchanged.

The direct oracle refused our odd subsampled extents (zscale error 1027).
The inspection now replicates only the final border into a subsampling-aligned
raw input, verifies that original codes stay unchanged, then compares only the
original rectangle. No resize or new encoded source is substituted.

Across 54 presentations (35,802 channel samples), the largest absolute
normalized difference after clipping the independent result into [0,1] is:

| Source depth | Maximum difference |
|---|---:|
| 8-bit | 0.0020102734659231958 |
| 10-bit | 0.0005358431544267017 |
| 12-bit | 0.00017896717106022364 |

The float oracle retains values from -0.9929103851318359 to 1.983231544494629
on these boundary fixtures. Integer conversion discards those excursions.
These observations support exposing clipping as a policy decision; they do not
qualify a general HDR renderer or an acceptable color-error threshold. Native
quantization, U16 bit replication and independent arithmetic remain distinct.
[Receipt](../../benchmarks/planar_rgb_inspection_2026-10-27.json),
[all per-presentation data](../../benchmarks/planar_rgb_inspection_2026-10-27.csv).
The oracle options are documented by
[FFmpeg](https://ffmpeg.org/ffmpeg-filters.html#zscale).

### Unclipped float route

The private prepared adapter also accepts an explicit RGB F32 target. It keeps
the source's sRGB transfer and BT.709 primaries: **F32 storage does not mean
linear light**. The pinned public `YcbcrToRgbF32` now replaces the original
private kernel, retaining zenavif's normalization/matrix recipe without final
integer saturation and quantization. Centered interpolation
retains exact dyadic weights in coded coordinates; odd crops keep the same phase.

The existing `ImageConversionOffer` suffices: this route reports extended-range
preservation and a destination reservation before initialization. The same
resolver and owner return the actual result. Float execution needs no managed
row scratch; source mapping, native decode allocations and metadata remain
separate costs. The integer path and its existing exact expectations are retained.

All 162 float crop cases match the corresponding full-frame samples bit-for-bit.
Original callback/error/cancellation/unwind and single-mapping controls now run
for both targets. The independent FFmpeg/zimg check covers 54 presentations with
a fixed absolute normalized RGB tolerance of **0.000001**, permitting float
arithmetic variation but not integer quantization or clipping. The largest
observed difference is 0.0000001341104507446289. A deliberately clipped float
control fails this gate for every presentation. CI runs the gate and retains
commands, binary hashes, source/output bytes and per-presentation measurements.

Run `just av1-planar-float-verify <native-artifacts> <fresh-output>` after the
native suite. [Receipt](../../benchmarks/planar_float_2026-10-27.json),
[all measurements](../../benchmarks/planar_float_2026-10-27.csv).
The initial private receipt predates shared packaging. The current route uses
public borrowed conversion and downstream OutputPlan processing, qualified below.
Non-centered siting, other matrices/transfers, HDR display mapping and complete
workspace accounting remain open.

### Converted video uses the shared image output

`ImageOutput::from_frame_pixels(frame, owner, orientation)` now adopts an actual
packed conversion result for the source's visible crop. The native AV1 matrix
runs this through the existing `Question::Image` resolver, including odd-origin
crops. The owner keeps strided normalized RGB16 and its sRGB context; source
8/10/12-bit YCbCr facts stay separately accessible through `source_frame()`.
Timing and presentation survive both adoption and later color replacement.
Adoption does not copy the converted payload or retain the consumed native
picture. Destination admission stays charged until the last output clone drops.

This is an ownership and interpretation boundary, not a new conversion engine.
The private strip bridge still controls unknown-siting assumptions and clips to
integer RGB; independent RGB accuracy, float excursions and general HDR remain
unqualified. The shared constructor does not verify conversion math, honor a
source render-size instruction automatically, or authorize metadata transfer.
Packed constructors still refuse native presentation claims without explicit
source-frame provenance; later transforms retain it by cloning interpretation.
No zenpixels API or kernel change was required for this adoption boundary;
`bytemuck`, already in the standalone dependency graph, is now a direct test
dependency for borrowing the typed admitted RGB allocation as bytes.

The [qualification receipt](../../benchmarks/converted_video_output_2026-10-27.json)
records 162 publications across 54 native presentations and 18 configurations,
the final AV1/still-image checks and the zero zenpixels API delta for this step.

## Native publication and re-encode

This standalone workspace runs the shared policy resolver over real
`Av1VideoDecoder` presentations, then sends the approved native owners to
`Av1VideoEncoder`. FFmpeg/libdav1d independently reads the resulting IVF files.
Expected samples come from strided source planes; no conversion is used.

The selected-image output trial compiles the same
[`ImageOutput`](../spatial-validation/src/image_output.rs) used by the spatial
harness. `just av1-output-check <fresh-artifacts>` exercises three frames in
each of the 15 depth/layout configurations with native encode/decode and
required FFmpeg readback. It preserves native plane addresses, code depth,
roles, color/siting/crop/epoch interpretation and presentation timing, survives
decoder reset/drop, and releases the picture with the final output owner.
Its packed-color convenience refuses planar code samples. The trial is private;
it does not establish a display conversion or a complete memory bound.

The trial now exposes an immutable `interpretation()` snapshot instead of mutable
output fields. The same native address, exact-sample, ownership and FFmpeg checks
passed after this change: run-heavy rc=0, 9 s, peak RSS 0.54 GiB, minimum available
13334 MiB, peak load 0.46. Log: `~/tmp/zencodec-interpretation-av1-2026-10-26.log`.

The output gate passed on 2026-10-26: run-heavy rc=0, 9 s, peak RSS 0.54 GiB,
minimum available 13206 MiB, peak load 0.36. Log:
`~/tmp/zencodec-common-output-video1-2026-10-26.log`. This measures the build/test
process. Native picture-address and ownership assertions establish the storage
handoff, rather than a process-memory estimate.

From an export outside the zen workspace's path overrides:

```sh
just av1-policy-setup
just av1-policy-check /path/to/fresh/persistent/artifacts
```

FFmpeg with libdav1d and the `timeout` command are required. Each oracle process
has a 30-second deadline. Existing artifacts are never overwritten: choose a
fresh directory for each run. The Cargo manifest pins the native PRs and patches
all shared zencodec types to this tree. `dependencies.lock` records resolution.

CI selects Ubuntu's archive mirror in both direct entries and the runner's
`apt-mirrors.txt`, keeping the signed FFmpeg package, with separate two-minute
update and three-minute installation deadlines. The initial
Azure mirror attempt exhausted the job's 15-minute deadline before tests began
(run 40237893129). Installation failure fails qualification; it never skips the
independent decoder checks. Run 40241228768 exposed the mirror-list indirection:
the earlier direct-entry edit left downloads on Azure and the installation
deadline correctly failed with status 124. Both configuration paths are now
rewritten; HTTP and HTTPS transfers also have per-transfer deadlines.

The matrix covers 8/10/12-bit mono, YUV 4:2:0, 4:2:2, 4:4:4 and identity-matrix
GBR, each at 17x13 with padded input strides, in both ordinary pooled and
bounded native-plane modes. Both modes independently re-encode and pass exact
FFmpeg/libdav1d readback. Other tests check queued epoch
changes at one/four native threads, multiple presentations per packet, repeated
packet ordinals, exact picture ownership, native and callback error traces,
refusal, stale choices, cancellation and caught callback panics.

The executable guide now configures `max_picture_bytes` before opening the
native session. Refusal precedes the publication callback, drops the native
decoder and retains the original `At<rav1d_safe::Error>` through the policy
adapter. Accepted native frames keep their storage allowance after decoder
drop. The native setting bounds requested plane bytes, including padding and
alignment; it excludes input, metadata, codec workspace and allocator overhead.
The native error is still `Error::Other` with a trace; structured refusal facts
and a shared resource category remain open.

Publication policy still runs after native reconstruction. This does not
establish a complete decoder RAM bound, pre-reconstruction metadata suspension,
container privacy, planar conversion, all AV1 profiles, encoder output scheduling
or performance.
The native decoder's broader conformance and film-grain tests live in
[rav1d-safe PR 537](https://github.com/imazen/rav1d-safe/pull/537); the session
adapter is [zenavif PR 74](https://github.com/imazen/zenavif/pull/74).

Frame/IVF fixture helpers are adapted from zenavif `ea055f7d`'s
`tests/media_session.rs` and `examples/media_ivf.rs`. They remain test code.

The native budget integration passed on 2026-10-25 with zenavif `feadc4c9`
and rav1d-safe `b19aec65`. Its 15 depth/layout configurations pass in both
allocation modes, alongside the refusal/trace/lifetime tests and strict
Clippy. [Qualification receipt](../../benchmarks/native_picture_policy_2026-10-25.txt):
run-heavy rc=0, 29 s, peak RSS 1.13 GiB, minimum available 15274 MiB,
peak load 1.68. This is build/test-process usage, not a decoder memory or speed
measurement.

### Process native float through OutputPlan

`float_processing.rs` executes native planar → unclipped sRGB F32 → linear RGB
F32 → sRGB through the existing `OutputPlan`. It supplies explicit descriptors,
current color context, named target and unclipped conversion options. The 54
native presentations and 162 crops pass exact full-image/1/2/4-row agreement.
Strided padding stays untouched. `replace_packed_color` adopts the processed
owner without copying pixels and retains source facts, geometry and video timing;
current output signaling becomes linear RGB while the source remains unchanged.

The misleading `ConvertOptions::forbid_lossy()` preset is now deprecated.
Its behavior-preserving replacement `retain_channels()` still enables clipping:
the tested conversion explicitly selects `with_clip_out_of_gamut(false)`.
The numerical contract remains open:
linear-srgb 0.6.12 uses adjusted continuous constants. Its maximum error against
that definition is 0.0000011395496919419656, but disagreement with the
[IEC/CSS extended sRGB definition](https://www.w3.org/TR/css-color-4/#color-conversion-code)
is 0.000056949194831901195. The original strict IEC probe failed and is recorded;
these are separate definitions, not an accepted IEC accuracy threshold.
Roundtrip error is at most 0.0000010728836059570312. Returning to the same color
space is not bit-exact preservation, and F32 storage alone promises neither.

The native tests gate agreement with the selected continuous curve and roundtrip
at absolute 0.00001. Batch equivalence remains byte-exact. All per-presentation
[measurements](../../benchmarks/float_processing_2026-10-27.csv) and the
[receipt](../../benchmarks/float_processing_2026-10-27.json) are retained. This
uses the reviewed OutputPlan revision without changing zenpixels source or adding
a package to the graph. Broader planar coverage, required range behavior and the numerical-model contract,
HDR rendering and workspace bounds remain open.

### Shared borrowed planar conversion

The pinned zenpixels review now supplies `sample::CodePlane`, explicit chroma
sampling/location and `ycbcr::YcbcrToRgbF32`. The native adapter maps its existing
borrowed words into these views, constructs one plan before policy, then converts
into admitted strided rows. The source mapping remains held; no source payload is
copied. The former private float kernel has been removed.

The shared plan accepts valid 8/10/12-bit code placements in U8/U16, both ranges,
centered 4:2:0/4:2:2 or 4:4:4, and BT.709-matrix/sRGB interpretation. Its unit
controls also exercise shifted words and nonzero code padding. The native adapter
retains its narrower declared-source checks and siting decision. Unsupported
color, encoding, geometry or location returns a traced error containing the
actual facts; invalid output geometry or descriptor leaves destination unchanged.

All 54 native float outputs are byte-identical to the earlier private baseline.
The 162 crop cases, downstream OutputPlan, cancellation, callback failures and
ownership checks pass against the committed dependency. Independent zimg
agreement retains the existing 1e-6 gate. The pixel workspace, strict lint,
documentation, API snapshots and no-default-feature WASM checks pass.

This adds six public types and nine methods across the two pixel crates, with
zero new features or packages. The converter prepares one shared color context;
execution allocates no payload or scratch. Native workspace and whole-operation
memory remain outside this statement. The adapter still translates zencodec's
earlier plane and sample descriptions; consolidating those types remains open.
Other matrices/transfers, non-centered siting, integer/mono/GBR output, HDR,
strict-preset cleanup and coupled streaming reconfiguration also remain open.

[Receipt](../../benchmarks/public_ycbcr_2026-10-27.json),
[shared contract](https://github.com/imazen/zenpixels/blob/6250da2fc073150d69cbc6746ffa6cc6724b743f/docs/borrowed-ycbcr-contract.md).

### Required unclipped Gamma22 execution

The executing native regression first failed on the previous pixel revision:
linear `-0.46646955609321594` became encoded zero despite
`with_clip_out_of_gamut(false)`. The fixed internal transfer steps preserve
negative and above-white values. They reuse the existing in-range SIMD results
and reflect the existing Adobe exponent `563/256`; this is not a claim of
literal gamma-2.2 equivalence or a change to the default clipped route.

All 54 native presentations now run linear → Gamma22 → linear. Per-presentation
`processing-*.csv` adds gamma and roundtrip errors and extended-sample counts.
`gamma-{linear,encoded,restored}-*.raw` retains every little-endian RGB F32 sample.
`just av1-gamma-verify <native-artifacts> <report.json>` independently reads those
files with Python's double-precision power function. It requires every expected
trace, enforces a fixed `2e-5 * max(1, abs(reference))` bound, and checks that
extended values do not collapse. Negative controls reject clipping, changed
roundtrip samples and missing files. Reports include trace hashes.

No public API, package or feature was added for this fix. General transfer/backend
range guarantees, finite-domain behavior and numerical-model choice remain open.

## Coupled fixed-output conversion

`coupled_route.rs` retains a shared `YcbcrToRgbF32` plan, the source mapping,
managed plane/row reservations and the native encoder through one
`Question::FrameRoute` decision. `FrameConversionOffer` records the encoder's
actual input interpretation/timing separately from source facts. Conversion
runs after acceptance of the route, before accepting the frame into the encoder.
Target drift or conversion failure terminates the operation and retains the
selected decision; no alternative is retried.

Three real decoded inputs (8-bit 4:2:0 full range, 10-bit 4:2:2 limited range,
12-bit 4:4:4 full range) become one fixed 8-bit GBR AV1 sequence. The output
configuration is published after the first frame and stays unchanged. The
converter applies the existing BT.709/sRGB matrix route followed by explicitly
clamped, nearest-integer RGB8 quantization; it reports neither sample nor extended
range preservation. The native encoder is lossless for those converted codes.
FFmpeg/libdav1d readback compares every emitted component against the actual
converted frames. This checks encoding integration, not an independent proof of
conversion accuracy; the shared float converter has its own independent matrix.

`just av1-coupled-check <fresh-artifacts>` exercises the native witness and strict
lint. Exact-sample callback and required-range preset refusals occur before
conversion execution or destination initialization. Shared conformance covers
missing evidence, unqualified encoders, allowed/disallowed/invalid siting
assumptions and retained selected facts. No new dependency or zenpixels API is
needed. Reservations cover three strided output planes and one RGB float row;
input owners, native workspace/references and bookkeeping are outside that bound.
The test retains converted frames for readback; it does not establish a streaming
memory bound. HDR, resizing, retiming, continuous muxer changes and packaged
transcode convenience remain unqualified.

## Explicit retiming through a prepared frame route

The native `coupled_native_retiming_keeps_source_provenance_and_fixed_output_clock`
case decodes three AV1 frames, then uses `Frame::represent` to share their pictures
with explicitly supplied output timing. The callback sees unchanged source facts
and previous-source timing separately from `FrameConversionOffer::encoder_input`.
A fixed encoder publishes its 1/90000 clock after the first frame. PTS values
9000, 10800 and 15300 and durations 1800, 4500 and 2700 survive in native output
packets. FFmpeg/libdav1d independently verifies exact decoded component samples.
A wrong promised timestamp, like a wrong interpretation, fails terminally before
sending the converted frame, releases native/staging owners and retains the
selected decision record.

This qualifies explicit one-input/one-output retiming at the encoder boundary.
It does not qualify a frame-rate converter (drop/duplicate/interpolation), audio
synchronization, negative timestamps or preservation of durations in IVF. The
independent IVF readback verifies samples; packet timing assertions inspect the
native encoder directly. Coupling this route to the exact MP4 destination gate
remains required before claiming end-to-end container timing. No public API,
zenpixels change or pixel copy was required for this retiming mechanism.

Reproduce with `just av1-coupled-check <fresh-artifacts>`. On 2026-10-27 the
five coupled tests and strict all-target lint passed; run-heavy reported
`rc=0 6s | peak-RSS 0.57GiB | min-avail 11253MiB | peak-load 0.65`.
Artifacts: `/mnt/v/output/zencodec/platform-regions-20261026-run/retiming-20261027-2`
on r7900x. Full log: `/home/lilith/tmp/zencodec-retiming-drift-20261027.log`.

## Retimed encoder output through an exact MP4 destination

`mux_native_coupled_retimed_encoder_packets_preserve_exact_mp4_timing` connects
native AV1 decode, the prepared retiming route, native AV1 encode and the exact
MP4 output adapter. The actual muxer is prepared before the first frame policy decision and
retained through header publication and finish. Later frame decisions observe committed output.
The three native packets go directly to that muxer without timestamp rewriting.
FFprobe 8.0.1 independently reads the 1/90000 clock and PTS/duration pairs
9000/1800, 10800/4500 and 15300/2700. FFmpeg/libdav1d verifies exact samples.
The forward destination interrupts its first write and accepts three bytes per
subsequent write; the output needs no finish patches.

The paired partial-header test lets 13 bytes escape, then returns BrokenPipe.
That error retains the mux drain stage and confirmed byte count through the
encoder operation. The operation becomes terminal, releases native/staging
owners, and refuses both another frame and another publication attempt.

This is finite compressed staging: native encoders are dropped before packet
writes. It does not qualify live sequential backpressure, a whole-operation RAM
bound, seekable-disk retiming, audio, frame-rate conversion or in-band changes.
The private preparation path now admits the actual muxer and destination before
policy, conversion and initial encoder input. A public prepared-route facade and
destination evidence in policy diagnostics remain API-design gates. These tests do not close issue 172.
No additional public API or zenpixels change was required for this composition.

Run `just av1-coupled-mux-check <fresh-artifacts>` with `AV1_TIMING_FFPROBE`
pointing to the pinned inspector. Both tests and strict lint passed on
2026-10-27: `rc=0 2s | peak-RSS 0.57GiB | min-avail 11829MiB | peak-load 0.14`.
Artifacts on r7900x:
`/mnt/v/output/zencodec/platform-regions-20261026-run/coupled-mp4-20261027-3`.
Full log: `/home/lilith/tmp/zencodec-coupled-mp4-terminal-20261027.log`.

## Destination preparation before encoder input

The private `PreparedOutput<M, D>` owns the actual begun muxer and writer without
draining its header. `publish(self, stop)` consumes that object and returns the
active output driver. Existing immediate-publication constructors use the same
implementation. Dropping a prepared output performs no writer effects.

`Candidate::push_route_with_destination` prepares this destination from the
retained native encoder's real configuration before calling the shared resolver.
It retains the destination through policy and converter execution, and returns
it only after accepting the selected frame. The retiming test checks that exact
preparation ran once before policy; later frames use the same active muxer.
The native IVF default exact-timing refusal occurs before policy or conversion,
with zero emitted bytes. A policy rejection after successful MP4 preparation
also leaves zero bytes and releases native/staging owners. Publication and its
partial-write failure remain a separate, consuming operation.

This is an adapter prototype, not a new published interface. Native preparation
can allocate; no whole-operation workspace bound is claimed. The generic private
preparation closure must obey the no-publication contract, and the frame policy
record still lacks destination-specific facts. The next public contract must
carry qualified destination facts and ownership without asking callers to
reconstruct or coordinate the objects. No zenpixels changes were needed.

Full native regression gate on 2026-10-27: all 40 cases, strict lint and the
independent gamma verifier passed. This includes existing cancellation, unwind,
partial publication, three-container disk/forward output, conversion and
frame-transition controls; it is not coverage of every codec/configuration.
Command: `just av1-policy-check <fresh-artifacts>` with the pinned timing inspector.
Run-heavy: `rc=0 37s | peak-RSS 0.05GiB | min-avail 11739MiB | peak-load 0.57`.
Artifacts on r7900x:
`/mnt/v/output/zencodec/platform-regions-20261026-run/prepared-mp4-full-20261027`.
Full log: `/home/lilith/tmp/zencodec-prepared-mp4-full-20261027.log`.

## Shared public output driver

The native suite now imports `zencodec_media::output` with the `std` feature.
The private driver copy has been removed. `Output::new` and
`PreparedOutput::new` require exact timing; the previous ordinary-timing path is
explicitly named `Output::with_container_timing`. `Forward` and `SeekBack` wrap
actual writers; the sealed destination interface prevents independent claims of
seek capability. `SeekBack` captures the current position and scopes patches to
that output, preserving existing prefixes. `PreparedOutput::publish` consumes
the retained muxer/writer. `Output::stats` exposes progress without exposing
mutable internal state.

All 40 native cases and strict lint passed against the shared implementation,
including the three-container forward/disk matrix, exact MP4 variable timing,
retiming without packet rewrites, unsupported exact timing, policy rejection,
partial publication, terminal cancellation and invalid final patches. The
independent gamma verifier also passed. Full log:
`/home/lilith/tmp/zencodec-public-output-fixed-20261027.log`; artifacts on r7900x:
`/mnt/v/output/zencodec/platform-regions-20261026-run/public-output-20261027-2`.
Run-heavy: `rc=0 44s | peak-RSS 0.57GiB | min-avail 11493MiB | peak-load 0.86`.

The workspace all-feature tests (including the new output doctest), default
no_std check, lint and existing API snapshot generator passed:
`rc=0 15s | peak-RSS 0.31GiB | min-avail 11165MiB | peak-load 0.89`.
The snapshot generator does not inventory the unpublished media crate; the site
extracts the driver signatures directly from source. No dependency declaration
or zenpixels API was added; media gains an opt-in `std` feature. The no_std
sans-I/O traits remain available without it.

This synchronous adapter does not qualify a live async encoder-output pipeline
or a whole-operation allocation bound. It preflights patches but cannot roll
back a writer that fails after partial overwrite. Native encoder/converter route
ownership and destination-specific policy diagnostics remain separate work.

### Explicit disk staging

The public owned operation also accepts `Staging::disk` through `with_staging`.
Native AV1/MP4 tests compare memory and disk output byte-for-byte and decode the
result independently. They cover callback refusal, exhausted disk/replay budgets,
read-only scratch, encoder release before final writes, and late spool failure
after publication with the original I/O cause and exact confirmed append count.
Packet unit tests preserve additions, clocks, epochs, ordinals and offsets,
verify scratch-prefix preservation and release shared replay charges.

The scratch file must be exclusively controlled by the caller; permissions and
deletion are application policy. Fault-injection aliases in tests do not qualify
external mutation. Spool writes can wait while the encoder retains raw state.
Only final-destination writes wait for encoder release. No throughput, total
workspace-memory bound or live sequential guarantee follows from these tests.
