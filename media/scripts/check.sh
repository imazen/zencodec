#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
python3 scripts/check_pins.py
python3 scripts/generate_reference_vectors.py --check
python3 scripts/generate_display_references.py --check
python3 scripts/generate_encode_color_references.py --check
python3 scripts/generate_av1_corpus.py --check
python3 scripts/generate_animation_corpus.py --check
cargo fmt --all -- --check
cargo test --locked --workspace --release --all-features
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
