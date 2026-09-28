# Native color reconstruction

`YuvView` validates component geometry, ceiling chroma sizes, storage and code
precision without scanning samples. It accepts unsigned 8–16-bit codes in U8 or
U16 storage, including explicitly shifted storage. It carries raw CICP values;
unknown primaries/transfer codes are preserved. Chroma location is separate.

`YuvToRgb` reconstructs source-encoded RGB using the signaled range and matrix.
Supported matrices are identity (GBR component order), BT.709, BT.601 (codes 5
and 6), and BT.2020 NCL. Monochrome uses the luma range independently of matrix.
Unsupported matrices fail. Matrix identity requires 4:4:4. Conversion does not
apply transfer functions, ICC profiles, primary conversion, clipping or tone
mapping. Its output CICP changes only matrix to identity and range to full-scale
float. Out-of-range source-encoded RGB remains out of range.

Reconstruction uses bilinear chroma with edge replication at the original
component boundary. This is an explicit algorithm choice, not a codec-mandated
reconstruction filter. Unknown siting on a subsampled axis fails; the caller may
explicitly provide an interpretation. AV1's vertical position maps to left
siting (horizontal offset 0, vertical offset 0.5), colocated to top-left, and
unknown/reserved to unknown. The raw AV1 code remains available on the owner.

Cropping retains absolute luma coordinates and the complete borrowed components.
An odd-origin crop therefore has the same pixels as cropping an already
reconstructed full frame, including neighboring chroma outside the crop. Nested
crops retain the same phase. Output is one caller-owned, tightly packed row of
RGB f32 triples; row bounds and output width are checked before any writes.

## References and executable evidence

