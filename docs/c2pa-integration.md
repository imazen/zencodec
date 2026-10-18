# C2PA integration direction

Use **c2pa-rs for manifest reading, construction, signing and verification**.
zenmeta's optional `audit::Report::c2pa` is a structural, unvalidated audit/diff
view for `no_std + alloc` consumers. It is not the foundation of a second C2PA
implementation. No production SDK adapter or codec preservation/signing contract
is implemented by this PR.

This plan was checked against [C2PA 2.4](https://spec.c2pa.org/specifications/specifications/2.4/specs/C2PA_Specification.html#_2_4_april_2026)
and [c2pa-rs 0.91.3](https://github.com/contentauth/c2pa-rs/releases/tag/c2pa-v0.91.3)
on 2026-10-17. Public API additions described below require their own review.

## Ownership

| Layer | Responsibility |
|---|---|
| zencodec | Opaque store sidecar, presence state, strict retention/capability negotiation and carrier reservation contracts; core + alloc only. Keep the existing Metadata layout. |
| Each codec | Extract/remove its carrier; reassemble packets; report physical ranges; reserve/patch bytes and finalize sizes, offsets, padding and checksums. No SDK dependency. |
| Proposed separate `zenc2pa` adapter | std integration with SDK Context, Reader, Builder and Signer; trust/network configuration; transactional output finalization. Keep it outside this workspace so default and all-features builds here remain SDK-free. |
| zenpipe | Resolve policy for every execution path, record operations actually performed, supply source ingredients and delegate signing to the adapter. An optional feature can enable the adapter. |
| zenmeta | Bounded structural audit/diff only. The caller retains original bytes for opaque data and forensic comparison. |

The shared opaque representation should be one complete C2PA Manifest Store root
`jumb`, including its header. Physical carrier framing and padding are separate.
SDK composed bytes are different: JPEG composition returns APP11 packets, PNG
returns a complete caBX chunk, and JXL returns the complete root box unchanged.
Do not wrap the latter again. See the SDK's
[composition dispatch](https://github.com/contentauth/c2pa-rs/blob/c2pa-v0.91.3/sdk/src/store.rs#L2647)
and [JXL composer](https://github.com/contentauth/c2pa-rs/blob/c2pa-v0.91.3/sdk/src/asset_handlers/jpegxl_io.rs#L890).

## Start with the public SDK APIs

1. Build an explicit shared `Context` with trust material and network settings.
   Read the original asset with
   `Reader::from_shared_context(&context).with_stream(format, stream)`. Retain
   `validation_results()` and `validation_state()`, not just constructor success.
2. Construct a `Builder` with the same context. Add the source using
   `add_ingredient_from_reader` and mark it `parentOf`; add actions from the
   pipeline's completed-operation journal. Do not manufacture successful source
   validation or discard failed ingredient status.
3. Initially encode an unsigned asset, then use `Builder::sign` or `sign_async`
   with the SDK's built-in handler and a separate seekable destination. This
   costs another asset read/rewrite; memory and exact pass counts need measurement.
4. Read the completed destination again with Reader and enforce the requested
   integrity/trust policy before making it visible.

The exact entry points are in the
[Reader](https://docs.rs/c2pa/0.91.3/c2pa/struct.Reader.html) and
[Builder](https://docs.rs/c2pa/0.91.3/c2pa/struct.Builder.html) APIs. `Signer` and
`AsyncSigner` already provide algorithm, certificate chain, signature callback,
reservation size and timestamp hooks. Implement those over an application's
KMS/HSM client; no private key or network client belongs in a codec. Use the
SDK's COSE, X.509 and RFC 3161 implementation rather than handwritten substitutes.
[Signer contract](https://docs.rs/c2pa/0.91.3/c2pa/trait.Signer.html).

For the first adapter, pin the investigated release and disable its defaults:

```toml
c2pa = { version = "=0.91.3", default-features = false, features = ["rust_native_crypto"] }
```

The SDK is MIT OR Apache-2.0, declares Rust 1.96.0, and defaults to OpenSSL plus
HTTP. Leave remote fetching, HTTP implementations, automatic thumbnails and
unneeded format features off; also explicitly disable runtime remote-manifest
and OCSP fetching. Supply trust updates and any required TSA client through the
application. This is a std dependency even when compiled for wasm; it does not
belong in zencodec's no_std graph.
[Features/MSRV](https://github.com/contentauth/c2pa-rs/blob/c2pa-v0.91.3/sdk/Cargo.toml#L15),
[verification settings](https://github.com/contentauth/c2pa-rs/blob/c2pa-v0.91.3/sdk/src/settings/mod.rs#L597),
[supported targets](https://github.com/contentauth/c2pa-rs/blob/c2pa-v0.91.3/docs/support-tiers.md#L47).

## Caller-embeds optimization

After the simple path works, JPEG/PNG encoders can reserve the SDK-composed
placeholder before emitting pixel data:

```text
finalize ingredients/actions
Builder::placeholder(format)
codec inserts composed placeholder and records physical ranges
encode pixels; finish all ordinary metadata/offset/size changes
Builder::set_data_hash_exclusions(ranges)  // data-hash path
Builder::update_hash_from_stream(format, finalized_asset)
Builder::sign_embeddable(format)
require exactly the reserved physical length; codec patches it
Reader validates actual final bytes; commit output
```

BMFF uses the SDK's BMFF hash/exclusion methods instead of data-hash ranges.
Neither path requires zen to implement another hash-assertion interpreter.
Never grow the reservation after hashing. An overlong signature/TSA response
aborts the unpublished transaction or triggers a complete new reservation/hash/
sign cycle. A seekable spool is the initial streaming sink; a buffer reports
`AT_FINISH`. Bytes already sent to a nonseekable transport cannot be patched.

The current SDK has limits that the integration must accommodate:

- `AssetIO` and `Context::with_io_handler` are private. Codecs cannot register
  custom handlers through the released public API; use its stream/embedding APIs
  and accept a second container scan initially.
- `sign_embeddable` is synchronous in 0.91.3. Use `sign_async` for the initial
  asynchronous path, or a synchronous transaction on an application-managed worker.
- The final padding step does not itself reject an oversized result; the adapter
  must compare lengths before patching.
- The SDK's after-sign validation is not a substitute for reading the final
  patched asset and verifying its hard binding.

Sources: [handler visibility](https://github.com/contentauth/c2pa-rs/blob/c2pa-v0.91.3/sdk/src/context.rs#L526),
[embeddable implementation](https://github.com/contentauth/c2pa-rs/blob/c2pa-v0.91.3/sdk/src/builder.rs#L3105),
[verification defaults](https://github.com/contentauth/c2pa-rs/blob/c2pa-v0.91.3/sdk/src/settings/mod.rs#L633).

## Per-format rollout

All seven families below have standard carriers in C2PA 2.4. Start with SDK
post-encode embedding, then add native reservations per tested capability.

| Format | Carrier and native reservation gate |
|---|---|
| JPEG | APP11 packets; bounded instance/sequence reassembly and exact fragment lengths. Test repeated headers and scan/restart data. [A.3.1](https://spec.c2pa.org/specifications/specifications/2.4/specs/C2PA_Specification.html#_embedding_manifests_into_jpeg). |
| PNG/APNG | caBX; before IDAT is recommended. Reserve complete chunk and CRC. Cover animation chunks. [A.3.2](https://spec.c2pa.org/specifications/specifications/2.4/specs/C2PA_Specification.html#_embedding_manifests_into_png). |
| WebP | C2PA is the last subchunk of the first RIFF. Finalize RIFF size and padding before hashing. RiffIO has a writer but no composed-manifest hook in 0.91.3; a raw `application/c2pa` bridge needs a separate prototype. [A.3.7](https://spec.c2pa.org/specifications/specifications/2.4/specs/C2PA_Specification.html#_embedding_manifests_into_riff_based_assets), [SDK handler](https://github.com/contentauth/c2pa-rs/blob/c2pa-v0.91.3/sdk/src/asset_handlers/riff_io.rs#L352). |
| AVIF/HEIF | C2PA UUID FullBox before media; reserve before finalizing iloc/other offsets. SDK computes BMFF v3 binding, including root offsets; sequences/Merkle modes need separate gates. [A.5](https://spec.c2pa.org/specifications/specifications/2.4/specs/C2PA_Specification.html#_embedding_manifests_into_bmff_based_assets), [hash computation](https://spec.c2pa.org/specifications/specifications/2.4/specs/C2PA_Specification.html#bmff-hash-computation). |
| JXL | Dedicated top-level jumb in a boxed file; bare codestream has no slot. Emit the outer C2PA store uncompressed: this SDK does not read brob-wrapped stores. Test full-root framing and insertion order. [A.3.9](https://spec.c2pa.org/specifications/specifications/2.4/specs/C2PA_Specification.html#_embedding_manifests_into_jpeg_xl), [SDK handler](https://github.com/contentauth/c2pa-rs/blob/c2pa-v0.91.3/sdk/src/asset_handlers/jpegxl_io.rs#L29). |
| GIF | C2PA_GIF application extension before the first image, bounded 255-byte subblocks. A tail append does not satisfy that placement. [A.3.8](https://spec.c2pa.org/specifications/specifications/2.4/specs/C2PA_Specification.html#_embedding_manifests_into_gifs). |
| TIFF/DNG | Tag 52545 and the specified main-IFD placement; finalize directory/strip/tile pointers before data hashing. General TIFF box hashing was removed in 2.4. SDK ARW/NEF reader support does not imply writing support. [A.3.6](https://spec.c2pa.org/specifications/specifications/2.4/specs/C2PA_Specification.html#_embedding_manifests_into_tiff_based_assets), [TIFF hashing](https://spec.c2pa.org/specifications/specifications/2.4/specs/C2PA_Specification.html#_tiff_specific_handling), [SDK writer list](https://github.com/contentauth/c2pa-rs/blob/c2pa-v0.91.3/sdk/src/asset_handlers/tiff_io.rs#L67). |

## Policy and validation gates

Proposed defaults are strip for web/privacy presets, keep-if-still-valid with an
explicit disposition for Preserve, and failure when PreserveExact cannot keep
valid provenance. Re-signing and forensic stale retention are explicit choices.
All routes must honor the policy, including identity/lossless shortcuts, metadata
rewrite and animation. An unchanged pixel result does not imply an unchanged hard
binding. Validate a candidate containing the old store before preserving it.
[Hard bindings and asset metadata](https://spec.c2pa.org/specifications/specifications/2.4/specs/C2PA_Specification.html#asset_metadata_bindings).

After an edit, create a new standard manifest with the source as a parent
ingredient and actual operations: opened, resized, cropped, orientation,
adjustedColor and transcoded as applicable. Use minimal product/version
software-agent information. Keep signature integrity, trust, timestamp and
revocation outcomes distinct. An update manifest cannot repair a broken parent
content binding. Selective redaction has its own signed protocol; it is not the
web stripping preset.
[Actions](https://spec.c2pa.org/specifications/specifications/2.4/specs/C2PA_Specification.html#_actions),
[ingredients](https://spec.c2pa.org/specifications/specifications/2.4/specs/C2PA_Specification.html#ingredient_assertion),
[update manifests](https://spec.c2pa.org/specifications/specifications/2.4/specs/C2PA_Specification.html#_update_manifests),
[redaction](https://spec.c2pa.org/specifications/specifications/2.4/specs/C2PA_Specification.html#_redaction_of_assertions).

Codec/testkit gates stay SDK-free: carrier byte equality, strict strip/unsupported
behavior, reservation footprint, malformed framing and pixel equivalence after
metadata edits. Adapter CI owns signatures, trust, timestamp failures and final
asset binding. Use pinned corpus fixtures plus deliberate covered-byte mutations.
c2patool and c2pa-rs share an engine: they are independent of zen's carrier code,
not two independent cryptographic implementations. Missing fixtures fail the
explicitly requested corpus job. Measure SDK build cost and memory for enabled
formats before claiming streaming/resource properties.
