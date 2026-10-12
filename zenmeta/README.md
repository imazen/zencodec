# zenmeta

Shared metadata implementation for image codecs, using zencodec's interchange
types. Metadata operations do not require zenpipe, zencodecs or a pixel decoder.

The default build supports rendering-preserving retention for an already parsed
ISO gain map. Optional `xmp` and `audit` features enable namespace-aware XML
inspection and structural metadata reports independently. All features support
`no_std + alloc`; no unsafe code is used.

```rust
use zencodec::{GainMapParams, Metadata, MetadataPolicy};

let metadata = Metadata::none();
let params = GainMapParams::default();
let retained = zenmeta::display::filter_for_gain_map(
    &metadata, &params, &MetadataPolicy::PreserveExact,
)?;
assert_eq!(retained, metadata);
# Ok::<(), zenmeta::display::Error>(())
```

An audit reports its coverage limits and opaque content. It does not certify
privacy. Container framing, packet reassembly and rewriting belong to codecs;
an inventory is not a safe generic deletion recipe.

See [source provenance](PROVENANCE.md) and the workspace
[design](../docs/zenmeta-design.md).
