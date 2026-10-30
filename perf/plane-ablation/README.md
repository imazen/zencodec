# Private component-storage ablation

This experiment tests a replacement for separate U8/U16 code views and scalar
value views. Nothing here is a public API proposal. The seven-argument private
constructor is a test fixture, not the intended caller-facing convenience.

```rust
let plane = Plane::new(bytes, channel, byte_order, width, height,
                       row_stride_bytes, sample_stride_bytes)?;
let row = plane.row(y)?;
let native_u16: &[u16] = row.typed::<u16>()?;
let code = plane.codes(encoding)?.code(x, y)?;
```

Storage has no color or grayscale implication. Numeric code depth/shift remains
in the existing canonical `SampleEncoding`, separate from storage. Component
roles, sampling grids and current color interpretation must travel in the owning
image's immutable interpretation; this experiment does not implement that owner.

One byte-backed view carries planar, semi-planar and interleaved components,
unaligned scalar storage, floating NaN payloads and signed/wide integers. Typed
borrowing checks exact scalar type, native endianness, contiguous samples and
alignment; it never silently copies or normalizes. Byte access remains possible
when a typed slice cannot be borrowed. U16 code extraction handles declared
endianness explicitly. F16 bytes can be carried, but no typed half-float accessor
is implemented. Generic foreign-endian float value loading is not implemented.

## Reproduce the small model

Export outside workspace path overrides and run `just plane-ablation-check`
under the resource wrapper. The committed `dependencies.lock` records the exact
standalone graph. The model passes tests, strict all-target Clippy and WASM.

## Reproduce the real AV1 mapping witness

Use a core source export containing this experiment and a separate zenavif
export of `94116d266299f347cfd10a75d440a5bc3b598d2c`.

1. Apply `native-zenavif.patch` in that zenavif export. It changes only sample
   encoding imports and U8 construction; no decoding or encoding behavior.
2. Append a `[patch."https://github.com/imazen/zenavif"]` table to the exported
   `perf/av1-policy-validation/Cargo.toml`, setting `zenavif = { path = "..." }`
   to the zenavif export. Keep the existing root/pixel/rav1d patches and pins.
3. Copy `native-dependencies.lock` to the harness's `Cargo.lock`, then run locked
   metadata. Verify one copy each of core, I/O, media and pixels. Against the
   ordinary harness lock, the only package source changes are zenavif and
   zenavif-parse from the same git tree to its patched export; versions do not move.
4. Run `just plane-ablation-native-check` under the resource wrapper.

The native witness encodes and decodes 17x13 AV1 frames at 8/10/12 bits in
4:2:0 and 4:4:4. After decoder drop it maps retained frames and checks every
row's pointer, raw samples and extracted codes against the actual native view.
It does not repack rows. This qualifies representation preservation, not AV1
codec correctness: no third-party decode comparison is claimed for this gate.
The other 52 native harness tests are not rerun by the focused test filter;
all-target Clippy compiles the complete harness.

## Outcome and limits

The byte-backed direction removes the need for a storage enum variant per scalar
type and permits per-plane representation. Native AV1 mapping needs no sample
copy. This is evidence for moving a shared storage primitive into zenpixels;
no zenpixels public changes are made yet.

The next contract cut must still reconcile `Picture`/`MappedPlanes` with
`ImageOutput`/`MappedImage`, separate presentation timing from sample ownership,
and bind per-component encodings and grids. A byte view alone does not solve
odd-crop chroma phase, mixed-channel color interpretation, signed placement,
metadata policy, mutable views, decoder mapping failure or hardware mapping.
TIFF/JXL native integration, performance and compile-time comparisons remain
unmeasured. See [the release ablation matrix](../../docs/frame-plane-ablation.md).

## Exact component grids (second experiment)

