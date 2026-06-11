# zencodec dev commands

# Format + regenerate the public-API surface snapshots (docs/public-api/).
# The snapshot test is local-only (never built or run in CI — see Cargo.toml).
fmt:
    cargo fmt --all
    cargo test -p zencodec --features _api-doc --test public_api_doc

# Regenerate the public-API surface snapshots only
api-doc:
    cargo test -p zencodec --features _api-doc --test public_api_doc

# Verify the committed snapshots are current
api-doc-check:
    ZEN_API_DOC=check cargo test -p zencodec --features _api-doc --test public_api_doc

# CI-exact clippy
clippy:
    cargo clippy --all-targets -- -D warnings

# Full workspace test gate (incl. the conformance testkit)
test:
    cargo test --workspace
