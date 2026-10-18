# Changelog

## [Unreleased]

### QUEUED BREAKING CHANGES

None.

### Fixed

- C2PA audit CBOR reparses, JUMBF traversal and report formatting share a
  cumulative allocation/item budget. Opaque findings stop at one terminal limit.
  Escaped, typed paths and source occurrence suffixes keep duplicate fields
  distinct in structural diffs (`4fa4561b`).

- `xmp::Packet::parse` (and every XMP reader built on it: `xmp::gainmap::parse_xmp`,
  `parse_xmp_full`, `xmp::container::Directory::parse`) rejects a packet nesting
  more than 64 elements before roxmltree parses it. roxmltree's parser recurses
  once per open element, so the existing check after parsing came too late: a
  packet nesting 8,848 elements (closed or not) overflowed a 2 MiB thread stack
  and aborted the process. A linear scan that skips comments, CDATA, processing
  instructions, declarations and quoted attribute values counts the depth first.

- Require appended child rewrite plans to borrow the exact parent subrange their
  reports describe. Equal lengths or equal bytes from another allocation are not
  sufficient provenance; rejection leaves the parent builder unchanged.

- Reject mixed rewrite inputs by borrowed input identity, validate an entire child
  plan before appending it, and preserve builder state when an append fails
  (`eddbdd51`).
- Recheck input limits after replacing a context's input and poll cancellation
  before packet services. Preflight expanded EXIF storage, including overlapping
  values and thumbnails, before invoking the compatibility serializer; recheck
  its final output against the metadata budget (`eddbdd51`).

### Added

- `c2pa` feature: `audit::Report::c2pa` structurally inspects a complete C2PA
  Manifest Store (JUMBF, CBOR claims/assertions and COSE headers). Certificates,
  RFC 3161 timestamp tokens and other uninterpreted data remain opaque.
  `audit::Finding` gains `InvalidJumbf`, `InvalidCbor` and `Opaque`. Input,
  cumulative requested allocation capacity, nesting and item limits bound the
  inspection; no signatures, hard bindings or credential semantics are verified.
  The example accepts raw stores; codecs own container framing. See the
  [c2pa-rs integration design](../docs/c2pa-integration.md) for signing and validation.
- Metadata/EXIF processing entry points bridge retained zencodec compatibility
  writers without duplicating their implementation. Reading needs no zenmeta.

### Changed

- Narrow the unreleased C2PA audit to structural inspection: remove handwritten
  DER interpretation, retain certificate/timestamp bytes as opaque findings and
  limit the example to raw stores. Signing and verification use the proposed
  separate c2pa-rs adapter (`5aaab6f2`).

- Return typed shared limit kinds for rewrite budgets. Distinguish individual
  packet limits from cumulative regenerated metadata using the same configured
  metadata cap. Rejection still leaves a plan builder usable.

- Rewrite execution checks now live in Context instead of the zencodec request.
  Retaining XMP can copy it byte-for-byte without an editor. PlanBuilder checks
  source bounds and does not record a carrier whose output exceeds its budget.


- Add optional `rewrite` support: bounded shared EXIF validation/filtering,
  application-service limits and fallible plan assembly. Codec public signatures
  use zencodec-owned request/plan/error types. Container planners remain with codecs.


### QUEUED BREAKING CHANGES

- None: zenmeta has no crates.io release yet. Changes to proposed PR-only paths
  do not remove a released API.

### Added

- Namespace-aware GContainer `Directory` inspection and length/padding editing share the existing RDF traversal. Recovered lossless tests retain semantic ordering, quote style and the malformed-input time bound; malformed or ambiguous directories now fail explicitly. No codec signatures expose this implementation type (`a81daf76`).

- Shared gain-map XMP parsing/generation and container-directory interpretation
  from zenjpeg PR 210 (`74e727f6`), behind `xmp`; no JPEG framing or codec
  dependency. Existing tests move with their parser.

- Explicit Apple compatibility helpers share the strict reader while retaining
  ultrahdr-core 0.6.0 selection, malformed-entry and numeric behavior. A frozen
  released reader checks mutations and truncations; no codec re-export is required.

- Shared TIFF/BigTIFF walker and tag names from zenextras PR 30 (`601682d3`),
  gated by the default-off `inventory` feature. Existing decoder-specific Rules
  stay in their codecs. No zenifd package is introduced.
- Apple MakerNote inspection and strict HDR parsing from ultrahdr PR 35
  (`1dee8471`), plus the existing binary-plist parser from ultrahdr-core 0.6.0.
  Both retain their tests and are gated by the default-off `apple` feature.
- Existing XMP inspection, EXIF/XML audit and gain-map retention guards from
  zencodec PR 130 (`27cfae5d`), with their regression tests. XMP and audit are
  independent, default-off features. zencodec remains the owner of shared EXIF,
  color, orientation and ISO gain-map types.

### Fixed

- Explicit package contents exclude repository instruction files; public API
  snapshot validation checks the packaged surface.

- Apple EXIF entry sizes use checked arithmetic, including on 32-bit targets;
  oversized counts reject instead of the released ultrahdr reader's overflow panic.

- Binary-plist parsing now checks count/offset arithmetic and stops values at the
  object-table boundary. Existing ultrahdr-core 0.6.0 parsing could panic on a
  crafted data length or consume offset-table bytes as a truncated value.
- Bound total plist expansion and requested output allocations, including repeated
  references; depth alone did not bound expansion. Allocations are fallible.
- Wide integers/UIDs use their low 64 bits as documented, after validating the
  complete value; the imported parser read the high bytes instead. Malformed date
  markers and over-wide lengths are rejected. These change parser behavior from
  the source release and must be reflected in consumer changelogs when delegated.

### Changed

- Use Apache-2.0 OR MIT for the extracted, controlled source with copyright-holder
  authorization. Third-party dependency licenses remain unchanged.
