# Checked codec operations: target and implementation

Implementation is on [PR 168](https://github.com/imazen/zencodec/pull/168).
This is an active implementation record, not a declaration that the complete
API is implemented or stable. [spec.md](spec.md) contains current signatures;
[the comparison review](api-review-2026-10-25.md) records the verified external
precedents and the conclusions retained from Argon's review.

## What still needs production implementation

- One fallible resolver with typed, qualified offers and bounded diagnostics,
  used by ordinary request execution and by later fact changes.
- Coherent transcode routes that couple metadata obligations, explicit
  zenpixels-convert work, encoder signaling and resource admission.
- Mandatory native suspension before affected conversion, encoder or publication
  effects. A metadata notification is insufficient.
- Metadata exclusion/retention with explicit inspection scope, including opaque
  carriers, malformed XML, reconstruction data and nested content.
- Storage charges retained through actual packet/frame/cache owners, including
  old video epochs during reconfiguration, and qualified encoded staging.
- Shared image item, level and region selection with bounded discovery and
  source-version binding for the gigapixel consumer.
- A generated consumer/adapter/implementor guide for the completed surface,
  with executable examples and migration diagrams.

## Implemented on the PR

| Contract | Implementation and verification |
|---|---|
| Native standard-source errors survive erasure | `StdSourceError` retains I/O kind, OS code and custom payload; source, truncation and erased-driver tests |
| Buffered input admission precedes exact reads | Checked lookback selection, bounded retained history, exact-cap EOF probe, terminal excess and fallible retained allocation |
| Driver input allowances | `IoLimits` across sync/async incremental drivers; independent byte/call/capacity oracles, affordable alternatives, bounded batches, atomic range publication after partial failure/cancellation |
| CPU scheduling independent of output pressure | `WorkQuota` limits complete steps per async turn; inline scheduling and packet loops yield; quota yields do not drain output; exact native QOI bytes at four quotas |
| Terminal erased video errors | `FusedEncoder` protects every work method and cancellation, preserves native causes, and remains failed after a caught unwind; decoder reset still reopens its session |
| Initial stored requests | Required policy, fidelity configuration and output descriptors checked at execution; inherited defaults, rejection and strided pixels tested; native baseline/progressive JPEG encoding bytes and decoding pixels match direct calls |

Driver input capacity is a scoped allowance. It excludes source caches,
codec/output allocations, allocator bookkeeping and transient reallocation.
Existing codec memory estimates and capability flags are not whole-operation
memory guarantees. The new request layer does not strengthen those old claims.

## The consumer shape

Keep the small conveniences and registered codec templates. A stored request
adds requirements when the application needs them:

```rust
use zencodec::{CodecSet, ImageFormat, operation::{DecodeRequest, EncodeRequest}};
use zenjpeg::{JpegDecoderConfig, JpegEncoderConfig};
use zenpixels::PixelDescriptor;

let codecs = CodecSet::new()
    .with_decoder(JpegDecoderConfig::new())
    .with_encoder(JpegEncoderConfig::new());
let image = DecodeRequest::new()
    .with_output(PixelDescriptor::RGB8_SRGB)
    .decode(&codecs, input)?;
let encoded = EncodeRequest::new(ImageFormat::Jpeg)
    .encode(&codecs, image.pixels())?;
```

The request remains reusable. A failed requirement is an error, not a successful
result with a warning that callers must discover. `preferring_output` explicitly
allows the decoder's native fallback. Conversion remains an explicit operation;
the next layer must qualify its interpretation and destination signaling together.

The intended extension is one resolver invoked at meaningful decisions. It
chooses only offered, permitted routes or returns a traced refusal/error. It
cannot invent support, waive a hard invariant, authorize stale facts or resume
after a terminal failure. The adapter constructs and admits executable routes;
codecs expose facts and mechanisms without depending on an application policy.

## Compatibility plan

| Change | 0.2 / breaking-release treatment |
|---|---|
| Stored request entry points and typed resolver support | Additive candidates; new signatures stay on the review PR |
| Native source errors | Concrete error-type change in the unpublished I/O crate; migrate typed source consumers |
| Input admission and work quotas | Opt-in additive controls; legacy defaults remain compatible |
| Fused erased encoder behavior | Stricter behavior in unpublished media APIs; error-only shims preserve explicit backend retry contracts |
| Legacy best-effort setters and capability coordination | Keep during migration; only retire after the checked replacement covers their real consumers |
| Metadata preservation names and mandatory suspension | Require explicit migration and codec qualification; no silent reinterpretation of released contracts |

The goal is successful execution that establishes the requested contract, or an
actionable refusal. Completion requires native codec and pipeline evidence for
each remaining row above, not only types or synthetic protocol tests.
