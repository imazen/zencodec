# Retaining a decoded result across adapters

Consumers normally keep `DecodeOutput` intact:

```rust
let decoded = request.decode(&codecs, &input)?;
use_pixels(decoded.pixels());
use_metadata(decoded.metadata());
```

An adapter may need to move pixel storage separately while retaining everything
else. `into_buffer()` deliberately discards the rest. The review proposal adds:

```rust
let (pixels, details) = decoded.into_parts();
// Move both owners through the adapter. No metadata or extension clone is needed.
let decoded = DecodeOutput::from_parts(pixels, details);
```

The complete added surface is:

```rust
#[non_exhaustive]
pub struct DecodeDetails {
    pub info: ImageInfo,
    pub source_encoding: Option<Box<dyn SourceEncodingDetails>>,
    pub extensions: Extensions,
}
impl DecodeDetails {
    pub fn new(info: ImageInfo) -> Self;
}
impl DecodeOutput {
    pub fn into_parts(self) -> (PixelBuffer, DecodeDetails);
    pub fn from_parts(pixels: PixelBuffer, details: DecodeDetails) -> Self;
}
```

`from_parts` has the same raw-constructor responsibility as `DecodeOutput::new`:
the details must describe the corresponding samples. Neither constructor performs
conversion, validates completeness, or updates provenance after processing.
Existing consumer signatures and the existing DecodeOutput representation are
unchanged. `into_buffer` remains useful when discarding details is intentional.

## What this establishes

The ownership test requires each source-detail and extension owner to drop once,
requires Send, and checks pixel-storage identity after reassembly. Native JPEG
baseline/progressive controls retain exact metadata and the original addresses of
pixels, boxed source details and native DecodedExtras. No descriptor negotiation
or native decode behavior changes. The API snapshot contains only the added type,
fields and methods.

The focused core output tests, five native admission/ownership tests, strict core
and native-harness Clippy, no-default-features build and API regeneration pass
(rc0, 13s, peak RSS 0.59 GiB). This is wrapper RSS, not a native memory bound.
An initial native test import used the retired private `decode` module; corrected
tests import public `zenjpeg::decoder::DecodedExtras`. That compilation failure
is retained in the qualification log.

## What it does not establish

This is the ownership bridge, not a streaming completion implementation.
Incremental decoders still need a terminal result channel carrying these details
with explicit completeness evidence and single-take/error/drop behavior. Native
JPEG controlled decoding does not yet return the owned extras carried by its
one-shot path. No HDR gain-map reconstruction parity is claimed by these fixtures.

An empty details object is not proof that opaque metadata is absent. Privacy and
retention requirements still need native evidence; no fallback may construct empty
details and claim complete discovery. Strict early-output admission remains open.
See [the admission audit](decode-descriptor-admission.md).

## Guide qualification

PR CI caught an omitted signature-catalog regeneration. The corrected eight-crate
catalog passes generation (rc0, 6s, peak RSS 0.26 GiB). Site/source controls and
the full browser suite pass on the candidate: all role tabs, inline context,
copy/search, signatures, keyboard navigation and 390/768/1440-pixel layouts
(rc0, 41s, peak RSS 0.33 GiB). No browser assertion changed.
