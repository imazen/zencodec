# zencodec dev commands

# Format + regenerate the public-API surface snapshots (docs/public-api/).
# The snapshot runner lives in the workspace-excluded apidoc/ package, so it
# is never built or run by plain `cargo test` or any CI job.
fmt:
    cargo fmt --all
    cargo test --manifest-path apidoc/Cargo.toml

# Regenerate the public-API surface snapshots only
api-doc:
    cargo test --manifest-path apidoc/Cargo.toml

# Verify the committed snapshots are current
api-doc-check:
    ZEN_API_DOC=check cargo test --manifest-path apidoc/Cargo.toml

# CI-exact clippy
clippy:
    cargo clippy --all-targets -- -D warnings

# Full workspace test gate (incl. the conformance testkit)
test:
    cargo test --workspace

# Metadata extraction matrix; callers apply their host's build resource limits.
metadata-check:
    cargo fmt -p zencodec -p zencodec-testkit -p zenmeta --check
    cargo test --workspace --all-features
    cargo check -p zenmeta --no-default-features
    cargo check -p zenmeta --no-default-features --features xmp
    cargo check -p zenmeta --no-default-features --features audit
    cargo check -p zenmeta --no-default-features --features apple
    cargo check -p zenmeta --no-default-features --features inventory
    cargo clippy --workspace --all-targets --all-features -- -D warnings

# Compare the actual published contract, not an unpublished metadata branch.
metadata-semver:
    cargo semver-checks check-release -p zencodec --baseline-version 0.1.26 --all-features

# Complete source diff from the cargo-read artifact, with VCS provenance checks.
contract-diff published_dir output revision="origin/main":
    python3 scripts/audit-published-contract.py "{{published_dir}}" "{{output}}" --revision "{{revision}}"

# Observational probes; these do not enshrine design gaps as test expectations.
contract-probes:
    cargo run -p zencodec --example contract_audit_probes

contract-semver-minimal:
    cargo semver-checks check-release -p zencodec --baseline-version 0.1.26 --only-explicit-features

contract-audit-check:
    cargo fmt -p zencodec --check
    cargo clippy -p zencodec --example contract_audit_probes -- -D warnings
    just contract-probes
    just contract-semver-minimal

metadata-target-check:
    cargo check -p zenmeta --target wasm32-wasip1 --no-default-features --features apple,audit,inventory,xmp
    cargo +1.88.0 check -p zencodec -p zenmeta --all-features

metadata-i686-check runner="env":
    CARGO_TARGET_I686_UNKNOWN_LINUX_GNU_RUNNER="{{runner}}" cargo test -p zenmeta --all-features --target i686-unknown-linux-gnu

# Shared XMP must remain available without any codec or std dependency.
metadata-xmp-portability:
    cargo +1.88.0 check -p zenmeta --no-default-features --features xmp
    cargo check -p zenmeta --no-default-features --features xmp --target wasm32-wasip1
    CARGO_TARGET_I686_UNKNOWN_LINUX_GNU_RUNNER=env cargo test -p zenmeta --features xmp --target i686-unknown-linux-gnu

metadata-coordinated-check:
    just api-doc
    just metadata-check
    just metadata-semver

metadata-coordinated-final:
    just api-doc
    cargo test -p zencodec --all-features --lib capabilities::
    just metadata-semver
    just metadata-target-check

metadata-error-check:
    cargo test -p zencodec --all-features --lib inventory::
    cargo clippy --workspace --all-targets --all-features -- -D warnings
    just metadata-semver
    just api-doc

metadata-rewrite-check:
    cargo check -p zencodec
    cargo test -p zenmeta --features rewrite --test rewrite
    cargo clippy -p zenmeta --features rewrite --lib -- -D warnings

metadata-directory-check:
    cargo test -p zenmeta --features xmp --test container_directory
    cargo clippy -p zenmeta --features xmp --lib --test container_directory -- -D warnings
    cargo check -p zenmeta --no-default-features --features xmp --target wasm32-wasip1
    CARGO_TARGET_I686_UNKNOWN_LINUX_GNU_RUNNER=env cargo test -p zenmeta --features xmp --test container_directory --target i686-unknown-linux-gnu

metadata-directory-oracle:
    ZENMETA_DIRECTORY_OUTPUT="$HOME/tmp/zenmeta-directory-oracle" cargo test -p zenmeta --features xmp --test container_directory rewrite_item_lengths_matches_items_by_semantic -- --exact
    python3 scripts/check-directory-oracle.py "$HOME/tmp/zenmeta-directory-oracle/rewritten.xmp"

# Shared exact timing and HDR metadata retention integration.
test-animation-hdr:
    cargo fmt -p zencodec -p zencodec-testkit -p zenmeta --check
    cargo test --workspace
    cargo clippy --workspace --all-targets -- -D warnings
    cargo check --workspace --no-default-features --target wasm32-unknown-unknown
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
    just api-doc

# Rational input, ordinary rounding and explicit exact timing contracts.
animation-policy-check:
    cargo fmt -p zencodec -p zenmeta -p zencodec-testkit --check
    cargo test -p zencodec --all-features
    cargo clippy -p zencodec --all-targets --all-features -- -D warnings
    just metadata-semver

metadata-clippy:
    cargo clippy --workspace --all-targets --all-features -- -D warnings

# Adversarial review cases for the shared timing/rewrite boundary.
review-contract-check:
    cargo test -p zenmeta --features rewrite --test rewrite
    cargo test -p zencodec --test animation_timing

# Reusable contract checks and harness regression tests.
testkit-contract-check:
    cargo fmt -p zencodec-testkit
    cargo test -p zencodec-testkit
    cargo clippy -p zencodec-testkit --all-targets -- -D warnings
