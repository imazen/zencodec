# Actual package cohort qualification, 2026-10-30

Base: `6b338ed415a39351b5994b652e57e03c9611723f`. Changes in this commit
remove only the version from testkit's repository-only zenmeta dev dependency,
add the reproducible package consumer gate and update documentation. No codec
implementation or assertion changed. I/O publication remains disabled.

## Why core alone is insufficient for the standing testkit rollout rule

The live published `zencodec-testkit 0.1.0` index still requires core `^0.1.26`
and pixels `^0.2.10`. Resolving it beside the extracted core 0.2.0 package
reproduces core 0.1.26 + 0.2.0 and pixels 0.2.16 + 0.3.1. The negative control
asserts this split instead of mistaking a successful resolution for type
interoperability. The candidate testkit 0.2.0 graph has exactly one version of
each family member: core 0.2.0, I/O 0.1.0, testkit 0.2.0 and both pixels 0.3.1.

`just release-cohort-check ARCHIVES NEW_OUTPUT` extracts the five actual `.crate`
files outside workspace overrides, records both resolved graphs, copies the
existing eleven `zencodec-testkit/tests/usage.rs` examples unchanged and executes
them against the candidate packages. Their tests and strict Clippy pass. This
is reference-codec caller coverage, not additional native codec qualification.

The gate also tests the actual I/O and testkit packages with all features/all
targets, doctests and strict Clippy. I/O: 23 unit tests plus one doctest pass
(one existing ignored doctest). Testkit: 62 unit tests pass. Repository-only
integration tests are not in those tarballs; their source-workspace evidence
remains separate. All patches point at extracted packages, never source trees.

## Packaging failure and correction

Before the dev-edge correction, standalone testkit metadata fails with
`no matching package named zenmeta found`. Its registry entry returned 404.
Only repository integration tests use that dependency; packaged src/ does not.
The path-only dev dependency preserves those repository tests while Cargo omits
it from the normalized artifact. No normal dependency or test assertion was
removed. The failed original archive and logs were retained.

Final cohort wrapper: `rc=0 12s | peak-RSS 0.34GiB | min-avail 13340MiB | peak-load 0.39`.
Earlier caller-only gate: `rc=0 11s | peak-RSS 0.33GiB | min-avail 13333MiB | peak-load 0.41`.
The original standalone metadata control exits 101. One preceding shell setup
failed to create its config; it is not evidence of a package failure.

The core tarball separately passes all-target tests and doctests against the
extracted pixel package. Its first wrapper's Clippy command misplaced the patch
before the external subcommand and exited 101 after those tests. Moving the same
config after `clippy` passes strict linting and graph verification: `rc=0 2s |
peak-RSS 0.39GiB | min-avail 13401MiB | peak-load 0.35`.

Catalog regeneration and strict Rustdoc plus all 27 source checks and the full
browser suite pass after the README/manifest edits: `rc=0 68s | peak-RSS 0.34GiB |
min-avail 12868MiB | peak-load 0.89`. A prior build correctly rejected a guide /
Rustdoc revision mismatch; rebuilding the reference fixed orchestration without
weakening the identity guard.

These are validation-wrapper resource figures, not codec memory/performance.
This does not qualify registry availability, enable I/O publication, approve a
release or qualify unfinished video presentation contracts. The release batch
needs owner approval and required green platform CI. Core can publish first in
that batch, followed by I/O and matching testkit before calling the rollout done.

## Final registry-only gate

`just release-registry-check NEW_OUTPUT` uses the same eleven caller examples
without extracting archives or writing patches. It requires the exact candidate
versions, one family identity each, and the crates.io source for every selected
dependency (only the generated consumer itself may be a local package).
The ordinary extracted-package mode still passes unchanged tests and strict
Clippy: `rc=0 12s | peak-RSS 0.30GiB | min-avail 13428MiB | peak-load 0.31`.

Before publication, registry mode fails as required at resolution: core 0.2.0
is unavailable; Cargo lists 0.1.26 and earlier. Wrapper `rc=1 0s | peak-RSS
0.03GiB | min-avail 13757MiB | peak-load 0.26`. No fallback ran and no registry
success is claimed. The positive registry gate remains to run after explicit
release approval, publication and index availability.