- [BT.2100-3 (February 2025), tables 6 and 9](https://www.itu.int/dms_pubrec/itu-r/rec/bt/R-REC-BT.2100-3-202502-I!!PDF-E.pdf): NCL coefficients and full/narrow digital quantization.
- [Khronos DataFormat, color-model equations](https://github.com/KhronosGroup/DataFormat/blob/661f4ef60a16c428fa1ed00e2e436b96bf7c51f7/colormodels.txt) and [quantization](https://github.com/KhronosGroup/DataFormat/blob/661f4ef60a16c428fa1ed00e2e436b96bf7c51f7/quantization.txt): read from a fresh source checkout. Equations were checked against the ITU source, not copied blindly from approximate matrices.
- [AV1 color semantics](https://github.com/AOMediaCodec/av1-spec/blob/5e04f3f75e73a5898d7616c47c52f032144b8f80/07.bitstream.semantics.md): matrix codes, identity constraints and chroma positions.
- `tests/color.rs`: all 192 independent Decimal YCbCr cases at 8/10/12/16
  bits, both word alignments, preserved padding, unclipped extrema, GBR identity,
  monochrome, invalid output immutability and every nonempty suffix crop of a
  7×5 frame across three samplings and three sitings. Spatial references sum
  continuous triangle-kernel weights independently of production's integer
  phase calculation. Float output is checked within two f32 epsilon units
  scaled by the expected magnitude.

HDR transfer interpretation is a separate contract. BT.2100-3 table 5
distinguishes inverse HLG OETF (scene light) from the EOTF (display light): the
latter also needs display peak/black and luminance-dependent system gamma.
Neither native U16 storage nor CICP alone supplies an absolute diffuse white.

## Explicit display conversion

`DisplayTransfer` makes that interpretation a required argument. sRGB takes
an explicit white luminance; PQ uses its normative 10,000 cd/m² scale; HLG
takes display peak, black and system gamma. HLG includes black lift and the
BT.2020 luminance-coupled OOTF through the existing `zentone` implementation.
Its output is absolute display-linear RGB, suitable for PQ or primary conversion.
BT.1886 takes explicit white/black and uses exponent 2.4. It is a selected
display interpretation of BT.709 signaling, not the inverse camera OETF.

`DisplayConversion` accepts full-range identity-matrix RGB CICP, checks it
against both explicit transfer interpretations, uses zenpixels' existing primary
matrix, and applies the destination inverse transfer. Unknown primaries fail;
HLG requires BT.2020. Alpha bypasses this operation. The caller explicitly
chooses rejection or component clipping for unrepresentable output; clipping
is neither tone mapping nor perceptual gamut mapping. A rejected row leaves
the destination unchanged. The scalar dry run trades work for that guarantee.

Production kernels are reused from pinned `linear-srgb` and `zentone` sources;
their f32 approximations are checked against separately generated 70-digit
Decimal references. `generate_display_references.py` covers 720 display cases
and 576 primary conversions, including saturated HLG, nonzero black, and wide
gamut results below zero or above one. Decimal primary matrices use independent
Gauss-Jordan elimination from published xy coordinates. Numeric budgets are
stated in `tests/display.rs`; the vectors are not generated by the tested Rust.

[BT.1886 Annex 1](https://www.itu.int/dms_pubrec/itu-r/rec/bt/R-REC-BT.1886-0-201103-I!!PDF-E.pdf)
supplies the nonzero-black display equation. BT.2100-3 table 5 supplies HLG's
black lift, inverse OETF and OOTF; table 4 supplies PQ. The prototype exposes
gamma instead of guessing a viewing environment from CICP.

## Packed images to native video components

`RgbToYuv` accepts a zenpixels packed RGB/gray view with explicit dimensions,
byte stride, storage and full-range descriptor. It preserves transfer and
primaries and rejects unresolved ICC input. U16 packed RGB means full
0..65535; it must not disguise right-aligned 10/12-bit component storage.
The target independently supplies significant bits (8–16), matrix, range,
subsampling and chroma location. Output is owned LSB-aligned U16 components;
the `YuvView` retains every descriptor axis.

Decimation uses a triangle filter in the source encoding, with edge replication
and explicit phase: co-sited 1/4,1/2,1/4 taps or centered 1/8,3/8,3/8,1/8 taps.
This is an algorithm choice, not a normative codec filter. Quantization is
nearest with halfway ties upward, clamped to the storage code range, without
dithering. Unknown subsampled siting fails. Native AV1 can signal left or
top-left 4:2:0; arbitrary centered/4:2:2 interpretations must not be silently
advertised as those positions.

Nonopaque pixels require an explicit encoded-domain matte; the default policy
rejects them. This does not invent an AV1 alpha stream. A separate alpha-capable
container/codec route is needed to preserve alpha. Limits bound component
allocation, and conversion does not allocate a whole float RGB intermediate.
The 352 Decimal forward references and independent spatial integration exercise
code quantization, all supported ranges/depths, odd dimensions and chroma phase.

## Packed output and extracted images

`frame::to_rgb` returns a full packed RGB buffer in U8, normalized full-range U16,
or F32. It carries the resulting RGB CICP in the descriptor and ColorContext;
raw unknown transfer/primary codes remain in the context. Integer quantization
rounds to nearest with upward halfway ties, after the caller's explicit reject
or clamp choice. F32 retains finite out-of-range reconstruction values when no
display conversion is requested.

Native reconstruction stays in f64 through integer packing. Exhaustive 10/12-bit
identity ramps found an off-by-one halfway decision when an intermediate f32
rounded first. Tests compare every one of the 1,024 and 4,096 source codes to
an independent integer formula. The output budget includes packed bytes and one
f64 RGB row; codec reference pictures are additional. Conversion is cancellable
between rows and all allocations are fallible.

`video::VideoRgb` applies this conversion to a native AV1 frame. Its optional
chroma override applies only when the bitstream position is unspecified. An
optional `DisplayConversion` must match the reconstructed source CICP exactly.
`video::encode_extracted` passes these pixels to an ordinary still encoder;
selection retains its actual timestamp and ordinal on the extraction result.
