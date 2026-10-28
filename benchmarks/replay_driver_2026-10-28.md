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

## Windows CI fixture correction

CI run `40786606889` on `62b92eff` failed three new raw-disk controls on both
Windows targets before opening a file: the fixture required `HOME`, while those
runners expose `USERPROFILE`. The fixture now accepts that home-directory variable
and closes any still-owned file before cleanup. No production behavior, test
expectation or disk-control applicability changed. CI run `40787178376` on
`d7b5a159` passes, including both Windows targets. The Documentation and Fuzz
workflows on that revision also pass. New configured-job and documentation
changes are qualified separately below.

## Native public driver and configured erasure

Frozen core `a39616ba599dd34b89720e03fb20aaf8a4815206`, native
`136784e486b6d6da5c47604ecf37025c7b19193a`, paired zenpixels `283655c5`:
the configured-job ownership tests and core strict Clippy pass. The complete
native/public-driver harness passes all 25 tests and strict all-target Clippy.
The parity matrix covers typed, configuration-erased and configured-job-erased
calls, RGB8/RGB16 with padding, checksums on/off, four acquisition routes and
Forward/SeekBack. Typed faults, admission refusal and owner cleanup pass. These
are same-native byte/readback controls, not an independent decoder oracle.

The separate native final integration gate passes 7+9+16 test executions; its
strict Clippy stops on four constant-size chunk-iterator suggestions in the new
fixture. That native gate remains incomplete. The aggregate run therefore exits
101: 154 s, peak RSS 1.17 GiB, minimum available 13633 MiB, peak load 2.02 under
`run-heavy --mem 3G --jobs 2`. Its following documentation stages were unrun and
are continued separately. No test expectation or production behavior changed.

## Generated API reference and guide

Frozen `c910076c379b837ffd2fd7a97777f0429187306d`: API generation/check,
eight compiler-derived catalogs, all-feature Rustdoc HTML for eight crates,
24 site/source checks and the full browser suite pass. Rustdoc uses
`-D warnings`; browser assertions retain the existing 390/768/1440 px checks.
Commands: `just api-doc`, `just api-doc-check`, `just code-catalog "$PWD"`,
`just docs-reference`, `just docs-site-browser-check 3547`. The generated API
snapshots and catalogs are copied back unchanged from the tested export.

Two failed documentation attempts are retained: seven short intra-doc links in
media output failed Rustdoc resolution (fixed with explicit `crate::output`
paths), then the added reference navigation link exposed a 768 px overflow
(fixed by allowing navigation to wrap and the header to grow). No warning or
browser assertion was relaxed. Final serialized wrapper receipt: rc0, 58 s,
peak RSS 0.29 GiB, minimum available 13899 MiB, peak load 0.96, 3G/jobs2.
All eight Rustdoc crate entry points return HTTP 200 in the preview, and the
configured-job method anchor is present. Remote access to the source-identity
manifest is verified from another machine. This is documentation/build
qualification on the generation host, not cross-platform release certification.
