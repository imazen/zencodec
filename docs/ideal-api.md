# Checked codec operations: target and implementation

Implementation is on [PR 168](https://github.com/imazen/zencodec/pull/168).
This is an active implementation record, not a declaration that the complete
API is implemented or stable. [spec.md](spec.md) contains current signatures;
[the comparison review](api-review-2026-10-25.md) records the verified external
precedents and the conclusions retained from Argon's review.

The site’s **Changes & why** section is maintained with each trait or adapter
change. It shows the problem, each role’s responsibilities, source-extracted
signatures, evidence and remaining scope. The [maintenance contract](site/README.md)
keeps the native examples and dependency revisions tied to that record.

The native-plane update also refreshes the unlisted migration diagram and
the guide's two admission/publication boundaries. Its exact examples execute
in the native AV1 matrix, and browser/source checks pass at 390/768/1440 px
with independent above/below expansion, whole-example copying and keyboard
navigation. The browser gate reports run-heavy rc=0, 8 s, peak RSS 0.17 GiB,
minimum available 15998 MiB and peak load 1.45; this measures the guide check.

## What still needs production implementation

The current priority is the API/native-codec validation loop. The spatial
candidate now exercises real TIFF directory graphs and JXL presentations before
adding a shared public selector. Performance tuning and additional platform
work do not gate that loop. See [the executable selection evidence](../perf/spatial-validation/README.md).

- Extend coupled routes beyond the qualified JPEG/PNG sources and JPEG/PNG targets.
  Add encoded rewrite choices, whole-container disclosure guarantees and
  qualified native workspace/staging bounds to the existing typed resolver.
- Extend native controlled-suspension qualification beyond the whole-source JPEG/PNG
  routes to native byte resumption. Native AV1 frame publication now uses the
  shared resolver. Native picture-plane admission precedes allocation, but
  coupled planar conversion, complete workspace admission and structured native
  storage-refusal facts remain open. The pixel-count cap and native plane-byte
  allowance do not cover work buffers, input, metadata or queued encoded output.
- Metadata exclusion/retention with explicit inspection scope, including opaque
  carriers, malformed XML, reconstruction data and nested content.
- Integrate managed storage ownership into policy waiting, conversion scratch
  and qualified encoded staging; qualify native codec/service storage separately.
- Shared image item, level and region selection with bounded discovery and
  source-version binding for the gigapixel consumer.
- Extend the source-extracted consumer/adapter/implementor guide as each remaining
  contract is implemented; keep executable examples and migration diagrams current.

## Implemented on the PR

| Contract | Implementation and verification |
|---|---|
| Synchronous parser bridge | `zencodec_io::ReadAtReader` supplies `Read + Seek` over a positional source, preserving native errors and its independent cursor. [Native TIFF experiments](../perf/spatial-validation/README.md) cover pages, reduced SubIFDs, planar edge tiles and wide strips. The shared spatial request, full interpretation and source-version binding remain open |
| Native video policy and accepted packet provenance | `Question::Frame` shares core interpretation/timing facts with media sessions. `zencodec_media::policy::PolicyDecoder` retains the native picture through every callback. Native AV1 queued clocks/epochs/colors and repeated ordinals preserve accepted context; refusal, stale decisions, cancellation, errors and caught unwinds release unpublished owners. 15 depth/layout configurations re-encode sample-exact under independent FFmpeg/libdav1d in both pooled and bounded native-plane modes. The native setting admits picture planes before allocation and keeps charges with native owners. Publication policy still follows reconstruction; complete workspace bounds and coupled conversion remain open. [Qualification and limits](../perf/av1-policy-validation/README.md) |
| Native batches fit admitted sink strips | `DecodeRowSink::max_buffer_rows` and shared copying split delivery automatically. Native PNG RGB8/RGBA16 at 97x65/257 preserve exact independent samples with 1/3/64-row transcode buffers. Native batch/workspace storage remains separate |
| Native standard-source errors survive erasure | `StdSourceError` retains I/O kind, OS code and custom payload; source, truncation and erased-driver tests |
| Buffered input admission precedes exact reads | Checked lookback selection, bounded retained history, exact-cap EOF probe, terminal excess and fallible retained allocation |
| Driver input allowances | `IoLimits` across sync/async incremental drivers; independent byte/call/capacity oracles, affordable alternatives, bounded batches, atomic range publication after partial failure/cancellation |
| CPU scheduling independent of output pressure | `WorkQuota` limits complete steps per async turn; inline scheduling and packet loops yield; quota yields do not drain output; exact native QOI bytes at four quotas |
| Terminal erased video errors | `FusedEncoder` protects every work method and cancellation, preserves native causes, and remains failed after a caught unwind; decoder reset still reopens its session |
| Initial stored requests | Required policy, fidelity configuration and output descriptors checked at execution; controlled sessions use the same registry templates and reject required-descriptor mismatch before adoption; inherited defaults, rejection and strided pixels tested; native baseline/progressive JPEG encoding bytes and decoding pixels match direct calls |
| Shared resolver domains | Typed prepared encoder and coupled transcode offers, configurable built-in policy, fallible application callback, scoped choices and bounded reports; stale/nested tokens, cancellation, unwind cleanup and native error traces tested. Metadata treatment, opaque-profile retention and qualified workspace bounds can filter complete transcode choices; unknown evidence refuses |
| Controlled interpretation protocol | Explicit suspensions and scoped replies; fused enforcement, coroutine await, and a shared policy adapter for existing transports. Authored controls cover early output, stale replies, finality changes, cancellation and unwind. [Native JPEG PR 240](https://github.com/imazen/zenjpeg/pull/240) supplies generic, erased and registered factories for the qualified whole-source byte-output route. Unqualified late ICC and float reconstruction refuse before native construction. [Native PNG PR 46](https://github.com/imazen/zenpng/pull/46) qualifies row and Adam7 reconstruction, packed and 8/16-bit layouts, late EXIF and conservative gain-map/extension discovery. Failed sessions release admitted storage while retaining typed refusal reports |
| Coupled native pixel transcode | `zencodecs::checked_transcode::TranscodeRequest` on the [integration PR 105](https://github.com/imazen/zenpipe/pull/105) retains the actual OutputPlan, checked encoder jobs and admitted row buffers during the shared callback. JPEG→sRGB PNG matches independent CMS/pixel inspection; JPEG preservation checks ICC/EXIF/XMP. PNG sources retain six integer layouts with exact independent readback and P3 CMS conversion. File/range and delayed network input match slice output. Callback/source failures and cancellation retain causes and release managed owners. Exact required density now passes from native PNG/JPEG input through retained jobs to native PNG output, with independent PNG metadata/pixel readback. Non-square JFIF aspect is retained; unrepresentable values and density with unqualified EXIF/XMP reconciliation refuse. Broader resolution cases remain [169](https://github.com/imazen/zencodec/issues/169). Native workspace and encoded staging remain unqualified |
| Managed storage owners | Byte/object admission follows allocations through subviews, packets, side data and old-epoch frames; exact strided 8/10-bit samples and concurrent contention checked. [Scope](storage-admission.md) excludes native workspace and explicit unaccounted copies |

Driver input capacity is a scoped allowance. It excludes source caches,
codec/output allocations, allocator bookkeeping and transient reallocation.
Existing codec memory estimates and capability flags are not whole-operation
memory guarantees. The new request layer does not strengthen those old claims.

## The consumer shape

### Selection beyond animation and pages

The native loop has established three constraints for the next shared selector:

- An image identity belongs to one opened operation. TIFF can share a SubIFD
  between parents, and a SubIFD need not be a reduced image. A containment edge
  and a resolution-level choice cannot be the same thing.
- Select the presentation/layer view before discovery. The same JXL file can
  expose one composited presentation or five layers. A boolean passed only to
  `decode(id)` cannot identify which layer the caller meant. Required views
  refuse when unsupported; a resolver may choose only an explicitly permitted
  alternative.
- Keep timing and decode dependencies separate from item identity. Selecting
  a JXL presentation can require earlier invisible or visible work. The backend
  supplies the restart target; a page ordinal or byte offset is insufficient.

The actual private candidates are `TiffItems::{images,children,next,decode}`
and `JxlItems::{open,next,decode}` under `perf/spatial-validation/src`. They
execute against the native codecs and share the opaque operation-bound identity.
The private `Collection` trait now executes the same `first_image` consumer
against both, returning traced errors and preserving source-replacement causes.
Unsupported child discovery refuses instead of reporting an empty collection.
The TIFF adapter now adopts native samples into a strided `PixelBuffer`,
retaining the selected directory's opaque ICC, alpha association and unapplied
orientation. Signed and wider samples require
[zenpixels PR 93](https://github.com/imazen/zenpixels/pull/93); the native strip
matrix checks exact bytes and original allocation ownership. Unknown color
stays unknown, and unresolved alpha refuses. This is raw sample carriage, not
an approved display conversion or privacy decision.
The collection is not yet a public `CodecSet` API. Its mapped output is now the
implemented `zencodec::image::ImageOutput`, also used by native AV1 validation.
Packed color, auxiliary values and component codes stay distinct. The output
has typed traced failures for invalid regions, missing planes and disagreement
between native mappings and held dimensions/code encoding.
Its interpretation is frozen with the storage and checked against packed sample
geometry. JXL retains the native source header/profile and selected-frame facts
separately from the actual output color profile; independent metadata checks
cover HDR, floating-point precision, preview and animation declarations. Native
encoder fixtures now qualify all seven named auxiliary roles, spot/CFA parameters,
half/float alpha precision and per-frame timecodes through the shared consumer,
using [native decoder PR 68](https://github.com/imazen/zenjxl-decoder/pull/68).
Absent timecodes remain distinct from present zero, and neither substitutes for
PTS. Source shifts 1/2/3 now qualify on odd-sized canvases after the native
fix for [subsampled channels](https://github.com/imazen/zenjxl-decoder/issues/69).
Spot channels additionally state whether they already contributed to returned
color; enabled and disabled rendering pass an independent F32 oracle for an
exactly representable fixture. Unknown/reserved roles remain unqualified. These source facts do not authorize
metadata passthrough or additional rendering of an already-composited color plane.
Each decode carries its own region
request; TIFF returns the selected canvas and region origin, while required JXL
regions refuse. The [platform review](platform-review-2026-10-25.md) records
WIC/OpenImageIO evidence and the distinction between containment and roles. The owned output now has `operation::resolve_image`, using the existing resolver
and a shared `PublicationOffer`. It returns the same owner only on acceptance;
failed decisions drop unpublished output. This is post-reconstruction publication,
not pre-allocation admission or a privacy certificate. Collection promotion still
needs native discovery-work accounting and source-revision validation. A private
common wrapper now bounds discovery calls consistently across cached/repeated
TIFF and JXL browsing; it does not bound work inside a native call. The native
collection adapters now carry one existing `StopToken` through discovery,
selected replay and final output checks. Cancellation injection covers observed
polls in classic TIFF/BigTIFF SubIFD and JXL presentation operations; native-call
latency remains unqualified. Both cancellation and timeout retain their exact
reasons through native errors and collection traces in these operations. Raw JXL layer implementation is
still [decoder issue 57](https://github.com/imazen/zenjxl-decoder/issues/57);
the existing [paged-decoder issue](https://github.com/imazen/zencodec/issues/1)
owns the shared-selector discussion. The [public-output receipt](../benchmarks/public_image_output_2026-10-26.json)
records the executing core/native qualification.

### Ordinary checked operations

Keep the small conveniences and registered codec templates. A stored request
adds requirements when the application needs them:

```rust
use zencodec::{CodecSet, ImageFormat, operation::{DecodeRequest, EncodeRequest}};
use zenjpeg::{JpegDecoderConfig, JpegEncoderConfig};
use zenpixels::PixelDescriptor;

let codecs = CodecSet::new()
    .with_decoder(JpegDecoderConfig::new())
    .with_encoder(JpegEncoderConfig::new());
let image = DecodeRequest::new()
    .with_output(PixelDescriptor::RGB8_SRGB)
    .decode(&codecs, input)?;
let encoded = EncodeRequest::new(ImageFormat::Jpeg)
    .encode(&codecs, image.pixels())?;
```

The request remains reusable. A failed requirement is an error, not a successful
result with a warning that callers must discover. `preferring_output` explicitly
allows the decoder's native fallback. Conversion remains an explicit operation;
the coupled transcode adapter qualifies its interpretation and destination
signaling together for the native configurations listed above.

The resolver now selects prepared encoder jobs. A caller can use
`preferring_fidelity(...)` and `with_policy_set(PolicySet::new().prefer_lossless(true))`,
or pass a `PolicyResolver` to `encode_with`. Required targets never become
preferences. The selected job is the same object that was checked before the
callback, and execution errors do not trigger a retry. The returned `Outcome`
contains both output and a bounded decision report.

The extension to other domains must preserve that execution link. The resolver
chooses only offered, permitted routes or returns a traced refusal/error. It
cannot invent support or authorize stale facts. Adapters construct and admit
executable routes; codecs expose facts and mechanisms without depending on an
application policy. Native late-metadata suspensions and coupled pixel transcodes
now execute through that same resolver. Complete-container disclosure, native
resource bounds, sequential publication and other codec routes remain open work.

## Compatibility plan

| Change | 0.2 / breaking-release treatment |
|---|---|
| Stored request entry points and typed resolver support | Additive candidates; new signatures stay on the review PR |
| Native source errors | Concrete error-type change in the unpublished I/O crate; migrate typed source consumers |
| Input admission and work quotas | Opt-in additive controls; legacy defaults remain compatible |
| Fused erased encoder behavior | Stricter behavior in unpublished media APIs; error-only shims preserve explicit backend retry contracts |
| Legacy best-effort setters and capability coordination | Keep during migration; only retire after the checked replacement covers their real consumers |
| Metadata preservation names and mandatory suspension | Require explicit migration and codec qualification; no silent reinterpretation of released contracts |

The goal is successful execution that establishes the requested contract, or an
actionable refusal. Completion requires native codec and pipeline evidence for
each remaining row above, not only types or synthetic protocol tests.

## Adapter policy defaults (2026-10-25)

`EncodeRequest::with_policy_defaults` lets a coupled adapter preserve its
prepared color description without replacing application requirements. Each
explicit field in the request policy, or the inherited registry policy when no
request policy is supplied, takes precedence. This includes explicit `false`.
Unspecified fields receive adapter defaults, and the complete result goes
through the codec's checked setter. Unsupported defaults refuse preparation.
The all-`None` default leaves the backend template untouched.

Local and transferable preparation share this rule. `operation_defaults` tests
inheritance, explicit replacement, all five fields, unchanged templates and
unsupported defaults through both paths. The full workspace, `no_std`, strict
core lint and public API snapshots passed. Resource-wrapper receipt for
`just operation-send-integration`: rc=0, 10 s, peak RSS 0.33 GiB, minimum
available 16869 MiB, peak load 0.51. This is test-process usage, not a codec
workspace bound.

## Executable API guide (2026-10-25)

The documentation build now produces `ideal.html`, linked as **Checked API**.
It separates consumer, adapter and implementor interfaces and distinguishes
implemented review changes from the remaining 0.3 target. The independent
migration page remains unlisted. The new page's one-line calls grow into an
explicit sRGB export, a conditional tenant policy, positional-file input and
async byte demand. Imports and return context expand independently inline.

The displayed Rust is `docs/examples/checked_operations.rs`, executed directly
by the pinned native harness. All 26 native JPEG/conversion scenarios passed;
the guide case covers baseline/progressive source, default P3 preservation,
metadata exclusion, a traced tenant refusal and identical file/async results.
Strict harness Clippy passed after simplifying the example's conditional.
`just docs-checked-examples` receipt: rc=0, 1 s, peak RSS 0.51 GiB, minimum
available 17664 MiB, peak load 0.27. These are test-process observations.

Core signatures are extracted from current source. The companion integration
signatures retain parameter names and public imports, captured from the exact
commit used by the compiled examples (`just checked-guide-api CHECKOUT COMMIT`).
The build rejects a revision/checksum mismatch. Browser checks cover exact
clipboard content, independent context expansion, keyboard navigation and
390/768/1440 px layouts with expanded signatures. Receipt for `just
docs-site-browser-check`: rc=0, 11 s, peak RSS 0.19 GiB, minimum available
16901 MiB, peak load 0.77. This is browser-test usage, not application memory.

Executing the default example exposed the P3 carrier-authority and packed
palette precision regressions fixed in the coupled adapter. Native PNG checked
verbatim emission is on [PR 46](https://github.com/imazen/zenpng/pull/46).
The broader content-dependent fidelity contract remains
[104](https://github.com/imazen/zencodec/issues/104).

Mobile navigation regression (2026-10-25): adding the checked API entry exposed
an unwrapped header in the CI browser at 390 px. The navigation now wraps within
the available width and the header grows with its links. The existing strict
no-overflow assertions pass without changed thresholds (`just
docs-site-browser-check 3547`, rc 0; run-heavy peak RSS 0.17 GiB, minimum available
17154 MiB, peak load 0.13). These are browser-process observations, not codec
memory measurements.

### Native PNG and admitted row capacity (2026-10-25)

All checked guide recipes now run on both native JPEG (baseline/progressive)
and the resulting PNG sources, including file input, delayed short network
reads, automatic selection, explicit CMS and callback refusal. The larger guide
fixture found and now covers native batches exceeding admitted sink rows. The
separate native integration checks six integer layouts with exact independent
PNG readback and P3-to-sRGB against a separately constructed CMS transform.

`just checked-guide-integration` passes the complete native harness and strict
Clippy with zenpipe `0287369d`, zenjpeg `02fa3866`, zenpng `3046bd77`,
zenpixels `f8596aae` and this core. run-heavy: rc=0, 13 s, peak RSS 0.73 GiB,
minimum available 15591 MiB, peak load 0.47. This is a test-process receipt;
native codec/CMS workspace and encoded staging remain unqualified.

The updated site passes its source/signature checks and browser checks at
390/768/1440 px, including inline expansion, complete-example copy and hidden
migration views (`just docs-site-browser-check 3557`; rc=0, 7 s; run-heavy peak
RSS 0.17 GiB, minimum available 15977 MiB, peak load 0.39).

### Living improvement record and exact-density example (2026-10-25)

The checked page now starts with a compact **Changes & why** section. Its video,
density, resolver and positional-I/O entries separate the reason for the change,
consumer/adapter/implementor consequences, extracted signatures and remaining
work. The main guide links to it. The existing unlisted migration page remains
separate from the implementation record.

The exact `with_resolution` example executes against native PNG with JPEG-decoded
input. It checks asymmetric physical densities, equivalent centimeter units,
unitless aspect, retained transcode density and unchanged pixels; exact 300 DPI
refuses. This guide gate supplements the independent reader checks in the
native transcode repository and the separate rational arithmetic oracle.

`just checked-guide-integration` passed native scenarios and strict Clippy with
zenpipe `ffecbf56`, zenjpeg `4535ec49`, zenpng `7f13367f` and zenpixels `f8596aae`.
run-heavy: rc=0, 18 s, peak RSS 0.75 GiB, minimum available 15443 MiB, peak load
0.56. `just docs-site-browser-check 3557` passed source checks, keyboard expansion,
full-example copying and 390/768/1440 px layouts: rc=0, 7 s, peak RSS 0.17 GiB,
minimum available 15845 MiB, peak load 0.37. These are test-process receipts,
not native codec memory bounds.
