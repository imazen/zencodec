# Shared metadata implementation: zencodec and zenmeta

Design recorded 2026-10-12. See the [published API audit](zenmeta-published-api-2026-10-12.md)
for release evidence and the [source/test migration map](zenmeta-migration.md) for
the existing implementations this design reuses. This document describes the
target; it does not claim that the migration is implemented.

## Decision

Add one crate, `zenmeta`, for metadata implementation that can evolve separately
from codec contracts. Keep a useful standard-metadata baseline in `zencodec`.
Neither metadata operations nor inventory require `zenpipe` or `zencodecs`.

The dependency direction is:

```text
codec -----> zencodec -----> established pixel/color dependencies
  |
  +-------> zenmeta -------> zencodec
```

Do not introduce a second new `zenifd` or `zenmeta-core` package. The proposed
zenifd code is a source of reusable modules, not a package that must be published.
Do not make zencodec depend on zenmeta, including through an optional feature.

## Useful without zenmeta

zencodec keeps the code and types already needed for normal image interchange:

- ICC payloads, CICP, color resolution/emission and luminance signaling;
- orientation and its EXIF parsing, writing, and reconciliation;
- ISO 21496-1 gain-map parameters, validation, parsing and serialization;
- raw EXIF/XMP transport;
- the published `Exif`, policy, filtering, and encoder-policy APIs;
- shared inventory types and the inventory operation contract.

Keeping the published EXIF APIs avoids an unnecessary breaking release. Their
existing implementation is the baseline to reuse, not code to freeze and copy
into zenmeta. New common IFD primitives must serve a demonstrated consumer and
preserve the existing parser's accepted inputs and malformed-input behavior.

The orientation helper currently delegates to `Exif::parse`. Do not replace it
with another independently maintained scanner merely to call it lightweight.
If extracting a bounded entry reader improves the baseline, make the existing
EXIF parser, orientation helper, and richer consumers use it and carry over their
tests. Standard low-level helpers may be public implementation support in
zencodec; that surface still needs deliberate API review.

Basic metadata availability does not imply pixel color conversion or gain-map
rendering lives in zencodec. Those stay with their established implementations.

## What zenmeta owns

Use modules and feature boundaries for:

- namespace-aware XMP inspection and supported serialization;
- Apple MakerNotes, binary plists and subsequent vendor handlers;
- richer EXIF inspection/editing built on zencodec's baseline;
- metadata audit, structural diff, coverage findings and opaque-data reporting;
- rendering-preservation checks before filtering;
- shared TIFF/IFD inventory traversal and other reusable inventory helpers.

The general XML/RDF parser representation, vendor implementation details and
traversal work structures belong to zenmeta. A codec needing only Apple
MakerNotes must not need XML, audit reports or TIFF inventory traversal.

The initial feature plan is default-empty, with independent `xmp`, `apple`,
`audit`, and `inventory` modules. Resolve their exact dependency edges from the
code being moved. An `xmp` parser must not automatically enable audit work;
an audit feature must not force XML when only EXIF is inspected. Keep
`no_std + alloc` support and forbid unsafe code.

## Public type ownership

Types crossing generic codec interfaces belong to zencodec or an established
owner such as zenpixels. zenmeta can construct those types. A codec may re-export
zencodec-owned metadata types without binding its public API to zenmeta.

Check every exposure route: function signatures, fields, enum payloads, type
aliases, re-exports, associated types, trait bounds and returned iterator items.
Do not expose a zenmeta parser or error type through one of these routes.

Private codec dependencies may adopt different compatible or incompatible
zenmeta releases independently, provided their zencodec interchange types unify.
Direct users of advanced zenmeta APIs deliberately accept zenmeta's versioning.
Behavioral compatibility still requires codec regression tests.

Do not change the identity of published types merely by replacing their
definitions with re-exports. In particular, ultrahdr-core's published
`AppleHdrInfo` and recursive `PlistValue` need compatibility treatment. Preserve
their original public definitions and convert internally when delegating to a
new implementation. This must not create a second parser.

## Inventory belongs to the configured decoder

Inventory describes what the selected decoder and backend do with the file under
the selected job settings. Keep it on the configured decode job; do not maintain
a separate copy of metadata policy, image-selection or gain-map settings.

Each codec gets a default-off Cargo feature named `inventory`:

