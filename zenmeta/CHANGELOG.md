# Changelog

## [Unreleased]

### QUEUED BREAKING CHANGES

- None: zenmeta has no crates.io release yet. Changes to proposed PR-only paths
  do not remove a released API.

### Added

- Existing XMP inspection, EXIF/XML audit and gain-map retention guards from
  zencodec PR 130 (`27cfae5d`), with their regression tests. XMP and audit are
  independent, default-off features. zencodec remains the owner of shared EXIF,
  color, orientation and ISO gain-map types.
