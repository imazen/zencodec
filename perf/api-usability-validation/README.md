# API admission and allocation probes

This standalone crate exercises the production policy resolver, reference-codec
decode requirements, and row-output staging. Its dependencies stay outside the
product graph. Counts come from dhat and recording-encoder pointers; they are
not native-codec memory bounds or timing benchmarks.

From the repository root:

```sh
just api-usability-runner-check
just api-usability-replay "$HOME/tmp/api-replay-unique-name" <core-commit>
```

Run the second command under the workspace's resource wrapper in an available
build slot. The results directory must not exist. Each run preserves source
identity, dependency hashes, stdout/stderr and individual exit statuses.
Failed tests, probes, source exports and lock checks make the runner fail.

For an in-place build, copy `dependencies.lock` to `Cargo.lock`, then run
`cargo test --locked` and `cargo run --locked`. Current Pulse tests are the
default. `baseline-6b0caf34` and `proposed` select historical test fixtures that
require that older core; they are not current-core feature combinations.

The dated TSV files retain the original observations. The
[review](../../docs/api-usability-review-2026-10-27.md) distinguishes the old
callback bypass from current admission behavior and remaining costs. The
[policy-binding experiment](../../docs/policy-binding-experiment.md) covers the
separate, unresolved requirement-placement and intermediate-ownership problem.

## Integration qualification

An isolated export of `97695dba` passed the nine current-behavior tests, all four
original proposed safety assertions, the allocation probes, and strict
all-target Clippy. The tracked dependency lock remained unchanged. The resource
wrapper reported `rc=0`, 16 seconds, peak RSS 0.45 GiB, minimum available memory
5297 MiB and peak load 0.66. These are build/test resource readings.

The runner's 22 fault controls also pass. They cover failed test/probe commands,
failed exports (including failure after producing a valid archive), unresolved
revisions, timeouts, lock changes, occupied directories and changed assertions.
Control logs remain available after success as well as failure.
