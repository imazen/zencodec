# Retained-job replay driver qualification — 2026-10-28

Production source: `03b91e5d530c15c209d8c0ca74da788028e09539`.
The source differs from `d7b76287` only by new fixture corrections and documentation;
production changes after that snapshot are field documentation. These are
correctness/build receipts, not runtime performance or compile-time comparisons.

## Passing scope

- Standard I/O tests, including all 38 planned-row tests (18 prior, 20 new).
- Standard and all-feature I/O strict Clippy; all-feature I/O tests.
- Sixteen joint encoding-route policy controls.
- Generated public API/check, eight source catalogs, 24 site-source checks.
- Full browser suite against its own frozen build: exact source scopes and copy,
  consumer/adapter/implementor tabs, independent inline expansion, signatures,
  search/pagination, keyboard navigation and 390/768/1440 px layouts.

Commands run in an isolated export with retained locks:

```sh
cargo test -p zencodec-io --features std --tests --locked
cargo clippy -p zencodec-io --features std --all-targets --locked -- -D warnings
cargo test -p zencodec-io --all-features --tests --locked
cargo clippy -p zencodec-io --all-features --all-targets --locked -- -D warnings
cargo test -p zencodec --all-features --locked --lib operation::encoding_route
just api-doc
just api-doc-check
just code-catalog "$PWD"
just docs-site-check
just docs-site-browser-check 3547
```

The native IMI/public-driver harness is a separate gate; these results do not
qualify its configuration, replay behavior, byte parity or storage costs.

## Failures and corrections retained

1. `d7b76287`: new RGB fixtures used non-pixel-aligned stride; new refusal
   checks incorrectly expected direct `UnmetRequirements`. Corrected fixtures
   use whole-pixel padding and inspect the existing typed `PolicyFailure` and
   its named unmet requirements. Existing assertions remain unchanged.
2. `f7730e0b`: tests passed; strict Clippy found seven undocumented public
   refusal fields. Documentation added without behavior changes.
3. First `03b91e5d` run: standard Clippy passed; the supervisor stopped its own
   job as disk space crossed the floor. Inactive Cargo targets were removed;
   source, locks and failed-run evidence were retained.
4. Resume: seven remaining build/source gates passed. Browser equality failed
   because the raw browser recipe connected to the older preview on port3537.
   The unchanged suite passed using the existing isolated-server recipe on3547.
   No assertion, expected source or viewport check was relaxed.

## Resource-wrapper receipts

All commands were serialized with `run-heavy --mem 3G --jobs 2`.

| Run | Exit | Elapsed | Peak RSS | Minimum available | Peak load |
|---|---:|---:|---:|---:|---:|
| Initial fixture failures | 101 | 20 s | 0.41 GiB | 7590 MiB | 1.12 |
| Corrected fixtures; missing-doc lint | 101 | 26 s | 0.41 GiB | 7489 MiB | 1.28 |
| Standard Clippy; disk-floor stop | 143 | 27 s | 0.32 GiB | 7530 MiB | 1.09 |
| Remaining gates; stale-preview browser | 1 | 61 s | 0.48 GiB | 7323 MiB | 1.39 |
| Isolated browser and source checks | 0 | 40 s | 0.29 GiB | 13432 MiB | 0.66 |

These wrapper measurements describe the validation commands, not encoder RAM,
application throughput, native workspace bounds or a before/after comparison.
