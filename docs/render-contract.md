# Shared rendering contract

`zencodec-render` is the optional processing layer over zencodec's `ImageOutput`
and `CheckedCollection`. It performs no input I/O. Codec registration, source
access and collection selection stay in the existing sans-I/O framework.

The default `Renderer::new(storage)` requires the full oriented canvas in sRGB
RGBA8. This is a display conversion: depth reduction rounds and values can clip
to the display range. For precision-sensitive applications, configure a
`RenderRequest` with the required descriptor, explicit `ColorTarget` and
conversion options. `RenderRequest::new(descriptor)` preserves the current
sample color space and starts with channel/depth restrictions. A descriptor is
not permission to relabel source color. `with_cms` supplies a backend when the
requested profile conversion requires one; unsupported plans return an error.

The operation prepares color for the held source, validates geometry and every
auxiliary alignment, admits output storage, projects orientation/crop, and
executes the prepared conversion. Scalar auxiliary values and semantic roles
survive independently of color replacement. Input interpretation is retained as
provenance; output facts describe the resulting pixels. Unknown auxiliary
registration refuses even when dimensions happen to match.

`render_selected(&mut CheckedCollection, &ImageId)` uses `decode_with` so the
collection's stored policy runs once on the final image. Cancellation is shared
through decoding and rendering. `render(ImageOutput, resolver, preset, stop)`
provides the corresponding operation for an already-decoded image. Failure
retains stage and concrete cause; rejected or partial output is not published.

The geometric helper `geometry::project` is also available for applications
requiring orientation/cropping without color arithmetic. It requires explicit
composited packed samples and rejects unresolved placement, raw layers or
missing source coverage. The default region is the full oriented canvas, not
the dimensions of an oversized raw frame. It does not synthesize missing
canvas pixels or perform blending.

The current implementation decodes the complete native selection. A small
requested display region is not a promise of proportional decoding work.
Identity geometry still copies; source and destination owners can coexist.
Managed output admission excludes codec/CMS workspaces and bookkeeping. Planar
video, resizing, HDR presentation and general auxiliary resampling are not
qualified by this API. These limits remain explicit errors or unprovided
operations, not silently weakened requests.

Validation imports the shared renderer directly. The native suite exercises
TIFF/BigTIFF/SubIFD selections, JXL composition/layers, U8/U16/F32 sources, all
eight orientations and registered auxiliaries; independent libjxl, libtiff and
Little CMS readers supply comparisons. On 2026-10-27 all 147 spatial cases and
strict lint passed, including a new default-render test preserving alpha after
all eight orientations. Existing assertions were retained. Raw evidence on
r7900x is in
`/mnt/v/output/zencodec/platform-regions-20261026-run/public-render-20261027-full`;
log `/home/lilith/tmp/zencodec-public-render-full-20261027.log`.
Run-heavy: `rc=0 58s | peak-RSS 0.60GiB | min-avail 11270MiB | peak-load 0.99`.

The new crate is isolated while its converter dependency is on review revision
`86d0e2d30406a006bf8332f142029b7f58353086`. The native dependency lock gains only
`zencodec-render`; existing package versions are unchanged. Core zencodec has
no new dependency. The standalone check, lint and compiling example passed:
`rc=0 9s | peak-RSS 0.33GiB | min-avail 11497MiB | peak-load 1.03`.
This is validation resource usage, not a compiler-performance comparison or
proof that the entire dependency graph supports a freestanding target.

The explicit Rust 1.89.0 check passed separately:
`rc=0 6s | peak-RSS 0.30GiB | min-avail 11434MiB | peak-load 0.70`.
The documentation workflow runs both standalone and native renderer checks.
