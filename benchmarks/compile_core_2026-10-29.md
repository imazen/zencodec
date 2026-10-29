# Candidate core compile cost — 2026-10-29

Exploratory observations, not clean-host confirmation. All 60 builds completed;
all 60 have sampled CPU-load flags and 10 have heavy-process observations. No
sample was removed. A/A controls are diagnostic, not a correction for those flags.
The packet does not identify the heavy processes, so their origin is unknown.

Sources: published crates.io 0.1.26, pre-redesign main `a20e4dd3`, and candidate
`e9f2f31d051f3a859c51bd5e70581c2ee90fd457`. The pre-redesign package calls itself
0.1.27 but that version is not published. Cargo search reports 0.1.26; cargo info
for 0.1.27 reports not found. The README's published-documentation link is corrected.

The consumer only re-exports zencodec. Each invocation uses an empty unique target,
two Cargo jobs, offline/frozen dependencies and no incremental compilation or
compiler wrapper. OS caches are warm. There are five observations per cell.
This is an actual whole consumer build comparison, not equal API functionality:
the candidate has a larger API, adds spin and selects paired zenpixels 0.3.1;
the older graphs use zenpixels 0.2.16. Native codecs, IO/media adapters, application
monomorphization and tests are excluded. All six exact locks and dependency trees
are retained with the source archives and runner in the artifact directory.

| Configuration | Published | Published A/A | Pre-redesign main | Candidate |
|---|---:|---:|---:|---:|
| dev-default | 1.93s | 1.95s | 2.26s | 2.79s |
| dev-std | 1.95s | 1.93s | 2.24s | 2.76s |
| release-default | 2.34s | 2.34s | 2.88s | 3.83s |

These are median wall times for the complete cold build. Cargo's zencodec-unit
medians, measured within those builds, are:

| Configuration | Published | Pre-redesign main | Candidate |
|---|---:|---:|---:|
| dev-default | 0.39s | 0.69s | 1.19s |
| dev-std | 0.40s | 0.69s | 1.17s |
| release-default | 0.71s | 1.24s | 2.14s |

The JSON receipt preserves minima/maxima, exact RSS observations, identities and
quality flags: [compile_core_2026-10-29.json](compile_core_2026-10-29.json).
Toolchain: rustc/cargo 1.99.1 on Ryzen 9 7900X. No target-cpu=native.

Commands: `just compile-time-prepare ROOT a20e4dd3 e9f2f31d`, then
`just compile-time-run ZENBENCH ROOT`, then `just compile-time-report ROOT LOG RECEIPT`.
zenbench 0.1.11 uses the unchanged five-block A/A Iterate plan. No statistical
confirmation or causal attribution to a particular API is claimed.

Wrapper: rc0, 181s, peak RSS 0.31 GiB, minimum available 13815 MiB, peak load 1.61.
Per-build /usr/bin/time RSS is in the receipt; wrapper RSS is not native codec memory.
Completed targets were deleted; compiler logs, timing pages and the zenbench packet
remain. Artifact root: `/home/lilith/tmp/zencodec-compile-20261029`.

The reporter's former target/cargo-timings path was stale after target cleanup.
A failing-first synthetic 60-invocation control reproduces that error; the reporter
now reads retained timing files and still accepts historical target layouts. All
five runner tests pass. Synthetic fixture values are not benchmark measurements.

Next: diagnose the resource observations before seeking confirmation, and measure
the IO/media consumer separately. These numbers do not close runtime or memory
qualification for the release contracts.
