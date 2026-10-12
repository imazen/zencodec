# zenmeta

Implementation crate in the zencodec workspace. Keep public codec interchange
types in zencodec; do not add a dependency from zencodec back to zenmeta.

Run the workspace `just metadata-check` and `just metadata-semver` gates under
the host's resource wrapper. Preserve the moved tests and source provenance.
Feature combinations must work independently; default builds have no XML parser.

## Known Bugs

- Resolved during extraction: the binary-plist parser imported from ultrahdr-core
  0.6.0 overflowed on a crafted data count, had no total expansion budget, and
  returned high rather than documented low bits for wide integers. Regression
  cases failed before the fix and pass after it. Compatibility-wrapper consumers
  must document these behavior changes when switching implementations.