- gate the full walker module and inventory-only dependencies;
- gate inventory-specific test files, examples and fuzz targets at their callers;
- keep the method and shared types available in zencodec;
- with the feature off, return an explicit unsupported-operation error;
- with it on, return the inventory or an operation error;
- capability reporting must agree with both configurations;
- normal decode never constructs an inventory, formats inventory labels or
  performs exhaustive traversal on behalf of inventory.

The Option-returning inventory methods are absent from zencodec 0.1.26. Their
unreleased signatures can be revised before publication. Prefer an actual
`Result<Inventory, Error>` contract over `Ok(None)` for unavailable support.

There is one signature constraint to respect when implementing that change:
`DecodeJob::Error` only requires `Error + Send + Sync`; the trait cannot construct
an arbitrary implementation's error in its default method. Do not add a required
constructor or a new associated-type bound to the published DecodeJob trait.
The recommended generic signature uses the already published shared error
envelope:

```rust,ignore
fn inventory(&self, data: &[u8]) -> Result<Inventory, At<CodecError>>;
```

Provide a default returning a new `UnsupportedOperation::Inventory` variant
inside the envelope. Retain `BoxedError` on the dynamic trait, matching its
existing conventions, and preserve the envelope as the error's concrete cause.
Keep the unsupported operation recoverable through both category inspection
and the existing cause-chain helpers. Neither default needs a bound on the
codec's associated Error type.

This chooses an existing common envelope over another public generic
`InventoryError<E>` wrapper. Codecs with native errors can preserve those as the
envelope's detail; already-located errors keep their trace. The choice is a
proposed signature, not an implemented or compiler-verified API change. Verify
it with a 0.1.26-style decoder that implements no inventory method and whose
native error cannot construct an unsupported-operation error. The default must
return an error, not panic.

A separate inventory trait is not required to save default-build compilation:
feature-gating the implementation does that. Consider a separate trait only if
measurements justify the additional dispatch/capability complexity.

Cargo features are additive. Default-off means a direct default dependency does
not select inventory; another dependency can enable it for the same package in
the resolved graph. Do not promise a per-caller ability to disable a feature
already enabled elsewhere. Check the actual resolved graph in minimal consumers.

The `inventory` feature controls implementation availability, not the user's
decode settings. Existing limits and stop tokens still apply. Malformed input
must produce a validated inventory with malformed/unknown parts, following the
existing contract; errors are for unsupported operation and operational failure.
Do not introduce an ordinary decode path that silently falls back to inventory.

## One parser, explicit interpretations

Share boundary and field-reading mechanics. Keep acceptance rules explicit:

- decode follows its existing tolerant or strict behavior;
- inventory describes those choices and accounts for all bytes;
- inspection exposes known values and opaque/malformed content;
- rewriting validates enough structure and dependencies to produce valid output.

The parsed EXIF tree is not an exhaustive byte map. The proposed `Exif::entries`
iterator omits skipped malformed entries and structural pointers. Shared entry
reading must retain ranges, duplicate occurrences, unreadable values, unknown
types and unresolved references so inventory does not infer coverage from the
salvaged EXIF tree.

Do not silently replace a tolerant decoder's ICC/XMP/MPF behavior with a strict
inspector. JPEG PR 225 documents concrete differences. Unify mechanics first;
preserve each caller's behavior until a separately reviewed fix changes it.

## Container rewriting stays with codecs

Codec-owned operations must be callable directly, without a registry or pipeline.
JPEG owns markers, ICC/extended-XMP reassembly, MPF offsets and JPEG emission.
TIFF owns image layout and pointers. HEIF owns items, extents and associations.
PNG owns chunks and checksums. Analogous rules apply to other formats.

Use inventory as evidence and a verification tool, not as a generic deletion
recipe. A decoder ignoring bytes does not mean they are safe to remove. A range
does not describe the reference/checksum updates needed when bytes move.

The rewrite sequence is inspect, resolve dependencies, plan, validate, emit.
Reuse the existing validated-plan and borrowed-output approach from zenpipe PR
87. Place generic requests/results in zencodec only when codec callers need
them, shared metadata processing in zenmeta, and container planners in codecs.
The pipeline's remaining API delegates to those operations.

Plan validation precedes output, but a caller's sink can still fail after writing
partial output. Do not describe that as transactional file replacement.

## Metadata meaning and preservation

Distinguish:

1. selected metadata used by this decoder;
2. every physical metadata packet, including duplicates;
3. logical payloads assembled from multiple ranges;
4. metadata associated with a frame, item, page or auxiliary image;
5. byte preservation, rendered-output preservation and privacy satisfaction.

