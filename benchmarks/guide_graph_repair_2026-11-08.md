# Separate guide graph repair

Base: PR201 `c8fcf2d8`. No production core source or carrier-only cohort changed.

The integration snapshot still named zenpipe f0322466 after the native policy
harness moved to its minimal public-facade descendant 154e16b0. Recapture through
`docs/capture-integration-api.mjs` changes only the revision; the extracted API
surface and its checksum are identical. The complete site build and all 27 Node
source checks pass after the manifest and snapshot agree.

The separate I/O example graph vendors bitmap source 5abfba28, whose manifest
still caps archmage at beta.1. Its descendant b446f830 changes only that cap and
the changelog; runtime sources are byte-identical. The harness now repeats garb
58ce9cad and existing linear528, resolves stable SIMD 0.9.31, and uses the
already-declared pixel/convert fde59b96 source. No unrelated package versions
changed. The graph checker finds 10 interface consumers and zero mismatches.
The source fetcher verifies the exact published descendant against the tested
vendor files. No vendor hash check is bypassed.

Compilation exposed one old `plane.code()` call in the media example test. It
now matches the canonical U8 plane and reads the same coordinate, retaining the
expected value 9 and adding an explicit sample-type assertion. Initial failed
compiles are retained; no assertion or target was removed.

The exact CI example selection passes 77 tests, zero ignored, followed by strict
Clippy. Test receipt: rc=0, 5s, peak-RSS 0.20 GiB. Clippy receipt: rc=0, 19s,
peak-RSS 0.32 GiB. The transport simulation CSV equals the committed baseline.
These runs are correctness checks, not performance measurements.

The independent policy-portability lock was also stale. Resolution changes its
pixel source to the existing manifest pin and unused patch records only. All
four CI commands pass: WASM check, no-default freestanding thumb check,
no-default library tests, and strict no-default Clippy. Combined receipt:
rc=0, 9s, peak-RSS 0.24 GiB.

Reproduce in an isolated export using the vendor setup and committed harness
lock, `just docs-site-examples`, `just docs-site-check`, and the four explicit
policy portability commands in `.github/workflows/docs.yml`. Preserve the
committed transport CSV comparison. Browser and remote CI remain separate.
