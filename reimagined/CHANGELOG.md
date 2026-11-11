# Changelog

All notable changes to this unpublished experiment are recorded here.

## [Unreleased]

### QUEUED BREAKING CHANGES

- `ed892eae`: the standalone 0.0.0 prototype adds DecodeEvent::Continue. This is
  not a change to the released zencodec package.

### Added

- `ed892eae`: retained document discovery, scoped native levels/channels, exact
  signed selections and immutable selection receipts.
- `e8570af8`: blocking source/document driver reusing the existing range planner
  and pixel protocol, with selection/publication guards.
- `f72a3d51`: lazy browsing, ROI, refusal, cancellation and adversarial controls.
- `fd98c4ea`: standalone native tiled-TIFF witness and independent libtiff oracle.

### Fixed

- `96864b18`: captured fixture Clippy issues, preserving original expectations.
- `e8570af8`: incomplete or swallowed-error selected completion, and retained
  document ownership after pre-cancellation.
- `fd98c4ea`: native validation-worker teardown retrying a closed transport.
