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
