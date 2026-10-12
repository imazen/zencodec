# Changelog

## [Unreleased]

### QUEUED BREAKING CHANGES

- None: zenmeta has no crates.io release yet. Changes to proposed PR-only paths
  do not remove a released API.

### Added

- Apple MakerNote inspection and strict HDR parsing from ultrahdr PR 35
  (`1dee8471`), plus the existing binary-plist parser from ultrahdr-core 0.6.0.
  Both retain their tests and are gated by the default-off `apple` feature.
- Existing XMP inspection, EXIF/XML audit and gain-map retention guards from
  zencodec PR 130 (`27cfae5d`), with their regression tests. XMP and audit are
  independent, default-off features. zencodec remains the owner of shared EXIF,
  color, orientation and ISO gain-map types.

### Fixed

- Binary-plist parsing now checks count/offset arithmetic and stops values at the
  object-table boundary. Existing ultrahdr-core 0.6.0 parsing could panic on a
  crafted data length or consume offset-table bytes as a truncated value.
- Bound total plist expansion and requested output allocations, including repeated
  references; depth alone did not bound expansion. Allocations are fallible.
- Wide integers/UIDs use their low 64 bits as documented, after validating the
  complete value; the imported parser read the high bytes instead. Malformed date
  markers and over-wide lengths are rejected. These change parser behavior from
  the source release and must be reflected in consumer changelogs when delegated.
