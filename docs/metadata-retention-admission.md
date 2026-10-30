# Filtering is not encoder retention admission

```rust
let job = config.job()
    .with_metadata_policy(metadata, MetadataPolicy::PreserveExact);
// The record was filtered. This setter does not prove native retention.
let encoded = job.encoder()?.encode(pixels)?;
```

The provided setter calls `Metadata::filtered` and then the native metadata
storage primitive. It returns no admission result. A codec can omit a carrier
it does not support, even after the caller supplied PreserveExact. The dyn setter
has the same limitation. This is separate from metadata privacy filtering and
from whether the pixels themselves support HDR.

## Executed JPEG/PNG witness

`just metadata-retention-probe FRESH_DIRECTORY` uses actual native encoders and
decoders on 97×65 RGB8 samples. Each request supplies CLL, mastering-display
metadata, or both, with PreserveExact. Empty-metadata controls use identical
configuration and samples. No capability flag suppresses a check.

| Codec | Requested fields | Native readback | Bytes identical to empty control |
|---|---|---|---|
| JPEG | CLL | absent | yes |
| JPEG | mastering display | absent | yes |
| JPEG | both | absent | yes |
| PNG | CLL | retained | no |
| PNG | mastering display | absent | yes |
| PNG | both | CLL only | no |

The PNG both-fields output is also byte-identical to its CLL-only output.
All six encodes succeeded. The witness exits nonzero because five routes do not
establish retention. It does not accept an arbitrary encoder error as a qualified
unsupported result. The unchanged native graph is pinned by
`perf/jpeg-policy-validation/dependencies.lock`: JPEG 7c5a9501, PNG 7b6d1436,
pixels f23ac753. This does not characterize newer native revisions or every
metadata carrier. PNG's positive case is native readback, not an independent
third-party decoder qualification. Identical-byte negative controls do not rely
on decoder interpretation to establish that the supplied field changed no bytes.

The caller-selected artifact directory retains all eight encoded files and
refuses to overwrite prior artifacts. CLL compares both values exactly; mastering
display uses the testkit's existing fixed-point tolerances. No tolerance was
loosened. No original conformance assertion or native implementation changed.

## Replacement contract to implement and review

```rust,ignore
// Proposed; these methods are not implemented yet.
let job = config.job()
    .try_with_metadata_policy(metadata, MetadataPolicy::PreserveExact)?;
let encoded = job.encoder()?.encode(pixels)?;
```

Use one atomic checked setter in the typed job, a provided consuming builder,
and dyn forwarding. Default implementations must refuse rather than infer
retention from static capabilities. Rejection leaves an existing mutable job
unchanged. A default cannot even promise to clear previously configured metadata.
Names should mirror `try_set_policy` / `try_with_policy`; an independent
capability-query choreography must not become the caller's responsibility.

The retained job must carry any obligations depending on the later pixel/color
declaration into planning and startup. Setter success is not proof of facts not
yet available. Validate those obligations before a header or other affected
output is committed. Native failure remains possible and retains its typed cause.
A callback can select a permitted adaptation, but cannot waive explicitly
required retention. An adapter must qualify the actual retained configured job,
not build another one from a capability report.

Do not merge PR158's conditional preservation checks as-is. Separate its native
HDR-pixel fixture from metadata admission. Keep supplied-metadata tests requiring
verified retention or explicit unsupported/refusal, including false/unknown
capability declarations and silent-success mutants. Native HDR pixel support is
not a guarantee of CLL/MDCV carriers, and a false carrier flag is not permission
to silently disregard a required field. Deprecate the old best-effort setters
only when the tested replacement is available; a renamed filter alone would not
fix this contract.

This design does not certify opaque ICC/XML as PII-free, preserve unreported
container metadata, or solve general deferred obligations. Those remain scoped
requirements of the retained publication owner and issue166.
