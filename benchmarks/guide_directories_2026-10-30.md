# Retained publication fixture directories

The Documentation workflow on 6bc504e2 (run 41011652214) failed four
checked-publication examples at checked_examples.rs:186 with AlreadyExists.
Each repeated its label in one process. No codec assertion was reached in the
failing case. Existing artifacts must remain intact.

Source d566ac157e7d4081cfcafda5cbabab45878f00f7 creates each directory
exclusively with a process-local ordinal, retrying occupied names. All prior
publication/cancellation assertions remain unchanged. A new helper control
checks repeated labels and retained sentinel bytes.

Validation: isolated source export outside workspace overrides, committed
JPEG dependencies.lock copied to Cargo.lock, locked commands:

```sh
cargo test --manifest-path perf/jpeg-policy-validation/Cargo.toml --locked -- --nocapture --test-threads=1
cargo clippy --manifest-path perf/jpeg-policy-validation/Cargo.toml --all-targets --locked -- -D warnings
```

All 69 native harness tests pass, none ignored or filtered. Strict Clippy
passes. The standalone directory control also passed twice. Linux x86_64,
3 GiB cap, two jobs, timeout 1200 seconds. Wrapper rc0, 33 seconds,
peak RSS 0.71 GiB, minimum available 12975 MiB, peak load 0.93.
This is validation resource usage, not a performance benchmark.

Full log: /home/lilith/tmp/zencodec-ci-20261030/run.log on r7900x.
Original failure: docs-failed.log beside it. Wrapper exited and no Cargo/rustc
processes remained. Cross-platform qualification belongs to CI.
