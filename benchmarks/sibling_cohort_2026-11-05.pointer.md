# Shared sibling pixel cohort — 2026-11-05

Baseline: zencodec `a51d4119`. Paired zenpixels and zenpixels-convert now use
`fa836abf72afe9826e866dc25712b7c800323962` in the workspace, fuzz and isolated
render graphs. The renderer's tracked dependency lock matches that source.

The new non-exhaustive ColorAuthority exposed an exhaustive match in the
conformance testkit. Unknown authority values now return a conformance error;
known ICC/CICP checks and every existing assertion remain unchanged. The narrow
unreachable-pattern allowance also admits the older exhaustive enum versions
already covered by the dependency range. No public signature changed.

`just sibling-cohort-check` passes: 1,449 test/doctest passes, zero failures and
39 pre-existing ignores across the workspace, plus strict all-feature,
all-target Clippy. Isolated fuzz and renderer checks pass. Baseline strict
Clippy passed before the pin update. No codec pixel/quality claim follows from
these shared-crate gates; downstream codecs and frozen EVAL need their own gates.

Receipts on **dev**, checked 2026-11-05:

`/mnt/v/output/zensim-siblings-20261105/zencodec/`

`phase2-zencodec-{baseline,cohort,cohort-fixed,isolated}.log` preserve the baseline,
initial compile refusal and successful checks. `SHA256SUMS` records all hashes.
Validation ran in an isolated source mirror on lilith-wsl through the shared
heavy lock, two jobs and a 4 GiB hard cap. Successful workspace receipt:
`rc=0 38s | peak-RSS 0.50GiB | min-avail 6825MiB | peak-load 3.71`.
