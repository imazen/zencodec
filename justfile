# zencodec dev commands

# Format + regenerate the public-API surface snapshot (docs/public-api/)
fmt:
    cargo fmt --all
    cargo test -p zencodec --test public_api_doc

# Regenerate the public-API surface snapshot only
api-doc:
    cargo test -p zencodec --test public_api_doc

# Verify the committed snapshot is current (what CI runs)
api-doc-check:
    ZEN_API_DOC=check cargo test -p zencodec --test public_api_doc

# CI-exact clippy
clippy:
    cargo clippy --all-targets -- -D warnings

# Full workspace test gate (incl. the conformance testkit)
test:
    cargo test --workspace
