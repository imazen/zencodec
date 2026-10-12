# Source provenance

The initial XMP, audit and display-retention implementations and their tests are
extracted from [zencodec PR 130](https://github.com/imazen/zencodec/pull/130),
commit `27cfae5d4de24bfb20d2005d77daf4ac9ff05f39`. Their original paths are
`src/xmp.rs`, `src/metadata_audit.rs`, `src/display_metadata.rs`,
`tests/metadata_inspection.rs`, `tests/display_retention.rs`, and
`examples/audit_metadata.rs`. That source is Apache-2.0 OR MIT; this crate
was initially extracted under the Apache-2.0 option. Imports and feature gates change with ownership;
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

TIFF/BigTIFF traversal comes from [zenextras PR 30](https://github.com/imazen/zenextras/pull/30),
commit `601682d365d7b8d772bb045f45bb554005f41fb1`, `zenifd/src/walk.rs` and
`zenifd/src/tags.rs`. The shared source and its inline tests now live in
`src/ifd/`, gated by `inventory`; no zenifd package is required.

On 2026-10-12 the copyright holder authorized Apache-2.0 OR MIT licensing for
the controlled source moved into zenmeta, including the formerly AGPL/commercial
IFD implementation and Apache-licensed Apple implementation. This does not
relicense dependencies or remove any third-party notices.

The test-only `tests/support/apple_0_6_0.rs` is the published Apache-2.0 Apple
reader, retained as a frozen compatibility oracle (only its GainMapMetadata
import changes). Mutation/truncation comparisons pin tolerance differences;
it is not used by shipped code. Strict and compatibility interpretations share
the production entry reader and numeric conversion helpers.
