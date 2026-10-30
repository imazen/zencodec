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
