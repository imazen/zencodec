# Color emission model (grounded design)

Status: **canonical.** This records the *minimal* shared color surface and — just
as importantly — the designs that were tried, dogfooded, adversarially reviewed,
and **rejected**, so they don't get rebuilt. The measured reader behaviour that
sets the defaults (which readers honour ICC, CICP, `nclx`) is in
imazen/zencodec#132.

## Thesis

The only thing that genuinely needs to be **shared** across codecs is a *pure
color-carrier policy*: given a source's color (`SourceColor`) and a target's
capabilities (`EncodeCapabilities`), decide which carriers to write (ICC vs
CICP). Everything else — pixel+metadata materialization, specialized
coefficient-domain transcodes, the decode→re-encode orchestration — already has
a home and must **not** be pulled into a grand "emit model" or a cross-codec
trait.

This was reached the hard way: an over-built `EmitFacts`/`EmitIntent`/`EmitPlan`
"scenario" model + a `TranscodeEncoder` trait were dogfooded into 5 codecs and
adversarially reviewed; the review + a full read of zenpixels/zenpipe killed
them (see *Rejected designs*). The grounded surface is ~360 lines with **zero
codec dependencies**.

## The shared surface — `zencodec::color`

```rust
pub fn resolve_color_emit(
    source: &SourceColor,
    current: &zenpixels::ColorSpace,
    target: &EncodeCapabilities,
    policy: ColorEmitPolicy,
) -> Result<ColorEmitPlan, ColorEmitError>;
```

The active color model determines ICC class. RGB and gray synthesis have
separate dispositions. Gray with alpha uses the same synthesis route as gray
without alpha. ICC profiles carried from the source must describe the current
model and interpretation. For converted pixels, pass their current color
space along with the untouched source provenance.

### Capabilities

- `icc()` and `cicp()` declare the available carriers.
- `cicp_is_valid_carrier()` admits CICP under the default policy.
- `cicp_safe_sole_carrier()` permits omitting a redundant ICC under Balanced.

## Lowering the plan

Preserve `KeepSource` bytes. Use
`SynthesizedIcc::for_cicp` for `SynthesizeFrom` and
`SynthesizedIcc::for_gray_cicp` for `SynthesizeGrayFrom`. The converter's
`icc-db` feature supplies the full supported export grid. A caller must check
the result before writing the header. If synthesis is unavailable and emitted
CICP cannot express the current color, refuse or require an explicit conversion.

HLG has no complete generic ICC export. The resolver preserves representable
native CICP where supported, and `IccRetention::Drop` prevents synthesis.
ICC profiles describe full-range reconstructed samples, so copying raw coded
matrix or narrow-range samples requires a suitable carrier or a conversion.

## Orientation (separate, tiny)

The double-rotation hazard (a decoder bakes orientation upright but the embedded
EXIF blob still says `Rotate90`) is closed by
`helpers::set_exif_orientation(blob, value)` — an offset-preserving inline rewrite
of the 0x0112 tag. It's applied by the **pipeline**, which knows when it baked
orientation. It is *not* part of color policy and not a "unified plan".

## Transcodes (pairwise, self-contained — not shared)

Specialized lossless/coefficient transcodes are **not** a generic capability:

- **JPEG → JPEG** (orient / recompress): entirely inside zenjpeg
  (`zenjpeg::lossless`, `zenjpeg::recompress`).
- **JPEG → JXL** (lossless embed): inside jxl-encoder via jbrd (its own
  `JpegData` parser — the JXL spec's recompression feature, **needs no zenjpeg**).

These preserve metadata verbatim, so they don't even call the color resolver.
The set of real pairs is tiny and well-known. The **dispatch** belongs in
**zenpipe**, which already depends on every codec — a small finite table of known
pairs calling those functions directly, plus `resolve_color_emit` on the
decode→re-encode path. No codec ever learns about another.

zenpipe already has the sketch: `try_lossless_jpeg` (in `lossless.rs`, currently
only called from tests) is the precedent to wire and generalize. **That's a later
piece**, tracked separately.

## Rejected designs (do not rebuild)

- **`EmitFacts { Fresh | Decoded | Passthrough }` + `PixelFidelity`** — nothing in
  the pipeline produces a `ColorOrigin`/fidelity: decode attaches no color to the
  buffer; provenance lives in `ImageInfo.source_color` and the carrier is a flat
  `Metadata`. A codec `Encoder` only ever sees a flat `Metadata` (via the
  `with_metadata_policy` → `with_metadata` path), so it could only ever build
  `Fresh` — the scenario machinery was dead code. The
  `PixelDescriptor` already *is* the current gamut, so deriving `Reauthored` was
  redundant. `resolve_color_emit(&SourceColor, …)` takes the type that flows.
- **`TranscodeEncoder` trait in zencodec** — a generic "output codec transcodes
  from source-format X" trait forces every output codec to *ingest* every input
  format (JXL←JPEG needs JPEG parsing; PNG←? needs zenpng; …) → **every codec
  depends on every other codec**. The real pairs are ~2 and each self-contained.
  zenpipe (deps-all) dispatches; no trait.
- **`EmitIntent` unifying color + metadata + orientation into one knob** —
  aesthetic, not grounded. `MetadataPolicy` (#17) and `ColorEmitPolicy` are fine
  apart; orientation is a one-helper correctness fix, not a policy axis.
- **A resolver that produces final `Metadata` bytes** — a third metadata producer
  alongside `Metadata::filtered` and `OutputMetadata`. The plan stays a pure
  description; the codec writes the carriers (see "Lowering the plan" above for
  why there is no atomic pixels+colour step on the encode path today).

## What landed (the surviving red-team fixes)

The 5-codec dogfood + adversarial review found real defects; the ones that
survived the grounding, all small and all on the `resolve_color_emit` shape:

1. `ColorEmitFields::new` / `CicpEmission` are constructible → `ColorEmitPolicy::Custom`
   is actually reachable downstream.
2. `cicp_is_valid_carrier` tier → PNG/WebP emit cICP under Balanced instead of
   laundering wide-gamut color through a synthesized ICC.
3. No redundant `SynthesizeFrom(sRGB)` (the canned table returns `None` for sRGB).
4. `set_exif_orientation` for the double-rotation hazard.

The `SynthesizeFrom`-silently-drops-color critical dissolves under lowering —
`icc_profile_for_primaries` always materializes a non-sRGB profile, never a CMS,
never a silent drop.
