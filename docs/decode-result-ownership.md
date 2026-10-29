# Retaining a decoded result across adapters

Consumers normally keep `DecodeOutput` intact:

```rust
let decoded = request.decode(&codecs, &input)?;
use_pixels(decoded.pixels());
use_metadata(decoded.metadata());
```

An adapter may need to move pixel storage separately while retaining everything
else. `into_buffer()` deliberately discards the rest. The ownership API provides:

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

The optional `IncrementalDecode::take_details(&mut self) -> Option<DecodeDetails>`
now carries these owners after successful completion. `Fused` exposes them only
after `Done` and delegates at most one take, including through error erasure.
Before completion and after failure/cancellation it does not call the native
getter. Older decoders default to `None`; this means unavailable, not empty.
Box/reference adapters, CodecSet selection, checked decode and PolicyDecoder
forward the hook. Consumers needing owned details must reject `None`.

The earlier transport witness moves an actual native one-shot result through a
private row adapter and public erasure; it still deliberately starts with a full
raster. The pinned JPEG controlled producer now also supplies actual terminal
details from its retained metadata scan. Baseline/progressive consumer controls
compare descriptive metadata, native segment bytes, source-detail presence and
every output row through DecodeRequest, CodecSet and PolicyDecoder. Native
controls additionally retain late comments and exact embedded MPF JPEG bytes.
This does not qualify HDR gain-map reconstruction or a whole-operation RAM bound.

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

## Terminal extraction for adapters

Ordinary consumers keep calling `DecodeRequest::decode`. An incremental adapter
that needs the complete owned result uses this sequence:

```rust,ignore
// After the driver has returned Event::Done successfully:
let details = decoder.take_details().ok_or(MyError::DetailsUnavailable)?;
// Move the finished storage; do not copy the collected raster.
let pixels = sink.into_buffer().ok_or(MyError::PixelsUnavailable)?;
let result = DecodeOutput::from_parts(pixels, details);
```

The optional hook avoids requiring every existing codec to implement completion
immediately. It does not make partial header information sufficient. A provider
returning Some must retain all result details its configured operation produced;
this still does not assert discovery of every opaque input carrier. Late privacy
or retention decisions need an admitted publication owner holding affected output
until those obligations are discharged. The getter does not retract live output.

Terminal-hook qualification: 673 core tests and 144 IO tests/doctests passed,
with strict core/IO Clippy and no-default-features build. That run then stopped
at an obsolete root `OutputInfo` import in the new native witness. Correcting
only that import to `decode::OutputInfo` gave six passing native tests, strict
harness Clippy and regenerated API snapshots (rc0, 10s, peak RSS 0.59 GiB).
The initial failing log is preserved. The core terminal controls deliberately
use an eager/repeating provider to prove the Fused gate, plus native-error and
cancellation paths that must not invoke the getter.

## Coroutine producer boundary

`CoroutineContext::set_details` stores owned details privately. Successful future
completion is insufficient: row delivery and sink finalization must also succeed.
Only then does `take_details` expose the owner. Native errors, cancellation,
invalid resume, and producer/sink unwind discard it; the decoder stays terminal.
No new Arc clone or heap allocation is introduced by the stack cleanup guard.
This is a source property, not a measured throughput or total-memory claim.

The producer tests found that sink.finish panic previously left a coroutine
resumable. Finalization now poisons state before callbacks. All 42 coroutine
tests, strict all-target Clippy and no-default-features build pass.

## Native completion qualification

JPEG `7c5a9501` retains the metadata scan owner, rather than reconstructing empty
extras from an early header or decoding pixels twice. Its 20 controlled tests,
18 MPF bounds tests, strict Clippy and no-default-features controlled build pass.
The shared harness passes seven admission/ownership tests and strict Clippy
(rc0, 13s, peak RSS 0.70 GiB). The pin/lock audit changes only zenjpeg and zenyuv.
Wrapper RSS measures build/test processes, not codec working memory. Encoded
input, native progressive storage and metadata allocations retain their existing
scope; this is not an early one-shot allocation-admission fix.

## Collecting without another raster

`CollectSink::into_buffer(self) -> Option<PixelBuffer>` moves the finished raster.
The collector owns its aligned PixelBuffer from begin, while preserving fallible
Vec reservation and the original allocation error cause. Existing `to_buffer`
continues to provide copying snapshots. Neither method proves decoder success;
use the driver result before extracting the finished image.

Pointer-identity controls cover U8, U16, F32 and the real JPEG controlled route
through final DecodeOutput reconstruction. Eight IO unit tests, 17 driver tests,
seven native admission/ownership tests, strict Clippy and no-default-features
checks pass (rc0, 15s, peak RSS 0.59 GiB). Existing capacity-refusal assertions
are unchanged. The owner uses the existing zenpixels API: zero zenpixels API
changes. Alignment slack is less than one sample alignment; native coefficients,
metadata, full encoded input and sink budgeting retain their existing scope.
No throughput or process-memory reduction is inferred from pointer identity.