`grid.rs` keeps registration separate from the storage view. The center of
sample `i` is `(origin + i * step) / denominator` in stored-image reference
coordinates, with reference sample zero centered at zero. Integer and fractional
origins are signed; spacing is positive. Fractions normalize exactly and
unrepresentable arithmetic refuses. `Grid::Unknown` never becomes an assumed
centered grid just because dimensions match or a crop is applied.

```rust
// Centered 4:2:0: centers are 0.5, 2.5, 4.5, ... in luma coordinates.
let chroma = Axis::new(1, 4, 2)?;
// Luma crop starts at x=1, while the borrowed chroma starts at sample zero.
let cropped = chroma.rebase(0, 1)?;
assert_eq!(cropped.center(0)?, (-1, 2)); // -0.5, not a new +0.5 origin
```

`Plane::window` borrows sample rectangles without changing their row/component
strides or values. Grid rebasing accounts independently for the chosen first
component sample and the change in image reference origin. It does not select
filter halos, resample, clip off-canvas data or apply orientation. Those are
separate operations with their own requirements.

All 12 standalone model controls pass, including exact crop composition,
negative offsets, normalization, unknown preservation and arithmetic limits.
The native AV1 matrix additionally checks every cropped sample and exact center
coordinate after the odd luma crop (1,3), retaining all samples from the chosen
component origin. The native focused gate runs 13 tests; its other 52 tests are
not rerun. Both strict Clippy gates and standalone WASM pass.

This chooses a coordinate convention, not a finished public grid API. Resize
registration, arbitrary affine grids, independently oriented auxiliary planes,
TIFF/JXL native adoption and a renderer using these grids remain unqualified.

## Unified owner/presentation experiment

`../av1-policy-validation/src/component_owner.rs` implements one private owner
and mapping contract for packed pixels and component planes. Frames add timing
and presentation flags around that image; they do not define another plane API.

```rust
let mapping = frame.image.map(None)?; // Option<&dyn Pulse>
for index in 0..mapping.plane_count() {
    let plane = mapping.plane(index)?.expect("declared plane");
    // plane.meaning is frozen; plane.samples is Packed or Component.
}
let replay = frame.represent(next_timing, Presentation::SHOW_EXISTING);
```

Frozen plane descriptions can be inspected before mapping. A native missing
plane, changed dimensions/representation or backend error is a typed failure,
not end-of-iteration. Errors and unwinds poison that mapping guard before a
second backend call can occur. Backend causes remain downcastable while default
Display/Debug omit arbitrary backend text. A new image mapping attempt is
independent; the image itself is not permanently poisoned by a transient map
failure. Returned samples borrow the mapping, which borrows the image owner.

The real AV1 owner test retains decoded 8/10/12-bit pictures after decoder drop,
re-presents them without changing sample ownership, verifies every component
code and proves that the last presentation releases the native picture. Packed
pixels use the same mapping interface. Optional Pulse is forwarded to native
mapping hooks; wrapper checks precede effects and successful exposure. A stopped
entry control uses a backend that panics if mapping is attempted.

The final focused gate passes 18 tests and strict full-harness Clippy. This is
not a completed replacement for ImageOutput/Picture. In particular:

- The prototype omits full image color/geometry/composition/source metadata;
  component storage facts do not replace those contracts.
- It has one owner per image. Existing independent auxiliary retention must be
  preserved, either through owner groups or a retention hook, before promotion.
  Replacing color must not accidentally retain a large unrelated color allocation.
- Its compatibility AV1 bridge adds a boxed mapping wrapper and constructs a grid
  vector per map. These are experiment adapter costs, not the intended native
  implementation; no allocation or speed equivalence is claimed.
- Only entry cancellation is qualified. Persistent/one-shot cancellation,
  reporting, stalled native mapping, and per-plane cancellation still need controls.
- The experiment validates semantic representation, not fixed physical strides
  across remapping. It does not authorize weakening existing production guards.

The direction is one image mapping abstraction plus presentation, with complete
metadata and independently retainable allocations. The exact private types and
constructors above are not ready to become public signatures.
