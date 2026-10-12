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
