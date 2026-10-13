# zenmeta

Shared metadata implementation for image codecs, using zencodec's interchange
types. Metadata operations do not require zenpipe, zencodecs or a pixel decoder.

The default build supports metadata/EXIF filtering and serialization, plus
rendering-preserving retention for an already parsed ISO gain map. Optional `xmp` and `audit` features enable namespace-aware XML
inspection and structural metadata reports independently. `apple` enables
MakerNote and binary-plist inspection without XML or audit. `inventory` enables
the TIFF/BigTIFF walker used with codec-specific consumption rules. All features support
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

The `xmp` feature also provides gain-map XMP parsing/generation and Google
container-directory inspection under `zenmeta::xmp::{gainmap, container}`.
JPEG APP framing and packet reassembly remain codec responsibilities.

Optional `rewrite` supplies bounded EXIF processing and fallible plan assembly
for codec-owned rewriters. Requests, services and completed plans are zencodec
types, so a codec need not expose zenmeta types in its public API.

`xmp::container::Directory` validates a complete GContainer packet and updates
secondary lengths by semantic while preserving unrelated source bytes. It
rejects ambiguous directories and missing carried payloads. Codecs still own
MPF offsets, payload layout and APP/box framing.

Basic EXIF reading, orientation, ICC/CICP and ISO gain-map interpretation remain
available in zencodec without zenmeta. New processing callers use
`zenmeta::metadata::filter` and `zenmeta::exif::{filter, retain, serialize}`.
The published zencodec writers are deprecated compatibility implementations;
these entry points delegate to that single implementation until a breaking
release can physically relocate it without a dependency cycle.

Rewrite requests carry policy, services and budgets but do not execute them.
`zenmeta::rewrite::Context::new` performs admission checks; packet processing and
bounded plan assembly also live here. `PreserveExact` can retain XMP without an
editor. Explicit XMP editing still requires a service, and keeping descriptive
bytes does not certify their privacy or make unsupported container edits valid.
