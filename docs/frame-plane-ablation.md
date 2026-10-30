# Frame/plane ablation before release

Publication is held for interchange design. Behavior can be corrected later;
consumer-owned struct literals, trait implementations and sample representations
are expensive to replace after release.

## First cut: one checked integer encoding

```rust
use zenpixels::{ChannelType, sample::SampleEncoding};
use zencodec::frame::{PlaneSamples, PlaneView};

let encoding = SampleEncoding::new(ChannelType::U16, 10, 6)?;
let plane = PlaneView::new(PlaneSamples::U16(words), width, height, stride, encoding)?;
// The conversion library accepts plane.encoding() directly.
```

The core duplicate and its media re-export are removed in the candidate. This
changes a type's defining crate, not sample values. No compatibility alias is
added: callers import pixel types from zenpixels. Constructor convenience can
live in that defining crate instead of requiring a second public encoding type.
See [qualification](../benchmarks/frame_ablation_2026-10-30.md).

## Remaining structural decisions

These are source-verified gaps, not claims that a new replacement is implemented.

| Case | Current representation | Decision required before release |
|---|---|---|
| U8/U16 integer YUV with padded rows | `Frame` + `PlaneView`; common encoding in `FrameInterpretation` | Retain direct typed borrowing and sample identity. |
| Interleaved component storage such as NV12/P010 | zenpixels `CodePlane` has component stepping; core `PlaneView` does not | One canonical view must preserve both component stride and borrowed raw storage. Do not insert a deinterleaving copy to hide this gap. |
| Different alpha/color code depths | `OwnedPlanes` and `FrameInterpretation` each impose one encoding | Put representation facts at component scope; keep convenience for uniform layouts without making it a requirement. |
| Floating/signed/wide scalar auxiliaries | `ImageOutput` offers byte-backed `ValueSlice` with `ChannelType`; `Frame` only maps U8/U16 | Reconcile mappings and typed access, including alignment/endian semantics. Do not interpret floats as integer code encodings. |
| Packed processing output plus auxiliary planes | `ImageOutput`/`PackedImageOwner`, separate from `Picture` | Avoid forcing generic applications to choose a second owner contract merely because a codec changes its storage layout. |
| Timing and repeated presentation | `Frame::represent` retains picture and interpretation; `ImageInterpretation` also has timing/presentation | Separate immutable sample ownership from presentation without losing repeat-frame sharing. |
| Crop, signed placement, subsampled grid, render size | `FrameGeometry`, `ImageGeometry`, `Placement`, and `AuxiliaryGrid` describe different spaces | Preserve distinctions; names alone are not grounds for merging. Odd-origin chroma crops must retain sampling phase. |
| Mapping errors | `MappedPlanes::plane` returns `Option`; `MappedImage::plane` returns `Result<Option<_>, At<OutputError>>` | A declared missing/unmappable plane needs a typed failure rather than accidental end-of-plane iteration. |

Replacing `PlaneView` with today's canonical `CodePlane` alone is insufficient:
`CodePlane` has step-aware code reads but no public raw row/word borrow and only
unsigned U8/U16 storage. Conversely, adding every scalar variant to `PlaneSamples`
alone leaves the duplicated owners, interpretation and mapping signatures intact.

## Acceptance controls for the next cut

Use real adapter implementations as the acceptance boundary, alongside shared
conformance tests. The matrix must cover native padded YUV, semi-planar component
views, mixed-depth alpha, float/signed auxiliary values, mapped-owner retention,
repeated presentations, and out-of-canvas/odd-chroma placement. Validate strides,
sample bits and typed failure behavior, not merely successful construction.

A mapping must borrow the retained owner; no normalization, relabeling or hidden
full-plane copy can satisfy an interchange test. Exactness evidence describes
current samples, not the source file's historical bit depth. Native AV1/JXL/TIFF
adoption and codec-agnostic processing must compile against the same pixel types
before this design gate closes.

## Executed storage ablation

The [private byte-backed model](../perf/plane-ablation/README.md) passes its
scalar/stride/endianness controls and real AV1 mapping checks at 8/10/12 bits,
4:2:0 and 4:4:4. It preserves every tested native row address and sample after
decoder drop. This supports eliminating type-specific storage variants while
keeping code interpretation separate. It does not yet justify promoting the
private constructor, merging image/frame owners or claiming TIFF/JXL coverage.
[Exact receipts and limits](../benchmarks/plane_model_2026-10-30.md).
