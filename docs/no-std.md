# Freestanding runtime crates

The core, IO, coroutine, media and metadata crates compile without the standard
library, including the core ICC parsing dependency. Disable default features on
consumer edges; Cargo unifies features across the complete dependency graph.
Native codecs and application adapters can independently require std.

The `std` feature explicitly enables zenpixels/std, native IO error kinds and
platform floating-point metadata math. Without it, gain-map scalar math uses
libm 0.2.16. No public signatures change. This repairs
[issue 165](https://github.com/imazen/zencodec/issues/165).

## Repeatable gates

```sh
just gainmap-portability-check
cargo test --workspace --all-features --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
```

The first recipe checks all five runtime packages on
`thumbv7em-none-eabihf`, where std is unavailable, then runs gain-map tests with
and without std. CI retains this freestanding check alongside its separate WASM
check. WASM compilation alone does not prove a std-free dependency graph.

## Qualification on 2026-10-29

Production source snapshot `752511c6c982c000a8c3a45b164f99a47631daa0` was
exported outside workspace overrides. The dependency update introduced no new
package identities: libm 0.2.16 was already transitive. It adds a direct core
edge, and disables implicit std on three zenpixels edges.

The five-package freestanding and WASM checks, both gain-map test configurations,
workspace all-feature tests and strict all-target/all-feature Clippy passed.
The existing gain-map assertions and numerical thresholds were unchanged.

```text
run-heavy: done rc=0 30s | peak-RSS 0.46GiB | min-avail 13559MiB | peak-load 1.47
```

Rust 1.88.0 core checks with no default features and with explicit std passed:

```text
run-heavy: done rc=0 5s | peak-RSS 0.25GiB | min-avail 14013MiB | peak-load 1.09
```

New math controls compare portable floor/round against platform result bits for
special values, deterministic floating-point bit patterns and half-integer ties.
NaN payload identity is not specified. Integer powers of two use a bit-constructed
oracle across normal and subnormal exponents, plus underflow and overflow.
Noninteger transcendental results are not certified bit-identical across math
backends. This is metadata math qualification, not a pixel-processing benchmark
or proof that every concrete codec supports freestanding execution.
