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
