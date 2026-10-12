# Source provenance

The initial XMP, audit and display-retention implementations and their tests are
extracted from [zencodec PR 130](https://github.com/imazen/zencodec/pull/130),
commit `27cfae5d4de24bfb20d2005d77daf4ac9ff05f39`. Their original paths are
`src/xmp.rs`, `src/metadata_audit.rs`, `src/display_metadata.rs`,
`tests/metadata_inspection.rs`, `tests/display_retention.rs`, and
`examples/audit_metadata.rs`. That source is Apache-2.0 OR MIT; this crate initially
uses the Apache-2.0 option. Imports and feature gates change with ownership;
parser behavior and test expectations are retained.

The EXIF implementation remains in zencodec. Its public iterator is reused by
audit, avoiding a second semantic EXIF parser. Video contracts from the source
integration branch are not part of this extraction.

Apple MakerNote parsing and its inline tests come from
[ultrahdr PR 35](https://github.com/imazen/ultrahdr/pull/35), commit
`1dee8471bb32d3189add54e63d8f45bfeebfb440`,
`ultrahdr-core/src/metadata/apple.rs` (Apache-2.0). The binary-plist parser and
inline tests come from the Apache-2.0 ultrahdr-core 0.6.0 release,
`src/metadata/bplist.rs`. The strict MakerNote parser rejects malformed/duplicate
rendering tags; the released ultrahdr helper is more tolerant and needs a
compatibility path before delegation.
