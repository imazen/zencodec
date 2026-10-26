# Native spatial contract experiments

## Image selection candidate

The native TIFF loop now also exercises a separate candidate in
[`src/image_items.rs`](src/image_items.rs) and
[`src/tiff_items.rs`](src/tiff_items.rs). It discovers main directories lazily,
follows arbitrary SubIFDs, and decodes the selected directory through the same
native region path. These types remain private to this experiment.

The important contract is an operation-bound image identity, distinct from its
relationship and requested composition. A SubIFD is not necessarily a reduced
image. Two parents can point to the same image; the result is a graph, not a
tree. A page number is not an animation timestamp. Foreign identities/cursors
and an unsupported uncomposited-layer requirement refuse before source I/O.

The native fixture matrix covers nested non-reduced SubIFDs and shared children
in classic/BigTIFF and chunky/planar RGB8, with exact selected-region parity
against the original rasters. Malformed cycles and discovery exhaustion return
explicit errors. Existing reduced-level selection tests remain unchanged.
The gate passed on 2026-10-25: run-heavy rc=0, 5 s, peak RSS 0.41 GiB, minimum
available 15881 MiB, peak load 0.15 (test process, not a decoder memory bound;
log `~/tmp/zencodec-spatial-items1-2026-10-25.log`).

This does not qualify JXL layers, non-image metadata directories, all auxiliary
roles, or remote revision binding. Discovery currently materializes one bounded
SubIFD tag and charges yielded relationships; it does not eagerly load the
whole graph. The native decoder owns TIFF parsing and compression throughout.

## JXL presentations and layers

### Common consumer loop

The private [`Collection`](src/collection.rs) trait now runs the same
`first_image` consumer against `TiffCollection` and `JxlItems`: discover lazily,
retain the returned identity, decode that identity. `first_child` selects a
native TIFF SubIFD; the JXL adapter returns `UnsupportedChildren` because it
cannot establish that query. Unsupported discovery is not an empty collection.
Repeated JXL discovery returns the same IDs even after the native scanner has
advanced. Cross-operation cursors refuse.

The common boundary returns `whereat::At<CollectionError>` and preserves the
original native/source cause. A source replacement injected after TIFF open is
still inspectable as the original `SourceFault::VersionChanged`, and a required
unsupported view refuses before any source read. The guide displays the exact
executed `first_image` function with its enclosing imports and return type.

The later loop below replaces the associated native sample type with a shared
mapped output and moves region requests to each decode. Public promotion still
requires complete interpretation, immutable admission and discovery allowances
whose scope is explicit. Network revision binding and pixel/privacy policy remain
separate gates. The cancellation loop below qualifies mechanical stop checks;
it does not establish a hard latency bound for native calls.

The full combined gate passed after adding the common path: run-heavy rc=0,
104 s, peak RSS 0.37 GiB, minimum available 15919 MiB, peak load 0.92; log
`~/tmp/zencodec-common-selection2-2026-10-25.log`. After extracting the displayed
function, `just spatial-consumer-check` reruns its native cases and strict
Clippy without repeating the full matrix. These are process receipts, not
native resource bounds.

### Repeated browsing has one operation allowance

A viewport can revisit an image, create another cursor or ask an unsupported
child query. Native TIFF currently counts returned relationships while JXL
limits its cached presentations. Those are different native limits, so neither
is presented as a universal discovery-work bound.

The private [`Discovery<C>` experiment](src/discovery.rs) wraps either adapter
without changing `Collection` or its consumer loop. It charges each `images`,
`children` and `next` attempt before delegation. Cached results, exhaustion and
errors consume attempts; opening another cursor and retrying cannot reset or
refund them. A refusal retains the method, allowance and usage through the
existing traced error. Already returned identities remain decodable after
exhausting discovery. `first_image` uses two attempts; `first_child` uses four.

`just spatial-discovery-check` executes the same repeated-cursor/exhaustion
consumer on real TIFF and JXL. Further cases check exact SubIFD samples in
classic/BigTIFF, typed unsupported-child and injected source failures, zero
allowance and refusal before further native polling or source I/O. The gate and
strict lint pass: `run-heavy: done rc=0 8s | peak-RSS 0.54GiB | min-avail 12793MiB |
peak-load 0.25` (test process). [Receipt](../../benchmarks/spatial_discovery_2026-10-26.json).

