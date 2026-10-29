# Incremental testkit pre-cancellation — 2026-10-29

Base: `d99f11a5`; changes are the enclosing commit. ARM64 isolated source export,
3 GiB cap, two Cargo jobs, timeout 900 seconds, no workspace path overrides.

The unchanged harness accepted a wrapper that ignored an already-tripped Pulse:
`harness_detects_precancellation_violations` failed with “faulty decoder must
fail conformance: ()”. The strengthened helper checks Cancelled and TimedOut
before normal fixture work, records all four mutating sink hooks (including
failed loans), and requires Finished afterward with no further sink effects.

Final commands:

```sh
cargo test --locked -p zencodec-testkit --test sansio_checks
cargo test --locked -p zencodec-testkit --lib
cargo clippy --locked -p zencodec-testkit --lib --test sansio_checks -- -D warnings
```

Results: 5/5 integration controls, 59/59 library tests, strict Clippy pass.
The integration target includes reference/source checks, three deliberate
pre-cancellation violations and DiagnosticPulse observation of honest versus
swallowed cancellation/timeout. No native codec or responsiveness benchmark was
run by this gate.

Retained attempts: first fixed-run fixture incorrectly left its inner decoder
unterminated while testing reason collapse; fixed to preserve the underlying
terminal transition. The next diagnostic test compilation missed a DecodeJob
import; adding it resolved both compiler diagnostics. No previous behavioral
assertion was weakened. Full logs: `~/tmp/testkit-precancel-20261029` on the
qualification host and coordinating host, including before.log, after.log,
after2.log and after3.log.

Final run-heavy receipt: rc=0, 16s, peak RSS 0.35 GiB, min available 14417 MiB,
peak load 0.56. This measures the validation process, not a codec memory bound.
Dependency resolution added only how-far-along 0.1.0, how-far-really 0.1.0 and
critical-section 1.2.0; diagnostic dependencies are testkit dev-only.

The compiler-derived core and testkit catalogs were regenerated with unchanged
public signatures. Core needed regeneration because the earlier package-audit
manifest comment changed its input fingerprint; documentation CI 40960770473
reported that stale catalog. The fresh isolated site build and all 26 source/site
checks pass. Initial export attempts omitted GITHUB_SHA, then reused the stale
core catalog; their logs remain, and the final build used explicit source identity
and regenerated catalogs. Browser layout was unchanged and not rerun here.

The subsequent complete testkit all-targets run passed 176 tests, with
0 failures and 0 ignored. Wrapper: rc=0, 9s, peak RSS 0.26 GiB,
min available 14454 MiB, peak load 0.26. Existing assertions remain intact.
