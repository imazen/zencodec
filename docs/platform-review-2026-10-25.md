# Platform review: consequences for the selected-image API

The platform review inspected zencodec at
[`da8dc02`](https://github.com/imazen/zencodec/tree/da8dc02ff59de149b94dafde69537122dad3818b).
It combined WIC/OpenImageIO documentation, Windows WPF imaging experiments,
independent TIFF readback, and reruns of existing native-codec test binaries.
Windows runtime evidence is **WIC-backed WPF**, not coverage of every raw IWIC
interface. No macOS runtime test was performed. Earlier source-review findings
about `PumpError::source` and mapping-error strings were already fixed by
[`6241cad4`](https://github.com/imazen/zencodec/commit/6241cad4).

## Decisions supported by the review

| Evidence | Consequence for zencodec |
|---|---|
| Windows selected equal-sized TIFF pages with different sample types and orientations in order 1, 0, 1 | Bind interpretation and geometry to the selected image. A collection-wide default cannot establish them. |
| OIIO explicit subimage reads preserved repeated/concurrent selection, channel subsets and partial edge tiles | Retain stable identities and explicit requests; avoid making a mutable current-page cursor the pixel contract. |
| WPF exposed a TIFF thumbnail whose `NewSubfileType` was zero; OIIO returned no thumbnail, but independent TIFF decoding matched its pixels | Keep identity, containment, thumbnail role and reduced-level role distinct. A SubIFD alone establishes neither of the latter two. |
| OIIO did not expose the native fixture's reduced SubIFD as mip level 1 | State catalog coverage. An unsupported discovery route must not masquerade as successful exhaustion. |
| All eight EXIF orientation cases matched independent full-image and cropped pixels, with untouched row padding and out-of-bounds refusal | Keep coordinate space, output origin and residual orientation explicit. Stored-coordinate regions are not oriented display regions. |

WIC's documented
[`SetPixelFormat`](https://learn.microsoft.com/en-us/windows/win32/api/wincodec/nf-wincodec-iwicbitmapframeencode-setpixelformat)
can succeed with a different format. zencodec should preserve its explicit
require/prefer distinction and verify actual output, rather than interpreting
successful configuration as proof that a requirement held.

Current WIC
[frame-chain roles](https://learn.microsoft.com/en-us/windows/win32/api/wincodec/ne-wincodec-wicbitmapchaintype)
include alternate images, layers, previews, thumbnails, alpha, depth and gain
maps, with potentially different dimensions. These are useful precedents for
related images; they are not interchangeable with temporal frames.

[OpenImageIO ImageInput](https://openimageio.readthedocs.io/en/stable/imageinput.html)
separates subimage/mip selection, native sample representation, channel ranges,
stride and metadata. Its thumbnail result can mean either no thumbnail or no
reader support. Our `Result<Option<_>, _>` discovery convention should preserve
the distinction when the adapter can establish it, and refuse an unqualified
query otherwise. Selecting a region does not establish cheap access: a backend
may reconstruct more than the requested output.

These precedents support the small checked-request API and a fallible policy
resolver. The resolver decides among disclosed alternatives; it cannot turn
unknown interpretation or an unimplemented operation into a successful result.
Lazy execution and bounded output queues also do not establish a bound on
compressed input, native workspace or retained uncompressed samples.

## Native implementation following the review

The shared [`Collection`](../src/collection.rs) now
takes an `ImageRequest` on each `decode`. Opening TIFF fixes composition and
resource allowances; it no longer fixes one rectangle for all pages and
children. A request selects the full image or an explicit stored-coordinate
region. Returned TIFF pixels carry the selected canvas size and region origin,
alongside the existing unapplied orientation and sample/color interpretation.

The native test visits differently sized pages and a shared SubIFD out of
order, alternates full/edge/interior requests, and compares every active byte
with the source raster. A rectangle valid on a larger page is rejected on a
smaller child. Empty and overflowing extents return an inspectable error with
the requested rectangle and selected dimensions. A failed request does not
consume the image identity. Classic/BigTIFF and chunky/planar variants run the
same consumer.

The JXL adapter currently qualifies full presentations only. It refuses a
required region instead of substituting a full canvas; a subsequent explicit
full-image request still matches native sequential decoding. No JXL ROI or
workspace-efficiency claim follows from this refusal test.

The [resulting implementation](https://github.com/imazen/zencodec/commit/483f46301356f88474e6ccb9a877317d86c90b3e)
passed the complete native matrix (42/42) and strict Clippy. The
[run receipt](../benchmarks/platform_regions_2026-10-26.json) records
`run-heavy: done rc=0 101s | peak-RSS 0.22GiB | min-avail 13083MiB |
peak-load 1.35`, for the test process. The source-extracted guide and browser
checks also passed, including inline context, clipboard, keyboard and
390/768/1440-pixel layouts.

## Evidence and limits

The review's independent tools were OpenImageIO 3.2.1.3, tifffile 2026.10.15,
Pillow 13.0.0 and imagecodecs 2026.10.10. Results and receipt hashes are preserved
in [the evidence record](../benchmarks/platform_selection_2026-10-25.json).
Raw artifacts remain outside git at
`/mnt/v/output/zencodec/platform-selection-20261025/`.

The reviewer freshly reran 38/40 selection cases and 27/27 checked native
JPEG/PNG cases, and verified the prior 40/40 full selection receipt. The latter
includes 147 JXL presentations, 35 requiring dependency replay. An initial
disk-test output-directory failure was corrected and that case passed on rerun.
The full receipt was `run-heavy: done rc=0 103s | peak-RSS 0.34GiB |
min-avail 11325MiB | peak-load 1.52`. It measures the test process, not a codec's
workspace bound. Log hashes were independently checked when integrating this
review.

Remaining promotion gates:

- Broader rendering/conversion contracts for depth, spot-color, gain-map and
  other auxiliary channels. The shared core output now carries verified native
  samples and interpretation; that alone does not define display conversion.
- Native discovery-work accounting beyond the now-tested common call allowance.
  Cancellation/timeout causes are qualified at observed polls; native latency
  remains unbounded. Single-object conditional HTTP reads now qualify through
  TIFF/SubIFDs. A private Linux/XFS clone source now qualifies stable disk bytes
  after overwrite/truncate; multi-object, browser and other filesystem/platform
  source contracts remain open. [Disk evidence](../benchmarks/file_snapshot_2026-10-26.json).
- Qualified thumbnail, pyramid and auxiliary roles, plus native JXL layers.
- Native workspace accounting, incremental still-codec input, planar conversion
  and whole-container privacy coverage. None follows from passing selection
  or output-buffer tests.

Performance work remains separate from these correctness gates. Existing
[TIFF I/O measurements](../benchmarks/spatial_tiff_reads_2026-10-25.tsv) show one
4096-byte buffering case changing 68 reads/3020 returned bytes to 9 reads/34390
bytes, including rereads. This is not a latency measurement. Large-graph cycle
checking cost and whole-operation memory remain unmeasured.

## Native output follow-through

The common output gate now has an implemented core surface:
[`zencodec::image::ImageOutput`](../src/image.rs). Native TIFF/JXL adapters and
AV1 validation import that same implementation. It retains frozen output facts,
separate source details, native storage and mapped plane interpretation. It
refuses disagreement between native dimensions/code encoding and the held
signal, with typed traced causes. Spot-color output records whether its
retained auxiliary samples already contributed to the returned color.

The [public-output receipt](../benchmarks/public_image_output_2026-10-26.json)
records the full native matrix, workspace tests, strict lint, no_std WebAssembly,
rustdoc and guide checks. Its integration process reports
`run-heavy: done rc=0 138s | peak-RSS 0.54GiB | min-avail 13374MiB | peak-load 1.50`.
This is not a codec workspace bound. The shared collection is now implemented
on the API review PR; native adapter factories remain private. Policy
preallocation admission, native work/latency bounds and broader source snapshots
remain separate qualification gates.

The native raw F32 factory now supplies a private `JxlLayers` adapter. Nine
local layers retain placement, blend/reference facts and their original sample
allocation through the same automatic policy publication used for TIFF and
presentations. Reverse/repeated selection, off-canvas samples, scoped IDs,
discovery exhaustion and exact stop reasons have executing tests. Independent
CMYK/alpha samples are bit-exact; lossy RGB conformance differences are recorded.
The [raw collection receipt](../benchmarks/raw_collection_2026-10-26.json) preserves
failures as well as the succeeding integration gate.

That integration required explicit inverted-CMYK storage and black-plane
polarity. Attaching source ICC bytes must not imply conventional device values.
Storage stays unchanged and unqualified conversion refuses. The native factory
still restricts output to F32, stored orientation, skipped previews, unrendered
spots and retained black. Remaining blend configurations, native negative-origin
fixtures, reference-only export, progressive output, raw random seeks and memory
bounds remain open; source composition facts do not promise lossless re-encoding.