This is a bound on calls, not on parser bytes, retained graph memory or CPU
inside one call. It excludes opening the native decoder. Native limits and
source admission remain required. Keep accounting in an operation/driver layer;
there is no evidence here for another public codec trait, a callback per item,
or requiring every caller to coordinate capability flags. Promotion still needs
native work accounting and integration with the existing operation context.

### Source identity is established by the transport

The [private HTTP source](src/http_source.rs) and [response harness](http_snapshot.py)
reuse IMI's HTTP range study at `f7652c97119c936b53d739098e2eab789a42592a`.
They add typed retained causes and deliberate protocol faults around native TIFF
selection. No HTTP dependency or validator type enters the public codec traits.

The source pins a strong ETag and representation length on its first successful
response. Every later request sends that ETag in `If-Match` and asks for identity
encoding. It requires exact 206 coverage, length and the same validator before
accepting the body. Ignored ranges, redirects and encoding changes refuse.
Errors permanently fail that source instance; retries preserve the original
cause and cannot silently adopt a replacement. This uses
[RFC 9110 conditional range semantics](https://www.rfc-editor.org/rfc/rfc9110.html#section-13.1.1).
`If-Range` permits replacement with a full response, which is inappropriate for
continuing a decode already tied to the original bytes.

`just spatial-http-check <fresh-artifacts> 3568` explicitly enables the private
`http` feature and runs real local HTTP/1.1. Classic/BigTIFF and chunky/planar
SubIFDs match original rasters through direct and 4096-byte buffered reads.
Replacement immediately after discovery refuses selected decode with a retained
412 cause; retry performs no more HTTP. Fourteen changed/malformed response
cases refuse through native open. Initial validator and range-admission cases
also run. The server independently verifies every request after opening is
conditional. The [receipt](../../benchmarks/http_snapshot_2026-10-26.json)
records 32 URL cases, 2186 requests and the complete external trace pointer.

The same trace makes the buffering tradeoff visible without timing a busy host:

| Selected SubIFD | Direct requests / body bytes | 4096-byte buffer requests / body bytes |
|---|---:|---:|
| Classic, chunky | 236 / 2977 | 13 / 47877 |
| Classic, planar | 340 / 3041 | 21 / 80773 |
| BigTIFF, chunky | 239 / 3353 | 12 / 44257 |
| BigTIFF, planar | 343 / 3417 | 20 / 77153 |

These totals include opening and repeated reads. Buffering reduces requests and
fetches more bytes in these fixtures; this establishes neither a speedup nor an
optimal window. Native admission and transport request/byte budgets must remain
separate. The full HTTP gate and strict lint report `run-heavy: done rc=0 9s |
peak-RSS 0.58GiB | min-avail 12907MiB | peak-load 0.28` (test process).

This qualifies one object's revision on a cooperating HTTP server. It does not
prove multipart-manifest coherence, CDN/browser/TLS behavior, a disk-file snapshot
or dishonest unchanged ETags. Existing verified bytes from the pinned revision
need not become invalid merely because a newer revision exists. File sources
still require a stable-byte ownership/locking contract. A failed read may have
written partial bytes into caller scratch; no selected output may publish them.

### Cancellation belongs to the opened operation

`TiffCollection::open_with_stop` and `JxlItems::open_with_stop` accept the existing
`zencodec::StopToken` once. The common `Collection` methods and `first_image`
consumer need no additional cancellation argument or capability coordination.
Default opening remains available when the caller does not need cancellation.

The TIFF adapter checks before opening, around every source read/seek, during
known graph traversal, between native coding units and before returning output.
The JXL adapter passes the same token into native options for discovery and
selected-frame replay, with checks before publishing IDs or mapped output.
Cached presentations and exhausted cursors still check cancellation: neither
an item nor successful exhaustion substitutes for a cancelled request.

`just spatial-cancellation-check` injects both `Cancelled` and `TimedOut` at every stop poll observed
in the successful classic-TIFF and BigTIFF planar SubIFD operations, and one
native JXL presentation operation. Each cancelled run must return an error and
no output. Separate controls cover cancellation before any source I/O, known
graph traversal without I/O, cached/exhausted JXL cursors and a failed native
scan that must not appear exhausted. Successful controls compare real samples.

These tests do not measure cancellation latency or promise interruption of a
blocking source call. TIFF compute checks remain at coding-unit seams; JXL uses
its native cooperative checks. Source deadlines, native workspace bounds and
broader codec configurations remain separate obligations. Stop reasons produced
by the adapter use the existing `CodecError` category; native causes remain
inspectable through the traced collection error. Native JXL now preserves both reasons through `Error::stop_reason()` while keeping
its existing coarse `ErrorClass::Cancelled`. The consumer reproduced the deadline
collapse before pinning the [issue 70 fix](https://github.com/imazen/zenjxl-decoder/issues/70).
The exact-reason gate passed: `run-heavy: done rc=0 67s | peak-RSS 0.59GiB |
min-avail 12535MiB | peak-load 0.83`. This is test-process usage, not a native
workspace or latency bound. The [receipt](../../benchmarks/spatial_stop_reasons_2026-10-26.json)
records the pinned implementation and before/after logs.

The full native gate passed with these adapter changes: `run-heavy: done rc=0
154s | peak-RSS 0.23GiB | min-avail 13419MiB | peak-load 1.10`. This is test-process
usage, not a decoder memory bound. The additional graph/failed-scan controls run
in the targeted cancellation gate. Evidence is recorded in
[`spatial_cancellation_2026-10-26.json`](../../benchmarks/spatial_cancellation_2026-10-26.json).

### Region requests belong to the selected image

The [platform review](../../docs/platform-review-2026-10-25.md) exercised WIC-backed
Windows selection and independent OpenImageIO/TIFF readback. Its immediate API
consequence is `Collection::decode(image, &ImageRequest)`: each request chooses
full pixels or a stored region. TIFF opening no longer fixes a shared rectangle.
Returned `StoredRaster::geometry` preserves the selected canvas dimensions and
region origin; orientation remains unapplied. A region cannot silently clip to
fit a smaller page or child. Invalid extents retain request/dimension context.

The common native tests alternate full/interior/edge requests on differently
sized pages and shared SubIFDs, select out of order, and recover after rejected
empty/overflowing/out-of-bounds requests. JXL currently refuses required regions
and still permits a subsequent explicit full-image decode. This does not qualify
JXL region implementation or efficient partial reconstruction.

The full native matrix and strict Clippy passed after this change: 42/42 tests;
`run-heavy: done rc=0 101s | peak-RSS 0.22GiB | min-avail 13083MiB |
peak-load 1.35`. The log is `~/tmp/zencodec-platform-regions-full2-2026-10-26.log`;
this measures the test process, not a native workspace bound.

### Native TIFF pixels through the shared owner

The TIFF adapter first creates `StoredRaster`: a `zenpixels::PixelBuffer`
and unapplied orientation, then moves it into the common `ImageOutput`.
[`tiff_pixels.rs`](src/tiff_pixels.rs) adopts the
native region allocation with its explicit dimensions and byte stride. It
does not normalize signed values, narrow wide integers or canonicalize NaNs.
The integration exposed a missing external constructor for `PixelBufferParts`;
[zenpixels PR 93](https://github.com/imazen/zenpixels/pull/93) adds it alongside
the signed/wide storage formats. Both crates are pinned at
`9dad0cb609bbc07ed329696902ed5a1a3d3951b6` in this standalone harness.

`just spatial-samples-check` runs all 22 native strip sample/layout cases
through checked adoption and identity row conversion, checking the original
allocation pointer, all active sample bytes, row padding and unapplied
orientation. It covers classic/BigTIFF, the existing four compression modes,
predictors and edge/interior/full-image regions. Signed types are qualified
in gray; unsigned and floating types also cover RGB/RGBA. This is not a claim
of GrayAlpha, half-float or arbitrary-band native coverage.

The selected directory supplies its own ICC bytes and alpha declaration.
Unknown transfer and primaries remain unknown. Missing, unspecified or
unsupported alpha association refuses with `InterpretationError::Alpha`;
associated and straight declarations survive unchanged. The per-directory
test decodes two differently tagged pages out of order and repeats a page,
checking its profile and orientation every time. Malformed or over-budget
ICC fields remain native errors rather than becoming absent metadata.
Opaque ICC carriage does not establish profile validity or a PII policy.

The targeted gate passed with run-heavy rc=0, 5 s, peak RSS 0.45 GiB,
minimum available 12273 MiB, peak load 0.92; log
`~/tmp/zencodec-native-samples3-2026-10-25.log`. This is a test-process receipt,
not a decoder memory bound. The shared collection is still private. Oriented
display geometry, unknown-color policy, full auxiliary rendering details and
the distinction between raw sample carriage and approved display conversion
remain promotion gates. Media's unsigned component-code encoding remains
distinct from signed/floating image samples.

### One mapped output for TIFF, JXL and video

[`ImageOutput`](../../src/image.rs) replaces the collection's associated
native sample type. It owns storage through a reference-counted handle and
returns borrowed planes through a mapping guard. `with_pixels` is the small
packed-color convenience; planar component codes require an explicit conversion
and refuse that convenience. The video bridge retains the native mapping guard,
sample encoding, roles, geometry/crop, color, epoch, timing and presentation
facts. It makes no pixel copy. Orientation remains unknown where the video
interface does not establish one.

TIFF moves its existing allocation into the owner. JXL explicitly requests
interleaved alpha when present, preserves all requested extra planes separately,
and attaches the profile of the **actual output** after format selection.
Opaque ICC and unknown transfer/primaries remain distinguishable. This is not
a privacy approval or proof of a display conversion. A source alpha plane must
not silently disappear from the ordinary packed-color view.

Auxiliary samples carry their declared role, name, source precision and source
dimensional shift without pretending to be grayscale. Emitted plane dimensions
are recorded independently of source subsampling. The native accessors in
[zenjxl-decoder PR 68](https://github.com/imazen/zenjxl-decoder/pull/68) supply
integer/float precision, exponent bits, spot-color linear RGBA and CFA indices.
The eight
native fixtures cover straight/associated alpha, extra channels, spot color,
gray-alpha, upsampled alpha, rotated geometry and an embedded grayscale ICC.
The generated native-encoder fixtures extend role coverage to 7/7 named roles:
alpha, depth, spot color, selection mask, black, CFA and thermal. Unknown/reserved
roles still need native fixtures. The generated subsampling matrix now checks
all seven named roles at shifts 1/2/3 on an odd 17x13 canvas, retaining the source
shift separately from full-size emitted planes. Independent libjxl checks orientation, role
codes, source bit depth and source
dimensional shift; pixel checks compare native sequential color/alpha/extra
samples with the selected path. A separate selected-animation test checks exact
PTS and duration ticks against libjxl, including variable durations and reverse
selection. This is not independent JXL pixel parity.

The initial trial exposed the existing whole-pixel stride restriction in
`PixelSlice`: a sample-aligned padded RGB-f32 row can still be rejected. The
adapter controls its allocations and now requests a whole pixel of padding.
The native 13-byte-padding regression remains in place. Arbitrary external byte
strides remain [zenpixels issue 91](https://github.com/imazen/zenpixels/issues/91);
the constructor's established rejection tests have not been changed.

[`image_output_tests.rs`](../av1-policy-validation/src/image_output_tests.rs)
compiles the **same** output implementation against native AV1. It checks 8/10/12
bits with mono, 4:2:0, 4:2:2, 4:4:4 and planar RGB, three frames each. It verifies
native plane addresses, exact active samples, interpretation/timing, survival
after decoder reset/drop and release after the last output owner drops.
Required FFmpeg readback checks the native encoded packets independently.
`just av1-output-check <fresh-artifacts>` passed with run-heavy rc=0, 9 s,
peak RSS 0.54 GiB, minimum available 13206 MiB and peak load 0.36; these are
test-process measurements.

`ImageOutput::interpretation()` now borrows a frozen snapshot. Its checked packed
constructor rejects sample/region size disagreement, empty or out-of-canvas
regions, and incompatible component interpretation. A cloned interpretation is
an editable detached value; it cannot relabel an existing output. Native TIFF
adoption, JXL owner-drop tests and the same AV1 matrix exercise this boundary.

JXL stores the native header, embedded source profile, selected frame index/name
and last-frame marker in `source_details`, using the existing `Extensions` type.
This retains original integer/float precision, exponent bits, tone-mapping facts,
intrinsic/preview sizes and animation loop/timecode-presence declarations. The
actual output profile remains attached to the pixels. These source details are
neither output color instructions nor permission to copy metadata to a derivative;
names and opaque profiles still require retention policy.

Independent libjxl metadata checks cover HLG, PQ, a PQ gradient, a 32-bit float
source with 8 exponent bits, an embedded ICC and a preview. Animation checks
retain exact tick rates, loop count and timecode-presence declarations. The
generated fixtures additionally exercise a 13x9 intrinsic size over an 8x6
canvas, intensity target 1000, minimum luminance 0.125, relative-to-display
behavior and `linear_below = 0.25`. The mapped consumer retains these values
after decoder/input drop and output cloning. Half/float alpha retains source
16/5 or 32/8 precision while the emitted storage is F32; exactly representable
source values also check these two fixtures independently of native sequential
decoding. Selected-frame timecodes preserve absent versus present zero, and
reverse selection keeps timecodes separate from rational PTS and duration.
libjxl independently verifies the generated source headers and frame timecodes.
The generator is native `jxl-encoder` at
`0255e5a9f61b7270cb3ed3f7b85fec2283deec55`; it is a standalone test dependency.
Generated binaries are retained under the explicit `SPATIAL_GENERATED_ARTIFACTS`
directory, created without overwriting existing results, and never committed.
The native surface remains under review in
[PR 68](https://github.com/imazen/zenjxl-decoder/pull/68), tracked by
[issue 67](https://github.com/imazen/zenjxl-decoder/issues/67).
The tone-mapping source fields are kept verbatim; no unqualified conversion to
`SourceColor::diffuse_white` is made. The
[libjxl metadata contract](https://libjxl.readthedocs.io/en/latest/api_metadata.html)
distinguishes source precision, intensity bounds and display-tone-map behavior.

The output now lives in `zencodec::image`, using the core's native frame mapping
contracts. The standalone harness imports that implementation directly rather
than keeping a parallel trial. Native frame mappings check region, plane count,
role-derived dimensions and sample encoding before exposing each plane. Invalid
regions retain their full extent/canvas; mapping failures retain typed native
causes through `At<OutputError>`. Packed pixels remain in `zenpixels`.

Collection discovery and render/transform policy admission still need
qualification. Auxiliary rendering, whole-container
metadata/privacy, native memory bounds, HTTP revision binding and raw JXL layers
remain open. The output is implemented on the API review PR; the collection is
still private and neither surface has been released.

The immutable-interpretation gate passed the full 46-test native suite and strict
Clippy: run-heavy rc=0, 116 s, peak RSS 0.22 GiB, minimum available 13502 MiB,
peak load 0.95. Log: `~/tmp/zencodec-interpretation-full-2026-10-26.log`.
This is test-process usage, not a native decoder resource bound.

The auxiliary-interpretation extension passed the full native matrix and strict
Clippy, plus the same AV1 ownership/sample matrix and guide browser checks.
The [receipt](../../benchmarks/jxl_auxiliary_output_2026-10-26.json) records the
tested commit, commands, dependency revisions and log hashes. Its full native
run reports `run-heavy: done rc=0 115s | peak-RSS 0.22GiB | min-avail 13601MiB |
peak-load 1.12`. This remains a test-process measurement.

The subsequent full native TIFF/JXL gate passed with the pinned pixel change:
run-heavy rc=0, 103 s, peak RSS 0.34 GiB, minimum available 11325 MiB,
peak load 1.52; log `~/tmp/zencodec-native-samples-full-2026-10-25.log`.

The source-subsampling extension found a native `SectionTooShort` failure for
valid reduced auxiliary planes with full-resolution color. The correction in
[decoder PR 68](https://github.com/imazen/zenjxl-decoder/pull/68) applies the
source shift independently of color upsampling and validates the effective 8x
ceiling. All 21 generated cases now pass common output adoption with exact
native sequential-versus-selected F32 bytes and distinct source/output geometry.
The independent oracle fully decodes generated fixtures. Its separate native
qualification also checks exact no-dither u8 samples against libjxl float output
under explicit matching quantization. That is not a claim of identical F32
arithmetic across decoders. See the [receipt](../../benchmarks/jxl_subsampled_output_2026-10-26.json).

### A retained spot plane can already be applied

The selected output now records `ColorContribution::{Separate, Applied}` beside
a spot channel's source linear RGBA. The native adapter can qualify both routes
without changing the image identity. This is an output fact, not source metadata
or permission to render it again. The private native boolean used by this test
is not a proposed shared configuration API.

A generated 9x7 black image with binary spot coverage is decoded in unrendered,
rendered and unrendered order. All active F32 bytes match independent libjxl
under the same setting, including the retained auxiliary plane. The exactly
representable original values also provide a direct oracle. Rendering changes
the color pixels while leaving auxiliary bytes unchanged; the contribution fact
survives decoder/input drop. This narrow fixture qualifies independent F32 parity,
not general cross-decoder floating-point equivalence.

The [receipt](../../benchmarks/jxl_spot_contribution_2026-10-26.json) records the
full TIFF/JXL matrix, the shared native AV1 output test and guide checks. Full
native run: `run-heavy: done rc=0 119s | peak-RSS 0.22GiB | min-avail 13298MiB |
peak-load 1.13`. This is test-process usage, not a native workspace bound.

### Policy follows the actual held output

`zencodec::operation::resolve_image` consumes an `ImageOutput`, invokes the
existing fallible resolver with `Question::Image`, and returns the same owner
with its decision record only on acceptance. Image and video publication share
`PublicationOffer::PreserveNative`; there is no extra callback interface. The
callback sees actual immutable output and bounded facts, including whether
retained spot planes already contributed to color. Those counts do not certify
that every source channel was discovered or retained.

The native TIFF case admits a selected SubIFD and retains its exact extent,
allocation address and samples across classic/BigTIFF and chunky/planar variants.
The JXL case alternates rendered/unrendered selections under one identity; a rule
against already-applied retained spots refuses the actual rendered route and
accepts the separate route, preserving all color/auxiliary bytes. The native AV1
matrix passes its mapped output through the same resolver while retaining plane
addresses, code depth, crop, timing and owner lifetime.

Authored ownership controls additionally check stale choices, callback failures
with retained typed traces, refusal, cancellation before/after callback and
unwind. Presets requiring metadata treatment, no opaque profiles or a workspace
bound refuse: this publication offer has no evidence for those obligations.
This boundary follows native reconstruction and is not allocation admission or
a whole-container privacy certificate. Caller-retained clones remain independent.

### Native qualification

[`src/jxl_items.rs`](src/jxl_items.rs) exercises the same operation-bound image
identities against native zenjxl-decoder at
`4365fe97a955bcd0b9c3f30ca33678e8b1e85478`. The existing test corpus remains pinned
at `8399f17c54c8138f65304bbc46efc80c269c7402` so changing the decoder does not
change its regression inputs. `codec-corpus` retrieves the immutable archive; missing
fixtures fail. The harness retains the compressed input and scans presentation
headers. It does not claim a streaming-input or native-workspace bound.

The matrix selects every presentation in reverse order and compares every f32
sample byte and extra channel against native sequential decoding. Output rows
have 13 padding bytes, whose sentinel must survive. The six fixtures contain
147 visible presentations; 35 require replaying an earlier visible frame. The
backend's opaque seek target owns the dependency restart; the adapter does not
guess a keyframe from the visible index or file offset.

The independent [`jxl_oracle.c`](jxl_oracle.c) uses libjxl only for test-time
discovery. It confirms presentation count, duration ticks and canvas dimensions.
It also demonstrates why view selection belongs before discovery:

| Fixture | Composited presentations | Uncomposited regular layers |
|---|---:|---:|
| `blendmodes.jxl` | 1 | 5 |
| `multiple_layers_noise_spline.jxl` | 1 | 5 |

The latter includes cropped 1024×576 layers at nonzero origins inside a
2048×1152 canvas. Native `coalescing=false` still returns the composited output,
so the candidate refuses required `Uncomposited` before parsing. This is tracked
in [zenjxl-decoder issue 57](https://github.com/imazen/zenjxl-decoder/issues/57).
The independent library is libjxl 0.11.1 locally; CI records the installed
library's exact numeric version in the test log. Install its development headers
and library before running the gate (`libjxl-dev` on Ubuntu). Its executable is
built outside the fresh TIFF artifact directory, and each invocation has a
30-second deadline.

The combined native tests, independent TIFF/JXL oracles and strict Clippy passed
with `jxl-items4` on 2026-10-25: run-heavy rc=0, 104 s, peak RSS 0.42 GiB, minimum
available 15782 MiB, peak load 1.64. Full log:
`~/tmp/zencodec-jxl-items4-2026-10-25.log`. This is the test process receipt,
not a per-codec or whole-operation memory guarantee.

These are native cross-path pixel checks and independent discovery checks, not
independent JXL pixel readback. Layer decoding, layer blend/extra-channel facts,
gain maps, out-of-order container chunks, complete metadata interpretation and
source replacement during network access remain unqualified. libjxl exposes
regular layers when coalescing is disabled; reference-only and low-frequency
frames remain internal ([decoder contract](https://libjxl.readthedocs.io/en/latest/api_decoder.html)).

## Reduced-level region candidate

This is an executable candidate, not a public spatial API. It selects a main
TIFF page, follows an explicit path of reduced SubIFDs, and returns a region in
stored coordinates. Native sample bytes and orientation remain unchanged.
TIFF parsing and coding-unit decoding come from the pinned native codec.

Missing qualification: full color/ICC interpretation, auxiliary-image dependencies,
whole-container privacy, live HTTP version binding, total native workspace bounds,
cross-unit batched scheduling, wide planar samples and independent wide-sample
readback. The current external
oracle covers the first classic RGB8 page, with both chunky and planar tiles;
it does not establish those missing contracts.

## Run

Use an export outside workspace path overrides. Install the oracle in an isolated
Python environment using `just spatial-oracle-install <python>`, then:

```sh
just spatial-validation-setup
run-heavy --mem 12G --jobs 4 -- timeout --kill-after=15s 5m \
  just spatial-validation-check <fresh-persistent-artifact-dir> <python>
```

Both oracle and artifact arguments are required. Missing tools fail the gate.
Each external oracle process also has a 30-second deadline. The harness uses
Pillow 13.0.0 with libtiff explicitly selected, checks that selection, and compares
its full-page bytes against the original fixture samples. The native encoder
writes the fixture grammar. No TIFF parser or compression implementation is
duplicated here.

## Measured coverage, 2026-10-25

The initial matrix covers 24 combinations: classic/BigTIFF, chunky/planar,
direct/4096-byte `BufReader`, and two independent pages plus one reduced SubIFD.
A 6×5 region at (13,12) crosses four 16×16 tiles, including the reduced image's
partial right/bottom tiles. Every returned sample matches the original raster.
The selected SubIFD also agrees with the native full-image route.

Fault cases cover partial writes followed by read errors, source replacement,
final cancellation, invalid reduced-image roles, discovery/output/unit/tag
allowances, and cursor/seek overflow. Concrete source errors survive TIFF,
standard I/O and framework error inspection. Output stays private until success;
an earlier successful result remains valid after a later failure.

Actual call and byte counts are in
[`spatial_tiff_reads_2026-10-25.tsv`](../../benchmarks/spatial_tiff_reads_2026-10-25.tsv).
For classic chunky page 0, buffering changes 68 reads / 3,020 bytes into
9 reads / 34,390 bytes. For its planar reduced SubIFD, the change is 229 reads /
2,228 bytes into 21 reads / 84,038 bytes. These are source-call counts and bytes
copied, including rereads. No disk throughput, network latency or codec CPU
performance was measured. A fixed `BufReader` alone is not evidence of a good
network plan; byte admission and round trips must be considered together.

Command on r7900x, Rust 1.99.1, isolated export with committed dependency lock:

```sh
run-heavy --mem 12G --jobs 4 -- timeout --kill-after=15s 5m \
  env TMPDIR=$HOME/tmp \
  CARGO_TARGET_DIR=$HOME/tmp/zencodec-io-experiments-2026-10-24/target \
  just spatial-validation-check \
  /mnt/v/output/zencodec/spatial-2026-10-25/run4 \
  $HOME/tmp/spatial-oracle-2026-10-25/bin/python
```

Tests and strict all-target Clippy passed. Full local log:
`~/tmp/zencodec-spatial-tiff4-2026-10-25.log`.

```text
run-heavy: done rc=0 1s | peak-RSS 0.19GiB | min-avail 16698MiB | peak-load 0.61
```

That process-tree peak includes compilation, tests, oracle and Clippy. It is not
a codec workspace measurement. Artifacts remain in the directory above with
`SHA256SUMS`; independent readback recorded Pillow 13.0.0 / libtiff 4.7.2.

## Strip and sample extension

`strip_tests.rs` adds 22 native layouts: Gray unsigned/signed 8/16/32/64 and
float 32/64; RGB/RGBA unsigned 8/16/32/64 and float 32/64. All run with classic
TIFF and BigTIFF, uncompressed/PackBits/LZW/Deflate, and no predictor versus the
applicable horizontal or floating-point predictor. Each checks a middle region,
the final partial strip and the complete image against original sample bits.
All eight orientations are reported without transforming stored coordinates.
Float fixtures include signed zero, infinities, subnormals and NaN payloads;
comparison is byte exact, including payloads. This is native fixture parity,
not an independent wide-sample oracle.

The same full gate passed with artifact argument `run5`. Full local log:
`~/tmp/zencodec-spatial-wide2-2026-10-25.log`.

```text
run-heavy: done rc=0 3s | peak-RSS 0.37GiB | min-avail 16559MiB | peak-load 0.45
```

This includes compilation and the original external RGB8 gate. GrayAlpha,
half-float TIFF, wide planar tiles and opposite-endian files remain unqualified.

## API implications

`zencodec_io::ReadAtReader` is the shared synchronous `Read + Seek` bridge used
by the native candidate, with its own cursor.
It adds no prefetch, retry or async suspension. End-relative seeks call `len`,
which may consume a sequential source. A file, range cache or HTTP adapter owns
those policies; adapting a parser does not make the parser resumable.

Pages, reduced levels, stored regions, and coding units have separate identities.
An arbitrary SubIFD is not automatically a pyramid level. An output-byte bound
does not bound metadata, native workspace or transport staging. The eventual
shared request needs executable choices and the existing policy resolver;
this experiment's mechanical cancellation check is not a second policy system.

The promoted bridge repeats this entire native matrix (`run6`, log
`~/tmp/zencodec-spatial-shared-bridge1-2026-10-25.log`): rc=0, 3 s, peak RSS
0.40 GiB, minimum available 16526 MiB, peak load 0.51 under `run-heavy`.
The I/O crate's full all-feature tests, strict Clippy and no-default-feature
check also pass through `just io-reader-check`. Direct native I/O values keep
their kind, OS code and payload; generic errors keep their concrete context.
Additional controls cover invalid counts, failed end-relative spool discovery,
EOF, standard buffered reads and no bridge-level retry on `Interrupted`.

## Executable guide and CI

The checked API page displays `docs/examples/checked_source.rs` directly. The
native disk fixture executes that same file. Documentation CI pins the TIFF
codec and Python oracle, runs this entire gate with command/job deadlines, and
requires it before Pages deployment.

The guide/native gate passed with `run7` (log
`~/tmp/zencodec-spatial-guide1-2026-10-25.log`): run-heavy rc=0, 3 s, peak RSS
0.39 GiB, minimum available 16483 MiB, peak load 0.51. Browser checks at
390/768/1440 px pass with signatures expanded and exact full-example copying
(`just docs-site-browser-check 3567`; rc=0, 8 s, peak RSS 0.17 GiB, minimum
available 16227 MiB, peak load 0.35). These are test-process receipts.

The common-output full gate passed on 2026-10-26: 43/43 native spatial tests
and strict Clippy; `run-heavy: done rc=0 112s | peak-RSS 0.37GiB |
min-avail 13386MiB | peak-load 0.91`. Log:
`~/tmp/zencodec-common-output-full-2026-10-26.log`. This is a test-process
receipt, not a decoder workspace bound.

The added exact-timing case and auxiliary-output cases passed with the 22 TIFF
sample cases and strict Clippy: `run-heavy: done rc=0 12s | peak-RSS 0.45GiB |
min-avail 13431MiB | peak-load 0.61`; log
`~/tmp/zencodec-common-output-timing-2026-10-26.log`.
