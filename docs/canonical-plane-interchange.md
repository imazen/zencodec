# Canonical plane interchange

A frame keeps one validated `zenpixels::plane::PlanarLayout`. It describes each
plane's role and encoding, plus chroma subsampling and siting. Frame geometry,
presentation timing, current color/alpha interpretation, source color claims and
configuration epoch remain in zencodec.

```rust
use zenpixels::{ChannelType, plane::{PlanarLayout, PlaneEncoding, PlaneRole, PlaneSpec},
    sample::{SampleEncoding, ChromaSubsampling, ChromaSiting}};
use zencodec::frame::{FrameGeometry, FrameInterpretation};

let y = SampleEncoding::new(ChannelType::U16, 10, 6)?;
let alpha = SampleEncoding::new(ChannelType::U8, 8, 0)?;
let layout = PlanarLayout::new(vec![
    PlaneSpec::new(PlaneRole::Luma, PlaneEncoding::Code(y)),
    PlaneSpec::new(PlaneRole::Alpha, PlaneEncoding::Code(alpha)),
], ChromaSubsampling::Mono, ChromaSiting::Unknown)?;
let interpretation = FrameInterpretation::new(FrameGeometry::full(640, 480), layout)
    .with_alpha_mode(zenpixels::AlphaMode::Straight);
assert_eq!(interpretation.uniform_code_encoding(), None);
# Ok::<(), Box<dyn std::error::Error>>(())
```

An absent common encoding means inspect the per-plane layout. It does not mean
8-bit, unknown storage, or permission to normalize the samples. `FrameFacts`
keeps all ordered plane specifications in a bounded inline record, so mixed-depth
conversion offers cannot compare equal accidentally. Policy questions retain the actual frame or
image, including its full layout.

For an owned software frame, `OwnedPlanes::new(planes, encoding)` remains the
uniform integer convenience. `OwnedPlanes::from_encoded_planes` accepts each
owned plane with its `PlaneEncoding`, checks the storage type, and moves the
owners without copying their sample buffers. U8, U16 and F32 are supported,
including accounted shared storage. Direct frame mapping validates roles,
dimensions and native encoding against the retained layout before any consumer
gets a loan. Missing native encoding refuses; it never inherits the request.
Image access and canonical planar lending use the same shared validation.

## Sample words versus spatial positions

`AnyPlane::width()` counts active sample words per row. `Samples::dimensions()`
counts pixels for packed color and sample words for component/value planes. An odd-width 3-pixel 4:2:0 image has 2 chroma positions, so an interleaved
CbCr plane is 4 words wide. Padding is excluded from active rows.

```rust,ignore
let mapped = output.map()?;
let chroma = mapped.plane(1)?.ok_or("missing chroma")?;
let Samples::Codes(codes) = chroma.samples else { return Err("integer codes required".into()) };
let first_component = codes.component(0)?;
let second_component = codes.component(1)?;
// Position 1, not word 1. Interpret component order using CbCr versus CrCb.
let a = first_component.code(1, 0);
let b = second_component.code(1, 0);
```

`codes.code(x, y)` is a convenience for single-component planes. It returns
`None` for an interleaved plane so a caller cannot accidentally read Cr as the
next Cb. `row()` still exposes the active interleaved words.

F32 planes use `Samples::Float { plane, encoding }`; their `FloatEncoding`
records `stored = nominal * scale + offset`. Mapping preserves values and bits,
including excursions and NaNs. It performs no transfer, gamut, range, scale or
normalization conversion. Typed samples are native-endian.

## Migration and limits

- Replace the five-argument `FrameInterpretation::new` with geometry plus a
  validated `PlanarLayout`.
- Read roles and encodings from `layout.planes()`, subsampling and siting from
  their layout accessors. There is no second mutable frame-wide declaration.
- `MappedPlanes::encoding` returns `Option<PlaneEncoding>` for each index.
- `FrameFacts::encoding` is optional, describing only a common integer encoding.
- `OutputError::Component` retains expected and actual `PlaneEncoding`, covering
  floating-point as well as integer contradictions.

This is a planar RGB/gray/YCbCr/alpha interchange contract. Arbitrary named
channels, CMYK/spot/depth/CFA roles and wider scalar auxiliaries are not folded
into this layout. They retain the separate auxiliary-value path. Exact spatial
grids, independent auxiliary ownership, tiled and deep images still require
additional contract validation. A layout does not certify metadata completeness,
color conversion support, losslessness or a whole-operation memory bound.

The migration is a release candidate until the core, renderer, native adapters,
compiler API snapshots and site gates are qualified. The reproducible core gate
is `just plane-main-check`; run it in an export outside workspace overrides.

## Review still open

[Issue 198](https://github.com/imazen/zencodec/issues/198) records the remaining
API decisions: whether mapped owners must always state encoding, where direct
`Frame::map` consumers get layout agreement checks, typed interleaved F32
access, and extensible sample enums. These are not qualified by the core tests.
The concrete mixed-layout policy equality and alpha-as-gray findings have
regression controls in `zencodec-media-testkit/tests/plane_layout.rs`.

## Current pixel main review

The [2026-11-09 adaptation review](pixel-main-adaptation-2026-11-09.md)
compares the latest merged pixel contracts and current zencodec PR heads.
It records the missing current planar color/alpha declaration, the proposed
checked lending boundary and the measured gray-test migration blocker.
These are proposed API changes; existing signatures have not been altered.

## Checked current interpretation (core contract stack, layer2)

`FrameInterpretation` retains `color_context: Option<Arc<ColorContext>>` for
the samples and `alpha_mode: Option<AlphaMode>` for real alpha. `ColorClaim`
is source/container provenance, never a fallback current context. A converted
frame gets a fresh interpretation; an existing frame keeps its context.
Absence lends an unknown ColorContext, with no automatic sRGB assumption.

`Frame::map` now validates crop/render geometry, alpha association, plane
count, storage shape/type and each owner's explicit encoding before returning
a mapping. `MappedPlanes::encoding` is required; a present plane returning
None refuses. ImageOutput uses that same boundary and retains MapError as its
typed cause. Component failures carry expected/actual dimensions and encoding.
They occur at map rather than after a consumer has received a partial view.

```rust,ignore
let signal = FrameInterpretation::new(geometry, layout)
    .with_color_context(current_color)
    .with_alpha_mode(AlphaMode::Straight); // only with a real alpha role
frame.with_planar(None, |view| {
    inspect(view.layout(), view.color_context(), view.alpha_mode());
})?;
```

`with_planar` borrows plane handles, not sample copies, and retains the native
map guard throughout the callback. Its handle reservation is fallible. A view
cannot escape the callback; callers may retain the Frame itself. Pulse is
optional and checked around native mapping. The callback owns cancellation
and error precedence for work it executes. Coded extents, odd crop phase and
render size remain distinct; lending performs no implicit crop or resampling.

Opaque profiles and native backend text do not enter routine Debug/Display;
original backend errors/traces remain accessible through source/find_cause.
`Samples` and `Row` are non-exhaustive before the first release of these views.