Compressed/transformed payloads do not have a simple offset correspondence with
their decompressed fields. Keep source-carrier provenance instead of inventing
physical field ranges.

Apple MakerNote HDR data can be a rendering dependency. Preserve its supported
interpretation or refuse an operation that cannot do so. Opaque MakerNotes are
not certified private. Do not label an approximate Apple-to-ISO mapping as an
exact preservation guarantee. Preserve the published compatibility helper while
making richer operations explicit about their model and limits.

Field-level policy changes and gain-map guard changes need semantic review even
when their Rust signatures are additive. Existing infallible Metadata::filtered
cannot become a full, fallible, container-aware sanitizer without changing its
contract; provide the richer operation separately.

## Reuse and verification

Move source with provenance, preserve existing tests and assertions, and add only
the integration tests needed by the new ownership/dependency boundaries. Separate
mechanical moves from behavior changes. Do not rewrite a parser while moving it.

Required gates:

- published-API comparison, including legacy types and feature surfaces;
- minimal/default, each metadata feature, and all supported combinations;
- no_std checks and dependency checks proving XML/inventory are absent by default;
- a codec built with and without inventory: capability and operation agree;
- generic/dynamic inventory parity under the same decode configuration;
- byte coverage, malformed/overlapping directories and resource limits;
- metadata extraction agreement with actual decoder results;
- unchanged compressed image data for metadata-only operations;
- pixel, orientation, color and gain-map interpretation preservation;
- valid rewritten offsets, sizes, checksums and item relationships;
- policy idempotence and explicit incomplete/opaque inspection outcomes;
- independent-reader oracles, outside production dependencies;
- retained duplicate implementations checked by differential tests.

Existing tests are evidence to carry over, not proof that every new combination
is already correct. Performance and memory claims require new measurements;
historical PR reports are not measurements of the migrated implementation.

### Keeping changes synchronized

Give each byte reader and semantic policy one owner. Codecs call the shared
implementation and retain tests for their own selection and acceptance rules.
Keep shared conformance checks in zencodec-testkit, including the disabled
inventory result and generic/dynamic parity; format fixtures stay with the
codec unless they exercise only the shared parser.

Make a shared-parser change run both its unit/oracle tests and the affected codec
fixtures. Pin the dependency revision used for each cross-repository run, then
validate the final release graph without development overrides. Compatibility
wrappers need differential tests against retained release behavior, especially
for malformed-input acceptance and recursive plist conversion.

API review must inspect resolved rustdoc types, not merely search source text
for `zenmeta::`: an alias or re-export can hide the dependency. Maintain old-client
compile fixtures alongside semver checks. Avoid speculative additions to
zencodec's stable types; add only data needed across a real codec boundary.

The [JPEG integration findings](zenmeta-migration.md#findings-from-the-completed-jpeg-adapter-checks-2026-10-13)
show where these contracts need continued attention: extraction versus pixel
interpretation, truncation versus strictness, and resolved metadata versus
adapter-specific detection. The report distinguishes fixed implementations from
possible future API changes; it does not propose speculative public types.

## Licensing and release boundaries

The source manifests differ: zencodec is Apache-2.0 OR MIT, ultrahdr-core is
Apache-2.0, and current zenjpeg/zenifd are AGPL-3.0-only OR
LicenseRef-Imazen-Commercial. Preserve notices and provenance. Do not silently
assign zencodec's license to copied sources or put a new differently licensed
dependency into published compatibility wrappers without deciding that boundary.

No breaking zencodec release is required by this design if published APIs stay
intact and new trait methods retain compatible defaults. Unreleased PR consumers
still need coordinated migration. New public APIs go through review before
publication; this document does not authorize publishing packages or removing
existing PRs.


## Contract boundary update (2026-10-13)

The codec-facing rewrite request and completed plan remain zencodec types.
Request execution (admission, cancellation, packet limits, XMP retention) and
bounded plan assembly live in zenmeta. Codecs own container validation and use
`zenmeta::rewrite::Context` internally; public signatures need not expose it.
`Plan::from_parts` accepts an input length and checks report bounds, but the
constructor does not certify that report actions match a codec's emitted bytes.

Published EXIF writing and Metadata filtering remain deprecated compatibility
implementations, as explicitly chosen for this migration. The new zenmeta entry
points delegate to those implementations; this is a staged physical extraction,
not a claim that all writer code has left zencodec. Removing those implementations
requires the queued breaking release. Baseline color, orientation, EXIF and ISO
gain-map reading remain usable without zenmeta or any pipeline/codec registry.
