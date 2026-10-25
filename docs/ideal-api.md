# Checked codec operations: target and implementation

Implementation is on [PR 168](https://github.com/imazen/zencodec/pull/168).
This is an active implementation record, not a declaration that the complete
API is implemented or stable. [spec.md](spec.md) contains current signatures;
[the comparison review](api-review-2026-10-25.md) records the verified external
precedents and the conclusions retained from Argon's review.

## What still needs production implementation

- Extend the fallible resolver beyond prepared encoder configuration, including
  later facts that require a new decision before affected output.
- Coherent transcode routes that couple metadata obligations, explicit
  zenpixels-convert work, encoder signaling and resource admission.
- Qualify native codecs against the implemented controlled suspension protocol
  before affected conversion, encoder or publication effects.
- Metadata exclusion/retention with explicit inspection scope, including opaque
  carriers, malformed XML, reconstruction data and nested content.
- Integrate managed storage ownership into policy waiting, conversion scratch
  and qualified encoded staging; qualify native codec/service storage separately.
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
| Initial stored requests | Required policy, fidelity configuration and output descriptors checked at execution; controlled sessions use the same registry templates and reject required-descriptor mismatch before adoption; inherited defaults, rejection and strided pixels tested; native baseline/progressive JPEG encoding bytes and decoding pixels match direct calls |
| First executing resolver domain | Typed prepared encoder offers, configurable built-in policy, fallible application callback, scoped choices and bounded reports; stale/nested tokens, cancellation, unwind cleanup and native error traces tested |
| Controlled interpretation protocol | Explicit suspensions and scoped replies; fused enforcement, coroutine await, and a shared policy adapter for existing transports. Authored controls cover early output, stale replies, finality changes, cancellation and unwind. Native baseline/progressive JPEG qualification covers whole-source, header-authoritative decoding; unqualified late ICC refuses before native construction. Failed sessions release their native storage while retaining reports |
| Managed storage owners | Byte/object admission follows allocations through subviews, packets, side data and old-epoch frames; exact strided 8/10-bit samples and concurrent contention checked. [Scope](storage-admission.md) excludes native workspace and explicit unaccounted copies |

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

The resolver now selects prepared encoder jobs. A caller can use
`preferring_fidelity(...)` and `with_policy_set(PolicySet::new().prefer_lossless(true))`,
or pass a `PolicyResolver` to `encode_with`. Required targets never become
preferences. The selected job is the same object that was checked before the
callback, and execution errors do not trigger a retry. The returned `Outcome`
contains both output and a bounded decision report.

The extension to other domains must preserve that execution link. The resolver
chooses only offered, permitted routes or returns a traced refusal/error. It
cannot invent support or authorize stale facts. Adapters construct and admit
executable routes; codecs expose facts and mechanisms without depending on an
application policy. Late metadata and resource decisions are still open work.

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
