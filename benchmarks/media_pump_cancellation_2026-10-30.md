# Shared media pump cancellation boundaries

The send/drain helpers forwarded Pulse but did not guard their own native and
publication effects. A backend ignoring Pulse could produce output even when
the caller was already cancelled; end_input had no Pulse parameter at all.

The control in media-testkit's encoder_fusing exercises all four helpers with
pre-cancellation, cancellation during successful receive, cancellation in a
successful downstream callback, and simultaneous native failure/cancellation.
It requires zero or one confirmed publication as appropriate, no next native
call, typed TimedOut propagation, and native error priority. The deliberately
ignoring backend cannot spin indefinitely: a second publication returns an error.

## Failing first

Snapshot fe1b1320 initially failed to compile because the new fixture called its
trait method info rather than stream_info. Correcting that fixture name in the
isolated export, with production unchanged, reproduced:

```
method=0 scenario=0: Err(Downstream("published twice"))
```

Expected was typed cancellation before any native or downstream effect. Original
and corrected failure logs are retained separately. No established assertion
changed.

## Fixed controls

Production snapshot c50c33e7 guards every native call and callback before entry
and after success. Original failures take priority over a concurrent stop.
Separate before/after Cargo targets prevent stale binary reuse. Both exports
use the same locked graph from the preceding ordinary-timeline qualification.

```sh
cargo test --locked -p zencodec-media -p zencodec-media-testkit --all-targets
cargo clippy --locked -p zencodec-media -p zencodec-media-testkit --all-targets -- -D warnings
cargo check --locked -p zencodec-media --no-default-features --target wasm32-unknown-unknown
```

All 90 tests pass, none ignored/filtered. Strict Clippy and WASM pass.
3 GiB/two-job wrapper: rc0, 8 seconds, peak RSS 0.29 GiB,
minimum available 13365 MiB, peak load 0.34.

Native snapshot d880185d switches the baked AV1 example to the shared send/drain
encoder pumps with None control. Generation, strict example Clippy and all twelve
independent sample/timing comparisons pass. The harness package was explicitly
cleaned and compilation from the new export recorded. Wrapper rc0, 7 seconds,
peak RSS 0.39 GiB, minimum available 13419 MiB, peak load 0.47.
This verifies ordinary native behavior; cancellation timing inside that native
encoder is not measured here. Wrapper usage is not codec performance evidence.

Full logs: /home/lilith/tmp/zencodec-pump-20261030/{before,before-retry,fixed,native}.log
on r7900x. Native outputs and independent probe/raw facts:
/mnt/v/output/zencodec/presentation-playback-20261030/{pump-files,pump-oracle}.
All wrappers exited. The helpers borrow the backend; callers must discard the
session on terminal error. No preemption or rollback guarantee is added.

The eight-crate compiler catalog was regenerated from the pushed production
source: rc0, 17 seconds, peak RSS 0.33 GiB. The local nightly compiler
(1.101.0-nightly, e9bbadf68) also renders several standard I/O re-exports using
their core::io paths; these generated name changes are not source API edits.
Catalog tests, site generation and all 26 site source tests pass: rc0,
4 seconds, peak RSS 0.34 GiB. Browser interaction was not rerun for this
signature/documentation-only update.
