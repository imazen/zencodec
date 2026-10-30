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

# Generate Rustdoc for the same eight crates as the guide's signature catalog.
docs-reference:
    python3 docs/build-reference.py

# CI-exact clippy
clippy:
    cargo clippy --all-targets -- -D warnings

# Full workspace test gate (incl. the conformance testkit)
test:
    cargo test --workspace

# Warning-free docs with the default workspace and standalone driver features.
doc-default:
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
    RUSTDOCFLAGS="-D warnings" cargo doc -p zencodec-io --no-deps --locked

# Check the published regression test with packaged seeds and an isolated build.
packaged-fuzz-regression:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo package --list -p zencodec --locked
    cargo package -p zencodec --locked
    package_metadata="$(cargo metadata --no-deps --format-version 1)"
    package_target="$(jq -r '.target_directory' <<< "$package_metadata")"
    package_version="$(jq -r '.packages[] | select(.name == "zencodec") | .version' <<< "$package_metadata")"
    CARGO_TARGET_DIR="$package_target/packaged-fuzz-tests" RUSTC_WRAPPER= \
        cargo test --manifest-path "$package_target/package/zencodec-$package_version/Cargo.toml" \
        --test fuzz_regression --locked

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
    python3 scripts/check-paired-semver.py zencodec 0.1.26 all

# Complete source diff from the cargo-read artifact, with VCS provenance checks.
contract-diff published_dir output revision="origin/main":
    python3 scripts/audit-published-contract.py "{{published_dir}}" "{{output}}" --revision "{{revision}}"

# Observational probes; these do not enshrine design gaps as test expectations.
contract-probes:
    cargo run -p zencodec --example contract_audit_probes

# Observational I/O defect probes plus exact output and stalled-writer checks.
io-contract-probes:
    cargo run -p zencodec-io --features tokio --example io_contract_probes

# Pinned external codecs and containers; use an export outside workspace patches.
io-validation-setup:
    python3 perf/io-validation/setup.py

io-validation-test:
    cargo test --manifest-path perf/io-validation/Cargo.toml --workspace --locked

io-validation-clippy:
    cargo clippy --manifest-path perf/io-validation/Cargo.toml -p io-validation --all-targets --locked -- -D warnings

io-validation-batching output cpus="2,3" suite="synthetic":
    bash perf/io-validation/run-batching.sh "{{output}}" "{{cpus}}" "{{suite}}"

io-validation-disk file cpus="2,3":
    IO_DISK_FILE="{{file}}" taskset -c "{{cpus}}" cargo bench --manifest-path perf/io-validation/Cargo.toml -p io-validation --bench disk --locked -- --cpu-policy=unrestricted --median-of-passes=3

# Linux: per-file eviction, verified using the process's kernel read-byte charge.
io-validation-disk-cold file cpus="2,3":
    IO_DISK_FILE="{{file}}" taskset -c "{{cpus}}" cargo bench --manifest-path perf/io-validation/Cargo.toml -p io-validation --bench disk_cold --locked -- --cpu-policy=unrestricted --median-of-passes=3

io-validation-disk-profile output file cpus="2,3":
    cargo build --manifest-path perf/io-validation/Cargo.toml -p io-validation --bin disk_probe --release --locked
    bash perf/io-validation/profile-disk.sh "{{output}}" "{{file}}" "{{cpus}}"

# Compile all disk comparison arms after source/error API changes.
io-disk-compile-check:
    cargo clippy --manifest-path perf/io-validation/Cargo.toml -p io-validation --bin disk_probe --bench disk --bench disk_cold --locked -- -D warnings

# Archive an explicitly collected evidence tree; never include Cargo caches.
io-validation-pack-evidence input output:
    python3 perf/io-validation/pack-evidence.py "{{input}}" "{{output}}"

io-validation-transport mp4 webm output:
    bash perf/io-validation/run-transport.sh "{{mp4}}" "{{webm}}" "{{output}}"

io-validation-oracle mp4 webm output:
    python3 perf/io-validation/strict-oracle.py "{{mp4}}" "{{webm}}" "{{output}}"

contract-semver-minimal:
    python3 scripts/check-paired-semver.py zencodec 0.1.26 none

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

metadata-tiff-tables:
    cargo test -p zenmeta --features inventory --test tiff_jpeg_tables
    cargo clippy -p zenmeta --all-features --all-targets -- -D warnings
    cargo +1.88.0 check -p zenmeta --no-default-features --features inventory
    cargo check -p zenmeta --no-default-features --features inventory --target wasm32-wasip1

# Adversarial review cases for the shared timing/rewrite boundary.
review-contract-check:
    cargo test -p zenmeta --features rewrite --test rewrite
    cargo test -p zencodec --test animation_timing

# Reusable contract checks and harness regression tests.
testkit-contract-check:
    cargo fmt -p zencodec-testkit
    cargo test -p zencodec-testkit
    cargo clippy -p zencodec-testkit --all-targets -- -D warnings

# Check the independently versioned testkit against its published baseline.
# Version 0.2.0 carries the approved implementor-only breaking changes.
testkit-semver:
    python3 scripts/check-paired-semver.py zencodec-testkit 0.1.0 default

# Checked target selection and ordered preferences, without encoding retries.
fidelity-check:
    cargo test -p zencodec-testkit --test fidelity_selection
    cargo clippy -p zencodec -p zencodec-testkit --all-targets -- -D warnings

# Independent frame/prefix oracles and typed refusal checks.
adversarial-check:
    cargo test -p zencodec-testkit --lib adversarial
    cargo clippy -p zencodec-testkit --all-targets -- -D warnings

# Instructions per decode for each io-driver shape (perf stat, two N points).
# Build once: cargo build --release -p zencodec-io --features tokio --example drvprobe
io-instr-probe cpu="4" lo="20000" hi="60000":
    #!/usr/bin/env bash
    set -e
    for m in direct slice pump at coro tokio; do
      for n in {{lo}} {{hi}}; do
        c=$(taskset -c {{cpu}} perf stat -x, -e instructions \
            ./target/release/examples/drvprobe "$m" "$n" 2>&1 \
            | awk -F, '/instructions/ {print $1}')
        echo "$m $n $c"
      done
    done

# Complete shared IO contract regression gate, including downstream testkit uses.
io-contract-check:
    cargo test --workspace --all-features --locked
    cargo clippy --workspace --all-targets --all-features --locked -- -D warnings

# Refresh the operation API and its conformance controls in an owned export.
operation-sync export:
    test ! -e "{{export}}/.git"
    test ! -e "{{export}}/.jj"
    mkdir -p "{{export}}/src/operation" "{{export}}/src/collection" "{{export}}/src/image"
    cp Cargo.toml "{{export}}/Cargo.toml"
    cp src/output.rs src/storage.rs src/output_patch.rs src/operation.rs src/lib.rs src/set.rs src/info.rs src/frame.rs src/frame_storage.rs src/image.rs src/resolution.rs src/collection.rs "{{export}}/src/"
    cp src/collection/*.rs "{{export}}/src/collection/"
    cp src/image/*.rs "{{export}}/src/image/"
    cp src/operation/*.rs "{{export}}/src/operation/"
    mkdir -p "{{export}}/src/operation/policy"
    cp src/operation/policy/*.rs "{{export}}/src/operation/policy/"
    cp src/traits/encoder.rs src/traits/encoding.rs src/traits/dyn_encoding.rs src/traits/mod.rs "{{export}}/src/traits/"
    cp zencodec-testkit/tests/operation_*.rs "{{export}}/zencodec-testkit/tests/"
    cp justfile "{{export}}/justfile"

# Native frame facts and policy publication guard share the still-image resolver.
frame-policy-sync export:
    just operation-sync "{{export}}"
    cp zencodec-media/Cargo.toml "{{export}}/zencodec-media/"
    cp zencodec-media/src/*.rs "{{export}}/zencodec-media/src/"
    cp zencodec-media-testkit/tests/storage_ownership.rs zencodec-media-testkit/tests/frame_policy.rs zencodec-media-testkit/tests/image_output.rs "{{export}}/zencodec-media-testkit/tests/"

frame-policy-check:
    cargo test -p zencodec-media-testkit --test frame_policy --locked
    cargo clippy -p zencodec -p zencodec-media -p zencodec-media-testkit --all-targets --locked -- -D warnings
    cargo check -p zencodec -p zencodec-media --no-default-features --locked

plane-view-check:
    cargo test -p zencodec --all-features --locked --lib frame_storage::tests
    cargo clippy -p zencodec -p zencodec-media -p zencodec-media-testkit --all-targets --locked -- -D warnings
    cargo check -p zencodec --no-default-features --target wasm32-unknown-unknown --locked
    just api-doc

compile-time-prepare root main ideal:
    python3 perf/compile-time.py prepare "{{root}}" "{{main}}" "{{ideal}}"

compile-adapters-prepare root main ideal:
    python3 perf/compile-time.py prepare-adapters "{{root}}" "{{main}}" "{{ideal}}"

compile-time-run zenbench root:
    "{{zenbench}}" pair "{{root}}/plan.json" --dry-run > "{{root}}/dry-run.json"
    "{{zenbench}}" pair "{{root}}/plan.json" --mode iterate --output "{{root}}/attempt"

compile-time-report root log receipt:
    python3 perf/compile-time-report.py "{{root}}" "{{log}}" "{{receipt}}"

# Pump, mapping, type-erasure and fused-session debugging contracts.
media-errors-sync export:
    just frame-policy-sync "{{export}}"
    cp zencodec-media-testkit/tests/error_chains.rs zencodec-media-testkit/tests/encoder_fusing.rs "{{export}}/zencodec-media-testkit/tests/"

media-errors-check:
    cargo test -p zencodec-media-testkit --locked
    cargo clippy -p zencodec-media -p zencodec-media-testkit --all-targets --locked -- -D warnings
    cargo check -p zencodec-media --no-default-features --target wasm32-unknown-unknown --locked
    just api-doc

frame-policy-integration:
    just frame-policy-check
    cargo test --workspace --all-features --locked
    just api-doc

# Standalone native AV1 policy guard, real re-encode and required FFmpeg oracle.
av1-policy-sync export:
    just frame-policy-sync "{{export}}"
    mkdir -p "{{export}}/perf/spatial-validation/src"
    cp perf/spatial-validation/src/image_items.rs perf/spatial-validation/src/image_output.rs "{{export}}/perf/spatial-validation/src/"
    mkdir -p "{{export}}/perf/av1-policy-validation/src"
    cp perf/av1-policy-validation/Cargo.toml perf/av1-policy-validation/verify_gamma.py perf/av1-policy-validation/verify_gamma_test.py "{{export}}/perf/av1-policy-validation/"
    cp perf/av1-policy-validation/src/*.rs "{{export}}/perf/av1-policy-validation/src/"
    mkdir -p "{{export}}/perf/av1-policy-validation/examples"
    cp perf/av1-policy-validation/examples/*.rs "{{export}}/perf/av1-policy-validation/examples/"
    mkdir -p "{{export}}/docs/examples"
    cp docs/examples/checked_video.rs "{{export}}/docs/examples/"

av1-policy-resolve:
    cargo generate-lockfile --manifest-path perf/av1-policy-validation/Cargo.toml

av1-policy-setup:
    cp perf/av1-policy-validation/dependencies.lock perf/av1-policy-validation/Cargo.lock

# Diagnostic: fails if distinct MP4 presentation programs lose information.
media-edit-list-probe:
    cargo run --manifest-path perf/av1-policy-validation/Cargo.toml --locked --example edit_list_probe

av1-policy-check artifacts:
    AV1_POLICY_ARTIFACTS="{{artifacts}}" cargo test --manifest-path perf/av1-policy-validation/Cargo.toml --locked -- --nocapture --test-threads=1
    cargo clippy --manifest-path perf/av1-policy-validation/Cargo.toml --all-targets --locked -- -D warnings
    just av1-gamma-verify "{{artifacts}}" "{{artifacts}}/independent-gamma.json"

# Native output transitions plus public build/documentation boundaries.
frame-route-check artifacts:
    just av1-policy-check "{{artifacts}}"
    cargo check -p zencodec -p zencodec-media --no-default-features --target wasm32-unknown-unknown --locked
    just doc-default

av1-policy-guide-check:
    cargo test --manifest-path perf/av1-policy-validation/Cargo.toml --locked guide_ -- --nocapture
    cargo clippy --manifest-path perf/av1-policy-validation/Cargo.toml --all-targets --locked -- -D warnings

av1-output-check artifacts:
    AV1_POLICY_ARTIFACTS="{{artifacts}}" cargo test --manifest-path perf/av1-policy-validation/Cargo.toml --locked output_native_ -- --nocapture --test-threads=1
    cargo clippy --manifest-path perf/av1-policy-validation/Cargo.toml --all-targets --locked -- -D warnings

av1-planar-check artifacts:
    AV1_POLICY_ARTIFACTS="{{artifacts}}" cargo test --manifest-path perf/av1-policy-validation/Cargo.toml --locked planar_ -- --nocapture --test-threads=1
    cargo clippy --manifest-path perf/av1-policy-validation/Cargo.toml --all-targets --locked -- -D warnings

# Complete qualification of the public output and its native consumers.
image-output-integration artifacts python:
    just io-contract-check
    just spatial-validation-check "{{artifacts}}-still" "{{python}}"
    just av1-output-check "{{artifacts}}-av1"
    cargo check -p zencodec --no-default-features --target wasm32-unknown-unknown --locked
    just doc-default
    just api-doc

# Public mapped output rejects native geometry/encoding disagreement.
image-output-check:
    cargo test -p zencodec-media-testkit --test image_output --locked
    cargo clippy -p zencodec -p zencodec-media-testkit --all-targets --locked -- -D warnings
    cargo check -p zencodec --no-default-features --target wasm32-unknown-unknown --locked

# Resolve the local workspace graph before its locked API gates.
image-output-api-setup:
    cargo metadata --format-version 1 > /dev/null

# Validate shared image output and refresh its canonical API documentation.
image-output-api-check:
    just image-output-check
    just doc-default
    just api-doc

# Empty geometries must remain safe through the borrowed native-plane API.
frame-storage-edge-check:
    cargo test -p zencodec --locked frame_storage::tests::empty_width_rows_remain_empty_with_any_stride
    cargo clippy -p zencodec -p zencodec-media --all-targets --locked -- -D warnings

# Verify the same core mapping contract with native still and AV1 owners.
shared-output-check artifacts:
    just spatial-output-check "{{artifacts}}-still"
    just av1-output-check "{{artifacts}}-av1"
    cargo check -p zencodec -p zencodec-media --no-default-features --target wasm32-unknown-unknown --locked

# Native TIFF spatial candidates and the positional-to-seek bridge.
spatial-validation-sync export:
    mkdir -p "{{export}}/zencodec-render/src"
    cp zencodec-render/Cargo.toml zencodec-render/dependencies.lock "{{export}}/zencodec-render/"
    cp zencodec-render/src/*.rs "{{export}}/zencodec-render/src/"
    mkdir -p "{{export}}/zencodec-render/tests"
    cp zencodec-render/tests/*.rs "{{export}}/zencodec-render/tests/"
    just io-core-sync "{{export}}"
    cp zenmeta/src/ifd/walk.rs "{{export}}/zenmeta/src/ifd/"
    cp zencodec-media/Cargo.toml "{{export}}/zencodec-media/"
    cp zencodec-media/src/*.rs "{{export}}/zencodec-media/src/"
    test ! -e "{{export}}/.git"
    test ! -e "{{export}}/.jj"
    mkdir -p "{{export}}/perf/spatial-validation/src"
    cp perf/spatial-validation/Cargo.toml "{{export}}/perf/spatial-validation/"
    cp perf/spatial-validation/dependencies.lock "{{export}}/perf/spatial-validation/Cargo.lock"
    cp perf/spatial-validation/oracle.py "{{export}}/perf/spatial-validation/"
    cp perf/spatial-validation/jxl_oracle.c perf/spatial-validation/layer_oracle.c "{{export}}/perf/spatial-validation/"
    cp perf/spatial-validation/build_jxl_oracles.py "{{export}}/perf/spatial-validation/"
    cp perf/spatial-validation/color_oracle.c perf/spatial-validation/build_color_oracle.py perf/spatial-validation/inspect_color_difference.py "{{export}}/perf/spatial-validation/"
    cp perf/spatial-validation/http_snapshot.py perf/spatial-validation/http_snapshot_test.py "{{export}}/perf/spatial-validation/"
    cp perf/spatial-validation/src/*.rs "{{export}}/perf/spatial-validation/src/"
    mkdir -p "{{export}}/docs/examples"
    cp docs/examples/checked_source.rs "{{export}}/docs/examples/"
    cp src/error.rs "{{export}}/src/error.rs"
    cp justfile "{{export}}/justfile"

spatial-layer-resolve:
    cargo update --manifest-path perf/spatial-validation/Cargo.toml -p zenjxl-decoder

spatial-tiff-resolve:
    cargo update --manifest-path perf/spatial-validation/Cargo.toml -p tiff

spatial-tiff-inventory-check artifacts:
    SPATIAL_INVENTORY_ARTIFACTS="{{artifacts}}" cargo test --manifest-path perf/spatial-validation/Cargo.toml --features old_jpeg --locked tiff_inventory_ -- --nocapture --test-threads=1
    cargo clippy --manifest-path perf/spatial-validation/Cargo.toml --features old_jpeg --all-targets --locked -- -D warnings

spatial-validation-resolve:
    cargo metadata --manifest-path perf/spatial-validation/Cargo.toml --format-version 1 > /dev/null

spatial-validation-setup:
    cp perf/spatial-validation/dependencies.lock perf/spatial-validation/Cargo.lock

collection-api-check:
    cargo test -p zencodec --lib collection_ --locked
    cargo clippy -p zencodec --lib --locked -- -D warnings
    cargo check -p zencodec --no-default-features --target wasm32-unknown-unknown --locked

# A genuinely freestanding target; WASM with std installed is not this gate.
collection-api-bare-metal:
    cargo check -p zencodec -p zencodec-io -p zencodec-coroutine -p zencodec-media -p zenmeta --lib --no-default-features --target thumbv7em-none-eabihf --locked

collection-api-resolve:
    cargo update -p zencodec
    cargo update --manifest-path perf/spatial-validation/Cargo.toml -p zencodec

# Run in a standalone source export so workspace path overrides cannot hide
# dependency splits. Retain every committed codec pin while updating core edges.
collection-validation-resolve:
    #!/usr/bin/env bash
    set -euo pipefail
    test ! -e .git
    test ! -e .jj
    cp perf/io-validation/locks/workspace.lock Cargo.lock
    cargo update -p zencodec
    for package in policy-check jpeg-policy-validation av1-policy-validation spatial-validation; do
      cp "perf/$package/dependencies.lock" "perf/$package/Cargo.lock"
      cargo update --manifest-path "perf/$package/Cargo.toml" -p zencodec
    done
    cp perf/io-validation/locks/harness.lock perf/io-validation/Cargo.lock
    cargo update --manifest-path perf/io-validation/Cargo.toml -p zencodec

collection-guide-check:
    cargo test --manifest-path perf/spatial-validation/Cargo.toml --locked guide_ -- --nocapture --test-threads=1
    cargo clippy --manifest-path perf/spatial-validation/Cargo.toml --all-targets --locked -- -D warnings

spatial-oracle-install python:
    uv pip install --python "{{python}}" --only-binary :all: Pillow==13.0.0

spatial-jxl-oracle-build output:
    python3 perf/spatial-validation/build_jxl_oracles.py "{{output}}"

# Recompile test helpers against an already pinned independent library. Preserve
# old helpers and evidence; both helper binaries require a new output directory.
spatial-jxl-oracle-relink pinned output:
    #!/usr/bin/env python3
    import hashlib, json, pathlib, subprocess
    pinned, output = pathlib.Path("{{pinned}}").resolve(), pathlib.Path("{{output}}").resolve()
    receipt = json.loads((pinned / "provenance.json").read_text())
    assert receipt["runtime_version"] == 11001
    assert receipt["source_sha256"] == "1492dfef8dd6c3036446ac3b340005d92ab92f7d48ee3271b5dac1d36945d3d9"
    output.mkdir(parents=True, exist_ok=False)
    library = pinned / "build/lib"
    for helper in ["jxl", "layer"]:
        source = pathlib.Path(f"perf/spatial-validation/{helper}_oracle.c").resolve()
        command = ["cc", "-std=c11", "-Wall", "-Wextra", "-Werror", "-I", str(pinned / "build/lib/include"), "-I", str(pinned / "libjxl-0.11.1/lib/include"), str(source), "-L", str(library), f"-Wl,-rpath,{library}", "-ljxl_dec", "-o", str(output / f"{helper}-oracle")]
        subprocess.run(command, check=True, timeout=60)
    assert subprocess.check_output([str(output / "layer-oracle"), "--version"], timeout=5).strip() == b"11001"
    receipt["helpers"] = {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in output.glob("*-oracle")}
    receipt["helper_sources"] = {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in pathlib.Path("perf/spatial-validation").glob("*_oracle.c")}
    receipt["reused_library"] = {str(p.resolve()): hashlib.sha256(p.read_bytes()).hexdigest() for p in library.glob("libjxl_dec.so.*") if not p.is_symlink()}
    assert receipt["reused_library"]
    (output / "provenance.json").write_text(json.dumps(receipt, indent=2) + "\n")

spatial-projection-check artifacts:
    #!/usr/bin/env bash
    set -euo pipefail
    : "${JXL_SELECTION_ORACLE:?supply current pinned libjxl helper}"
    export SPATIAL_PROJECTION_ARTIFACTS="{{artifacts}}"
    cargo test --manifest-path perf/spatial-validation/Cargo.toml --locked canvas_projection -- --nocapture --test-threads=1
    cargo clippy --manifest-path perf/spatial-validation/Cargo.toml --all-targets --locked -- -D warnings

# Unified held-image color/geometry/publication candidate; native TIFF and JXL.
spatial-render-check:
    cp zencodec-render/dependencies.lock zencodec-render/Cargo.lock
    cargo fmt --manifest-path perf/spatial-validation/Cargo.toml -p spatial-validation
    cargo test --manifest-path perf/spatial-validation/Cargo.toml --locked canvas_render -- --nocapture --test-threads=1
    just spatial-validation-lint

spatial-render-oracle-check artifacts:
    #!/usr/bin/env bash
    set -euo pipefail
    : "${SPATIAL_COLOR_ORACLE:?supply compiled independent Little CMS reader}"
    export SPATIAL_COLOR_ARTIFACTS="{{artifacts}}"
    cargo test --manifest-path perf/spatial-validation/Cargo.toml --features cms_oracle --locked canvas_render -- --nocapture --test-threads=1
    just spatial-validation-lint cms_oracle

# Reproduce a raw-layer oracle disagreement with explicit headers and libraries.
# Native captures are reused; this does not rerun or time the Rust decoder.
spatial-layer-oracle-compare captured native output headers="/usr/include" libdir="/usr/lib/x86_64-linux-gnu" library="jxl":
    mkdir -p "{{output}}"
    cc -std=c11 -Wall -Wextra -Werror -I "{{headers}}" perf/spatial-validation/layer_oracle.c -L "{{libdir}}" -Wl,-rpath,"{{libdir}}" -l"{{library}}" -o "{{output}}/layer-oracle"
    python3 "{{native}}/perf/raw-layer-validation/compare.py" "{{captured}}" "{{native}}/zenjxl-decoder/resources/test" "{{output}}/layer-oracle" "{{output}}"

# The 0.7 noise regression reproducer builds only the independent decoder.
spatial-old-oracle-build source build:
    cmake -S "{{source}}" -B "{{build}}" -G Ninja -DCMAKE_BUILD_TYPE=Release -DBUILD_TESTING=OFF -DJPEGXL_ENABLE_TOOLS=OFF -DJPEGXL_ENABLE_DOXYGEN=OFF -DJPEGXL_ENABLE_MANPAGES=OFF -DJPEGXL_ENABLE_BENCHMARK=OFF -DJPEGXL_ENABLE_EXAMPLES=OFF -DJPEGXL_ENABLE_JNI=OFF -DJPEGXL_ENABLE_SJPEG=OFF -DJPEGXL_ENABLE_OPENEXR=OFF -DJPEGXL_ENABLE_SKCMS=OFF -DJPEGXL_ENABLE_TCMALLOC=OFF -DJPEGXL_ENABLE_PLUGINS=OFF -DJPEGXL_FORCE_SYSTEM_HWY=ON -DJPEGXL_FORCE_SYSTEM_LCMS2=ON -DJPEGXL_FORCE_SYSTEM_BROTLI=ON
    cmake --build "{{build}}" --target jxl_dec --parallel 2

spatial-validation-check artifacts python features="":
    mkdir -p "{{artifacts}}-oracle"
    cc -std=c11 -Wall -Wextra -Werror perf/spatial-validation/layer_oracle.c -ljxl -o "{{artifacts}}-oracle/layer-oracle"
    cc -std=c11 -Wall -Wextra -Werror perf/spatial-validation/jxl_oracle.c -ljxl -o "{{artifacts}}-oracle/jxl-oracle"
    just spatial-validation-with-oracles "{{artifacts}}" "{{python}}" "{{artifacts}}-oracle" "{{features}}"

spatial-validation-with-oracles artifacts python oracles features="":
    JXL_LAYER_ORACLE="{{oracles}}/layer-oracle" SPATIAL_LAYER_ARTIFACTS="{{artifacts}}-layers" SPATIAL_ARTIFACTS="{{artifacts}}" SPATIAL_GENERATED_ARTIFACTS="{{artifacts}}-generated" SPATIAL_PROJECTION_ARTIFACTS="{{artifacts}}-projection" SPATIAL_ORACLE_PYTHON="{{python}}" JXL_SELECTION_ORACLE="{{oracles}}/jxl-oracle" cargo test --manifest-path perf/spatial-validation/Cargo.toml --features "{{features}}" --locked -- --nocapture --test-threads=1
    just spatial-validation-lint "{{features}}"

spatial-validation-lint features="":
    cargo clippy --manifest-path perf/spatial-validation/Cargo.toml --features "{{features}}" --all-targets --locked -- -D warnings

# Start the real HTTP simulator and keep each fixture family in its own namespace.
spatial-validation-all artifacts python port="3568":
    just spatial-http-lifetime-check
    just spatial-validation-check "{{artifacts}}" "{{python}}" file_snapshot
    just spatial-http-check "{{artifacts}}-http" "{{port}}"

spatial-consumer-check:
    cargo test --manifest-path perf/spatial-validation/Cargo.toml --locked common_ -- --nocapture
    cargo clippy --manifest-path perf/spatial-validation/Cargo.toml --all-targets --locked -- -D warnings

spatial-cancellation-check filter="cancellation_":
    cargo test --manifest-path perf/spatial-validation/Cargo.toml --locked "{{filter}}" -- --nocapture --test-threads=1
    cargo clippy --manifest-path perf/spatial-validation/Cargo.toml --all-targets --locked -- -D warnings

# One operation-wide call allowance across native discovery and cached cursors.
spatial-discovery-check:
    cargo test --manifest-path perf/spatial-validation/Cargo.toml --locked discovery_budget_ -- --nocapture --test-threads=1
    cargo clippy --manifest-path perf/spatial-validation/Cargo.toml --all-targets --locked -- -D warnings

# Candidate descriptions must not reconstruct pixels or bypass cancellation.
spatial-description-check:
    cargo test --manifest-path perf/spatial-validation/Cargo.toml --locked description_ -- --nocapture --test-threads=1
    just spatial-validation-lint

# Real native factories share one checked consumer while preserving source modes.
spatial-factories-check artifacts:
    SPATIAL_ARTIFACTS="{{artifacts}}" cargo test --manifest-path perf/spatial-validation/Cargo.toml --locked collection_factory_ -- --nocapture --test-threads=1
    just spatial-validation-lint

# Exact native pins are in the manifest; callers supply pinned independent readers.
spatial-packaged-resolve:
    cargo update --manifest-path perf/spatial-validation/Cargo.toml
    cp perf/spatial-validation/Cargo.lock perf/spatial-validation/dependencies.lock

spatial-packaged-check artifacts:
    #!/usr/bin/env bash
    set -euo pipefail
    : "${JXL_SELECTION_ORACLE:?supply the pinned libjxl presentation reader}"
    : "${JXL_LAYER_ORACLE:?supply the pinned libjxl raw-layer reader}"
    export SPATIAL_ARTIFACTS="{{artifacts}}/factories"
    export SPATIAL_LAYER_ARTIFACTS="{{artifacts}}/layers"
    mkdir -p "{{artifacts}}"
    cargo test --manifest-path perf/spatial-validation/Cargo.toml --locked collection_factory_ -- --nocapture --test-threads=1
    cargo test --manifest-path perf/spatial-validation/Cargo.toml --locked selected_jxl_presentations_match_native_sequential_with_dependencies -- --nocapture
    cargo test --manifest-path perf/spatial-validation/Cargo.toml --locked raw_collection_native_samples_and_owner_bound_policy_match_independent_layers -- --nocapture
    cargo clippy --manifest-path perf/spatial-validation/Cargo.toml --all-targets --locked -- -D warnings

# Native option scope is qualified separately from shared resource requirements.
spatial-admission-check artifacts:
    SPATIAL_LAYER_ARTIFACTS="{{artifacts}}" cargo test --manifest-path perf/spatial-validation/Cargo.toml --locked native_jxl_admission_ -- --nocapture --test-threads=1
    just spatial-validation-lint

spatial-owned-check:
    cargo test --manifest-path perf/spatial-validation/Cargo.toml --locked owned_jxl_output_ -- --nocapture --test-threads=1
    just spatial-validation-lint

# Real profile-bearing JXL output through zenpixels-convert's public finalizer.
spatial-conversion-check:
    cargo fmt --manifest-path perf/spatial-validation/Cargo.toml -p spatial-validation
    cargo test --manifest-path perf/spatial-validation/Cargo.toml --locked native_cmyk_finalization_ -- --nocapture --test-threads=1
    just spatial-validation-lint

spatial-output-plan-check:
    cargo fmt --manifest-path perf/spatial-validation/Cargo.toml -p spatial-validation
    cargo test --manifest-path perf/spatial-validation/Cargo.toml --locked native_output_ -- --nocapture --test-threads=1
    just spatial-validation-lint

# Test-only independent F32 ICC math. All oracle inputs/outputs stay in artifacts.
spatial-color-oracle output:
    python3 perf/spatial-validation/build_color_oracle.py "{{output}}"

# Inspect one saved worst sample against matrix/type-3 ICC math.
spatial-color-inspect selection:
    python3 perf/spatial-validation/inspect_color_difference.py "{{selection}}"

spatial-color-check artifacts:
    #!/usr/bin/env bash
    set -euo pipefail
    : "${SPATIAL_COLOR_ORACLE:?supply compiled independent Little CMS reader}"
    export SPATIAL_COLOR_ARTIFACTS="{{artifacts}}"
    cargo fmt --manifest-path perf/spatial-validation/Cargo.toml -p spatial-validation
    cargo test --manifest-path perf/spatial-validation/Cargo.toml --features cms_oracle --locked native_color_ -- --nocapture --test-threads=1
    cargo clippy --manifest-path perf/spatial-validation/Cargo.toml --features cms_oracle --all-targets --locked -- -D warnings

image-owner-check:
    cargo test -p zencodec --all-features --lib packed_owner_ --locked
    cargo clippy -p zencodec --all-features --lib --locked -- -D warnings

image-color-replacement-check:
    cargo fmt -p zencodec
    cargo test -p zencodec --test packed_color_replacement --locked -- --nocapture
    cargo clippy -p zencodec --all-features --lib --test packed_color_replacement --locked -- -D warnings

image-color-replacement-integration:
    just image-color-replacement-check
    just image-owner-integration

spatial-color-owner-check:
    cargo fmt --manifest-path perf/spatial-validation/Cargo.toml -p spatial-validation
    cargo test --manifest-path perf/spatial-validation/Cargo.toml --locked native_output_owner_ -- --nocapture --test-threads=1

# Compare real owner payloads while requiring identical samples/profiles/oracles.
spatial-retention-receipt baseline current log output:
    python3 perf/spatial-validation/record_retention.py "{{baseline}}" "{{current}}" "{{log}}" "{{output}}"

image-owner-integration:
    just io-contract-check
    cargo check -p zencodec --no-default-features --target wasm32-unknown-unknown --locked
    just doc-default
    just api-doc

auxiliary-grid-check:
    cargo test -p zencodec --test auxiliary_registration --test packed_color_replacement
    just image-owner-integration

# Core requirement forwarding, ownership and every workspace contract.
collection-context-integration:
    cargo fmt -p zencodec
    just image-owner-integration

collection-context-native artifacts:
    cargo fmt --manifest-path perf/spatial-validation/Cargo.toml -p spatial-validation
    SPATIAL_ARTIFACTS="{{artifacts}}" cargo test --manifest-path perf/spatial-validation/Cargo.toml --locked collection_factory_ -- --nocapture --test-threads=1
    just spatial-validation-lint

# Same complete native suite and test timeout as the documentation workflow.
spatial-ci-check artifacts features="":
    #!/usr/bin/env bash
    set -euo pipefail
    : "${SPATIAL_ORACLE_PYTHON:?supply pinned Pillow/libtiff reader}"
    : "${JXL_SELECTION_ORACLE:?supply pinned libjxl presentation reader}"
    : "${JXL_LAYER_ORACLE:?supply pinned libjxl layer reader}"
    export SPATIAL_ARTIFACTS="{{artifacts}}/native"
    export SPATIAL_GENERATED_ARTIFACTS="{{artifacts}}/generated"
    export SPATIAL_LAYER_ARTIFACTS="{{artifacts}}/layers"
    export SPATIAL_COLOR_ARTIFACTS="{{artifacts}}/color"
    export SPATIAL_PROJECTION_ARTIFACTS="{{artifacts}}/projection"
    export SPATIAL_INVENTORY_ARTIFACTS="{{artifacts}}/inventory"
    mkdir -p "{{artifacts}}"
    timeout --kill-after=15s 10m cargo test --manifest-path perf/spatial-validation/Cargo.toml --features "{{features}}" --locked -- --nocapture --test-threads=1
    timeout --kill-after=15s 5m cargo clippy --manifest-path perf/spatial-validation/Cargo.toml --features "{{features}}" --all-targets --locked -- -D warnings

# Automatic selected-image publication policy, including native failure seams.
spatial-checked-collection-check:
    cargo test --manifest-path perf/spatial-validation/Cargo.toml --locked checked_collection_ -- --nocapture --test-threads=1
    cargo clippy --manifest-path perf/spatial-validation/Cargo.toml --all-targets --locked -- -D warnings

# Caller explicitly selects Linux/reflink qualification; no unsupported-FS skips.
spatial-file-snapshot-check artifacts:
    SPATIAL_ARTIFACTS="{{artifacts}}" cargo test --manifest-path perf/spatial-validation/Cargo.toml --features file_snapshot --locked file_snapshot_ -- --nocapture --test-threads=1
    cargo clippy --manifest-path perf/spatial-validation/Cargo.toml --features file_snapshot --all-targets --locked -- -D warnings

spatial-file-snapshot-resolve:
    cargo update --manifest-path perf/spatial-validation/Cargo.toml -p rustix --precise 1.1.5

# Native selection over IMI-derived real HTTP, with deliberate response faults.
spatial-http-check artifacts port="3568":
    python3 perf/spatial-validation/http_snapshot.py "{{artifacts}}" "{{port}}" -- cargo test --manifest-path perf/spatial-validation/Cargo.toml --locked --features http http_snapshot_ -- --nocapture --test-threads=1
    cargo clippy --manifest-path perf/spatial-validation/Cargo.toml --all-targets --features http --locked -- -D warnings

spatial-http-lifetime-check:
    python3 perf/spatial-validation/http_snapshot_test.py

spatial-http-resolve:
    cargo update --manifest-path perf/spatial-validation/Cargo.toml -p ureq

spatial-layer-check artifacts:
    mkdir -p "{{artifacts}}-oracle"
    cc -std=c11 -Wall -Wextra -Werror perf/spatial-validation/layer_oracle.c -ljxl -o "{{artifacts}}-oracle/layer-oracle"
    SPATIAL_LAYER_ARTIFACTS="{{artifacts}}-layers" JXL_LAYER_ORACLE="{{artifacts}}-oracle/layer-oracle" cargo test --manifest-path perf/spatial-validation/Cargo.toml --locked raw_collection_ -- --nocapture --test-threads=1
    cc -std=c11 -Wall -Wextra -Werror perf/spatial-validation/jxl_oracle.c -ljxl -o "{{artifacts}}-oracle/jxl-oracle"
    JXL_SELECTION_ORACLE="{{artifacts}}-oracle/jxl-oracle" cargo test --manifest-path perf/spatial-validation/Cargo.toml --locked jxl_native_coalescing_ -- --nocapture
    cargo clippy --manifest-path perf/spatial-validation/Cargo.toml --all-targets --locked -- -D warnings

# Compile the source-extracted layer guide and execute it with a real CMYK file.
spatial-layer-guide-check:
    cargo test --manifest-path perf/spatial-validation/Cargo.toml --locked raw_collection_guide_ -- --nocapture
    cargo clippy --manifest-path perf/spatial-validation/Cargo.toml --all-targets --locked -- -D warnings

spatial-placement-check artifacts:
    mkdir -p "{{artifacts}}-oracle"
    cc -std=c11 -Wall -Wextra -Werror perf/spatial-validation/jxl_oracle.c -ljxl -o "{{artifacts}}-oracle/jxl-oracle"
    JXL_SELECTION_ORACLE="{{artifacts}}-oracle/jxl-oracle" cargo test --manifest-path perf/spatial-validation/Cargo.toml --locked placement_ -- --nocapture
    cargo clippy --manifest-path perf/spatial-validation/Cargo.toml --all-targets --locked -- -D warnings

spatial-output-check artifacts:
    mkdir -p "{{artifacts}}-oracle"
    cc -std=c11 -Wall -Wextra -Werror perf/spatial-validation/jxl_oracle.c -ljxl -o "{{artifacts}}-oracle/jxl-oracle"
    SPATIAL_GENERATED_ARTIFACTS="{{artifacts}}-generated" JXL_SELECTION_ORACLE="{{artifacts}}-oracle/jxl-oracle" cargo test --manifest-path perf/spatial-validation/Cargo.toml --locked output_ -- --nocapture
    cargo test --manifest-path perf/spatial-validation/Cargo.toml --locked strip_tests -- --nocapture
    cargo clippy --manifest-path perf/spatial-validation/Cargo.toml --all-targets --locked -- -D warnings

spatial-subsampling-check artifacts:
    mkdir -p "{{artifacts}}-oracle"
    cc -std=c11 -Wall -Wextra -Werror perf/spatial-validation/jxl_oracle.c -ljxl -o "{{artifacts}}-oracle/jxl-oracle"
    SPATIAL_GENERATED_ARTIFACTS="{{artifacts}}-generated" JXL_SELECTION_ORACLE="{{artifacts}}-oracle/jxl-oracle" cargo test --manifest-path perf/spatial-validation/Cargo.toml --locked output_generated_subsampled_ -- --nocapture
    cargo clippy --manifest-path perf/spatial-validation/Cargo.toml --all-targets --locked -- -D warnings

# Subsampled source extras remain distinct from their emitted registration.
spatial-auxiliary-grid-check artifacts:
    SPATIAL_GENERATED_ARTIFACTS="{{artifacts}}" cargo test --manifest-path perf/spatial-validation/Cargo.toml --locked output_generated_subsampled_ -- --nocapture --test-threads=1
    just spatial-validation-lint

# Real TIFF sample carriage through the common pixel owner and row converter.
spatial-samples-check:
    cargo test --manifest-path perf/spatial-validation/Cargo.toml --locked strip_tests -- --nocapture
    cargo test --manifest-path perf/spatial-validation/Cargo.toml --locked tiff_pixels -- --nocapture
    cargo test --manifest-path perf/spatial-validation/Cargo.toml --locked common_ -- --nocapture
    cargo clippy --manifest-path perf/spatial-validation/Cargo.toml --all-targets --locked -- -D warnings

# Typed and erased planned jobs share the complete destination driver.
planned-row-check:
    cargo test -p zencodec-io --features std --locked
    cargo clippy -p zencodec-io --features std --all-targets --locked -- -D warnings
    cargo test -p zencodec-io --all-features --tests --locked
    cargo clippy -p zencodec-io --all-features --all-targets --locked -- -D warnings

# Configured job erasure preserves the retained owner before native startup.
configured-job-check:
    cargo test -p zencodec --all-features --lib traits::dyn_encoding::job_erasure_tests --locked
    cargo clippy -p zencodec --all-features --all-targets --locked -- -D warnings

# Operation-bound requirements, callback composition and diagnostic controls.
policy-binding-check:
    cargo test -p zencodec-testkit --test policy_binding --locked
    cargo clippy -p zencodec-testkit --test policy_binding --locked -- -D warnings

# Early resource deadlines share the same predicate as joint route admission.
policy-workspace-check:
    cargo test -p zencodec --lib operation::policy::requirements::tests --locked
    cargo test -p zencodec-testkit --test policy_binding --locked
    cargo clippy -p zencodec --all-targets --locked -- -D warnings

# Independent review probes, kept outside the product dependency graph.
api-usability-runner-check:
    timeout 60s bash perf/api-usability-validation/head-replay-controls.sh

api-usability-replay artifacts revision:
    bash perf/api-usability-validation/head-replay.sh "{{artifacts}}" "{{revision}}"

operation-policy-binding-check:
    cargo test --workspace --all-features --locked
    cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
    cargo test --manifest-path perf/api-usability-validation/Cargo.toml --locked
    cargo clippy --manifest-path perf/api-usability-validation/Cargo.toml --all-targets --locked -- -D warnings

operation-policy-check:
    cargo test -p zencodec-testkit --test operation_policy --test operation_requests --locked
    cargo test -p zencodec --lib operation:: --locked

resolution-core-check:
    cargo fmt -p zencodec -p zencodec-testkit --check
    cargo test -p zencodec --lib resolution:: --locked
    cargo test -p zencodec-testkit --test operation_resolution --locked
    just operation-policy-check
    cargo clippy -p zencodec --all-targets --locked -- -D warnings
    cargo clippy -p zencodec-testkit --test operation_resolution --locked -- -D warnings
    cargo check -p zencodec --no-default-features --locked

storage-sync export:
    just io-core-sync "{{export}}"
    cp zencodec-media/src/frame.rs zencodec-media/src/packet.rs "{{export}}/zencodec-media/src/"
    cp zencodec-media-testkit/tests/storage_ownership.rs "{{export}}/zencodec-media-testkit/tests/"

storage-cost-sync export:
    test ! -e "{{export}}/.git"
    test ! -e "{{export}}/.jj"
    cp perf/io-validation/benches/admission.rs "{{export}}/perf/io-validation/benches/"
    cp justfile "{{export}}/justfile"

storage-cost-check:
    cargo clippy --manifest-path perf/io-validation/Cargo.toml -p io-validation --bench admission --locked -- -D warnings

storage-cost-bench cpu="4":
    taskset -c {{cpu}} cargo bench --manifest-path perf/io-validation/Cargo.toml -p io-validation --bench admission --locked -- --cpu-policy=unrestricted --median-of-passes=3

coroutine-cost-sync export:
    just control-sync "{{export}}"
    cp zencodec-io/Cargo.toml "{{export}}/zencodec-io/"
    cp zencodec-io/benches/coroutine_admission.rs zencodec-io/benches/fixture.rs "{{export}}/zencodec-io/benches/"

# One sink loan per event, including retained final/provisional batches.
coroutine-row-delivery-check:
    cargo test -p zencodec-coroutine --locked
    cargo clippy -p zencodec-coroutine --all-targets --locked -- -D warnings

coroutine-cost-check:
    cargo clippy -p zencodec-io --bench coroutine_admission --locked -- -D warnings

coroutine-cost-bench cpu="4":
    taskset -c {{cpu}} cargo bench -p zencodec-io --bench coroutine_admission --locked -- --cpu-policy=unrestricted --median-of-passes=3

# Refresh an owned validation export without inheriting parent workspace patches.
io-core-sync export:
    test ! -e "{{export}}/.git"
    test ! -e "{{export}}/.jj"
    test -f "{{export}}/zencodec-io/Cargo.toml"
    cp Cargo.toml "{{export}}/Cargo.toml"
    cp src/lib.rs src/set.rs src/source.rs src/storage.rs src/frame.rs src/frame_storage.rs src/image.rs src/collection.rs src/operation.rs "{{export}}/src/"
    mkdir -p "{{export}}/src/collection" "{{export}}/src/operation" "{{export}}/src/image" "{{export}}/src/set"
    cp src/set/*.rs "{{export}}/src/set/"
    cp src/collection/*.rs "{{export}}/src/collection/"
    cp src/image/*.rs "{{export}}/src/image/"
    cp src/operation/*.rs "{{export}}/src/operation/"
    mkdir -p "{{export}}/src/operation/policy"
    cp src/operation/policy/*.rs "{{export}}/src/operation/policy/"
    cp zencodec-io/src/*.rs "{{export}}/zencodec-io/src/"
    cp tests/packed_color_replacement.rs "{{export}}/tests/"
    cp zencodec-io/tests/*.rs "{{export}}/zencodec-io/tests/"
    cp justfile "{{export}}/justfile"

# Native output parity under independently configured compute quotas.
io-quota-native-check:
    cargo test --manifest-path perf/io-validation/Cargo.toml -p io-validation --test encoder_retention --locked
    cargo clippy --manifest-path perf/io-validation/Cargo.toml -p io-validation --test encoder_retention --locked -- -D warnings

# Default row admission agrees for typed, shimmed and handwritten dyn encoders.
encoder-begin-check:
    cargo test -p zencodec --all-features --locked --lib begin_dispatch_tests

# Concrete codec causes, trace wrappers and nested causes survive erasure.
error-cause-check:
    cargo test -p zencodec --all-features --locked --lib error::tests

# Standard parser bridge and native source error semantics.
io-reader-check:
    cargo test -p zencodec-io --all-features --locked
    cargo clippy -p zencodec-io --all-targets --all-features --locked -- -D warnings
    cargo check -p zencodec-io --no-default-features --locked

# Profile the actual per-presentation resolver and bounded diagnostic report.
frame-cost-sync export:
    just frame-policy-sync "{{export}}"
    cp Cargo.toml "{{export}}/Cargo.toml"
    cp perf/frame-policy-cost.rs "{{export}}/perf/"
    cp benches/frame_policy.rs "{{export}}/benches/"
    cp examples/frame_policy_probe.rs "{{export}}/examples/"

frame-cost-check:
    cargo run --example frame_policy_probe --locked -- record 1000
    cargo run --example frame_policy_probe --locked -- resolve 1000
    cargo run --example frame_policy_probe --locked -- combined 1000
    cargo clippy -p zencodec --bench frame_policy --example frame_policy_probe --locked -- -D warnings

# Rust value layout, distinct from process memory and throughput measurement.
policy-layout-check:
    cargo run --example frame_policy_probe --locked -- layout

frame-cost-build:
    CARGO_PROFILE_RELEASE_DEBUG=1 CARGO_PROFILE_RELEASE_LTO=thin cargo build --release --example frame_policy_probe --locked
    CARGO_PROFILE_BENCH_DEBUG=1 CARGO_PROFILE_BENCH_LTO=thin cargo bench --bench frame_policy --no-run --locked

frame-cost-bench cpus="2,3":
    CARGO_PROFILE_BENCH_DEBUG=1 CARGO_PROFILE_BENCH_LTO=thin taskset -c "{{cpus}}" cargo bench --bench frame_policy --locked -- --cpu-policy=unrestricted --median-of-passes=3

# Run saved same-workload executables without rebuilding either side.
frame-cost-saved-bench executable cpus="2,3":
    test -x "{{executable}}"
    taskset -c "{{cpus}}" "{{executable}}" --cpu-policy=unrestricted --median-of-passes=3

frame-cost-saved-profile executable output mode="record" count="1000000" cpus="2,3":
    test -x "{{executable}}"
    test ! -e "{{output}}"
    taskset -c "{{cpus}}" perf stat -x, -o "{{output}}" -e instructions,cycles,cache-misses,context-switches "{{executable}}" "{{mode}}" "{{count}}"

# Fixed confirmation plan. Native hardware classification and busy gates stay on.
# PIN selects the measured thread; the OS process boundary must be a known class.
frame-cost-saved-confirm executable output cpus="2,3" pin="2":
    test -x "{{executable}}"
    test ! -e "{{output}}"
    ZENBENCH_PIN="{{pin}}" ZENBENCH_RESULT_PATH="{{output}}" taskset -c "{{cpus}}" "{{executable}}" --mode=confirm --cpu-policy=auto --workload-id=zencodec-policy-history-v1 --scope=zen4-policy-history --processes=5 --rounds=30 --margin=1 --confidence=0.95 --timeout=120

# Explicitly coordinated benchmark window. Restore normal priority only for
# this child; the caller still supplies run-heavy's memory cap and a deadline.
frame-cost-priority-confirm executable output cpus="2,3" pin="2":
    #!/usr/bin/env bash
    set -euo pipefail
    sudo -n renice -n 0 -p "$$"
    exec just --justfile "{{justfile()}}" --working-directory "$PWD" frame-cost-saved-confirm "{{executable}}" "{{output}}" "{{cpus}}" "{{pin}}"

# Diagnostic activity trace only; never use these timings as confirmation.
frame-cost-saved-diagnose executable cpus="2,3":
    #!/usr/bin/env bash
    set -euo pipefail
    sudo -n renice -n 0 -p "$$"
    export ZENBENCH_GATE_TRACE=1
    exec taskset -c "{{cpus}}" "{{executable}}" --mode=iterate --cpu-policy=auto --rounds=30 --median-of-passes=3

quiet-command-controls:
    python3 perf/quiet-command-test.py

frame-cost-profile output mode="record" count="1000000" cpus="2,3":
    test ! -e "{{output}}"
    taskset -c "{{cpus}}" perf stat -x, -o "{{output}}" -e instructions,cycles,cache-misses,context-switches "${CARGO_TARGET_DIR:-target}/release/examples/frame_policy_probe" "{{mode}}" "{{count}}"

frame-cost-sample output mode="record" count="5000000" cpus="2,3":
    test ! -e "{{output}}"
    taskset -c "{{cpus}}" perf record --call-graph dwarf -F 997 -o "{{output}}" "${CARGO_TARGET_DIR:-target}/release/examples/frame_policy_probe" "{{mode}}" "{{count}}"

frame-cost-sample-report input:
    perf report --stdio -i "{{input}}" --percent-limit 2 --sort symbol

# Rerun the exact existing fixture set without regenerating containers.
io-validation-media mp4 webm ivf:
    cargo run --locked --manifest-path perf/io-validation/Cargo.toml --bin media_transport -- "{{mp4}}"/*.mp4 "{{webm}}"/*.webm "{{webm}}"/*.mkv "{{ivf}}"/*.ivf

# Private executable proposal: bounded range plans, storage leases, ordered delivery.
io-admission-check:
    cargo test --manifest-path perf/io-validation/Cargo.toml -p io-validation --test admission --locked
    cargo clippy --manifest-path perf/io-validation/Cargo.toml -p io-validation --all-targets --locked -- -D warnings

io-admission-bench cpus="2,3":
    taskset -c "{{cpus}}" cargo bench --manifest-path perf/io-validation/Cargo.toml -p io-validation --bench admission --locked -- --cpu-policy=unrestricted --median-of-passes=3

# Native QOI with an explicit raw-retention control and a gated short writer.
io-encoder-retention-check:
    cargo test --manifest-path perf/io-validation/Cargo.toml -p io-validation --test encoder_retention --locked -- --nocapture
    cargo clippy --manifest-path perf/io-validation/Cargo.toml -p io-validation --all-targets --locked -- -D warnings

# Private resolver experiment: conditional choice followed by shared reservation.
io-policy-resolution-check:
    cargo test --manifest-path perf/io-validation/Cargo.toml -p io-validation --test policy_resolution --locked
    cargo clippy --manifest-path perf/io-validation/Cargo.toml -p io-validation --test policy_resolution --locked -- -D warnings

# One typed resolver, configured presets, traced failures and adversarial choices.
io-policy-engine-check:
    cargo test --manifest-path perf/io-validation/Cargo.toml -p io-validation --test policy_engine --test policy_execution --locked
    cargo clippy --manifest-path perf/io-validation/Cargo.toml -p io-validation --test policy_engine --test policy_execution --locked -- -D warnings

io-policy-native-check:
    cargo test --manifest-path perf/io-validation/Cargo.toml -p io-validation --test policy_native --locked
    cargo clippy --manifest-path perf/io-validation/Cargo.toml -p io-validation --test policy_native --locked -- -D warnings

# Native bounded XML parsing and per-carrier disclosure qualification.
io-policy-disclosure-check:
    cargo test --manifest-path perf/io-validation/Cargo.toml -p io-validation --test policy_disclosure --test policy_engine --locked
    cargo clippy --manifest-path perf/io-validation/Cargo.toml -p io-validation --test policy_disclosure --test policy_engine --locked -- -D warnings

# Native publication ordering plus explicit waiting, finality and epoch controls.
io-policy-stream-check:
    cargo test --manifest-path perf/io-validation/Cargo.toml -p io-validation --test policy_stream --test policy_disclosure --locked
    cargo clippy --manifest-path perf/io-validation/Cargo.toml -p io-validation --test policy_stream --test policy_disclosure --locked -- -D warnings

# Resolve newly enabled harness features outside workspace path overrides.
io-policy-update-lock:
    cargo update --manifest-path perf/io-validation/Cargo.toml -p zenmeta

# Standalone native JPEG qualification; requires djpeg.
jpeg-policy-setup:
    cp perf/jpeg-policy-validation/dependencies.lock perf/jpeg-policy-validation/Cargo.lock

jpeg-policy-update-lock:
    cargo update --manifest-path perf/jpeg-policy-validation/Cargo.toml

jpeg-policy-check artifacts="$HOME/tmp/zencodec-jpeg-oracles":
    just codec-graph-check perf/jpeg-policy-validation/Cargo.toml
    JPEG_POLICY_ARTIFACTS="{{artifacts}}" cargo test --manifest-path perf/jpeg-policy-validation/Cargo.toml --locked -- --nocapture --test-threads=1
    cargo clippy --manifest-path perf/jpeg-policy-validation/Cargo.toml --all-targets --locked -- -D warnings

jpeg-policy-sync export:
    test ! -e "{{export}}/.git"
    test ! -e "{{export}}/.jj"
    mkdir -p "{{export}}/perf/jpeg-policy-validation/src"
    cp perf/jpeg-policy-validation/Cargo.toml perf/jpeg-policy-validation/dependencies.lock "{{export}}/perf/jpeg-policy-validation/"
    cp perf/jpeg-policy-validation/src/*.rs "{{export}}/perf/jpeg-policy-validation/src/"
    mkdir -p "{{export}}/perf/jpeg-policy-validation/src/bin"
    cp perf/jpeg-policy-validation/src/bin/*.rs "{{export}}/perf/jpeg-policy-validation/src/bin/"
    mkdir -p "{{export}}/zencodec-testkit/src"
    cp zencodec-testkit/src/fixtures.rs "{{export}}/zencodec-testkit/src/"
    mkdir -p "{{export}}/docs/examples"
    cp docs/examples/jpeg_policy.rs "{{export}}/docs/examples/"
    cp docs/examples/pixel_conversion.rs docs/examples/checked_operations.rs "{{export}}/docs/examples/"
    cp justfile "{{export}}/justfile"

jpeg-preparation-build:
    cargo build --manifest-path perf/jpeg-policy-validation/Cargo.toml --bin encode_preparation --locked
    cargo clippy --manifest-path perf/jpeg-policy-validation/Cargo.toml --bin encode_preparation --locked -- -D warnings

jpeg-preparation-check:
    cargo test --manifest-path perf/jpeg-policy-validation/Cargo.toml --locked preparation:: -- --test-threads=1
    cargo clippy --manifest-path perf/jpeg-policy-validation/Cargo.toml --all-targets --locked -- -D warnings

# Copies the harness into an isolated export. The preflight scenarios run on
# the pinned zenjpeg's shared hooks; nothing is patched.
jpeg-preflight-setup export: (jpeg-policy-sync export)

jpeg-preflight-check:
    cargo test --manifest-path perf/jpeg-policy-validation/Cargo.toml --features native-preflight --locked native_preflight:: -- --test-threads=1
    cargo clippy --manifest-path perf/jpeg-policy-validation/Cargo.toml --features native-preflight --all-targets --locked -- -D warnings

jpeg-preflight-build:
    cargo build --manifest-path perf/jpeg-policy-validation/Cargo.toml --features native-preflight --bin encode_preparation --locked

jpeg-preflight-profile executable output revision:
    python3 -m unittest discover -s perf/jpeg-policy-validation -p test_preparation_profile.py
    python3 perf/jpeg-policy-validation/profile-preparation.py "{{executable}}" "{{output}}" --source-revision "{{revision}}" --native-hooks

jpeg-preflight-ci directory target:
    python3 -m unittest discover -s perf/jpeg-policy-validation -p test_native_graph.py
    python3 perf/jpeg-policy-validation/check-native-preflight.py "{{directory}}" --target "{{target}}"

jpeg-preparation-profile executable output revision:
    python3 -m unittest discover -s perf/jpeg-policy-validation -p test_preparation_profile.py
    python3 perf/jpeg-policy-validation/profile-preparation.py "{{executable}}" "{{output}}" --source-revision "{{revision}}"

# Paths are required; this integration never silently skips missing fixtures.
io-policy-media-check +fixtures:
    cargo test --manifest-path perf/io-validation/Cargo.toml -p io-validation --bin policy_media --locked
    cargo run --manifest-path perf/io-validation/Cargo.toml -p io-validation --bin policy_media --locked -- {{fixtures}}
    cargo clippy --manifest-path perf/io-validation/Cargo.toml -p io-validation --bin policy_media --locked -- -D warnings

io-policy-cost-build:
    cargo clippy --manifest-path perf/io-validation/Cargo.toml -p io-validation --bench policy --bin policy_probe --locked -- -D warnings
    cargo build --manifest-path perf/io-validation/Cargo.toml -p io-validation --release --bin policy_probe --locked
    cargo bench --manifest-path perf/io-validation/Cargo.toml -p io-validation --bench policy --locked --no-run

io-policy-bench cpus="2,3":
    taskset -c "{{cpus}}" cargo bench --manifest-path perf/io-validation/Cargo.toml -p io-validation --bench policy --locked -- --cpu-policy=unrestricted --median-of-passes=3

io-policy-allocations output:
    python3 perf/io-validation/profile-policy.py "${CARGO_TARGET_DIR:-target}/release/policy_probe" "{{output}}"

io-policy-reanalyze output:
    python3 perf/io-validation/profile-policy.py "${CARGO_TARGET_DIR:-target}/release/policy_probe" "{{output}}" --reanalyze

# Isolated no_std check; invoke in the prepared export to exclude path overrides.
io-policy-portability target="wasm32-unknown-unknown":
    cp perf/policy-check/dependencies.lock perf/policy-check/Cargo.lock
    cargo check --manifest-path perf/policy-check/Cargo.toml --locked --target "{{target}}"

# Isolate policy code using the real fidelity source, whereat and enough.
io-policy-core-portability target="thumbv7em-none-eabihf":
    cp perf/policy-check/dependencies.lock perf/policy-check/Cargo.lock
    cargo check --manifest-path perf/policy-check/Cargo.toml --locked --no-default-features --target "{{target}}"

io-policy-portability-check:
    just io-policy-portability wasm32-unknown-unknown
    just io-policy-core-portability thumbv7em-none-eabihf
    cargo test --manifest-path perf/policy-check/Cargo.toml --locked --no-default-features --lib
    cargo clippy --manifest-path perf/policy-check/Cargo.toml --locked --no-default-features --lib -- -D warnings

# Copy only this experiment's text into an existing disposable source export.
io-policy-sync export:
    test ! -e "{{export}}/.git"
    test ! -e "{{export}}/.jj"
    test -f "{{export}}/perf/io-validation/Cargo.toml"
    mkdir -p "{{export}}/perf/io-validation/src/policy" "{{export}}/perf/policy-check/src"
    cp perf/io-validation/src/policy/*.rs "{{export}}/perf/io-validation/src/policy/"
    cp perf/io-validation/src/policy_execution.rs "{{export}}/perf/io-validation/src/"
    cp perf/io-validation/src/policy_native.rs "{{export}}/perf/io-validation/src/"
    cp perf/io-validation/src/policy_disclosure.rs "{{export}}/perf/io-validation/src/"
    cp perf/io-validation/src/policy_stream.rs "{{export}}/perf/io-validation/src/"
    cp perf/io-validation/src/policy_media.rs "{{export}}/perf/io-validation/src/"
    cp perf/io-validation/src/policy_benchmark.rs "{{export}}/perf/io-validation/src/"
    cp perf/io-validation/src/bin/policy_media.rs "{{export}}/perf/io-validation/src/bin/"
    cp perf/io-validation/src/bin/policy_probe.rs "{{export}}/perf/io-validation/src/bin/"
    cp perf/io-validation/benches/policy.rs "{{export}}/perf/io-validation/benches/"
    cp perf/io-validation/profile-policy.py "{{export}}/perf/io-validation/"
    cp perf/io-validation/Cargo.toml "{{export}}/perf/io-validation/"
    cp perf/io-validation/tests/policy*.rs "{{export}}/perf/io-validation/tests/"
    cp docs/examples/policy.rs "{{export}}/docs/examples/"
    cp perf/policy-check/Cargo.toml "{{export}}/perf/policy-check/"
    cp perf/policy-check/dependencies.lock "{{export}}/perf/policy-check/"
    cp perf/policy-check/src/lib.rs "{{export}}/perf/policy-check/src/"
    cp justfile "{{export}}/justfile"

# Reuse the IMI transport model to compare read API policies.
io-transport-model:
    cargo test --manifest-path perf/io-validation/Cargo.toml -p io-validation --bin transport_contracts --locked
    cargo run --manifest-path perf/io-validation/Cargo.toml -p io-validation --bin transport_contracts --locked

# Documentation source stays under docs/ on main; only its artifact is deployed.
code-catalog export:
    python3 docs/generate-code-catalog.py "{{export}}" --source-root "{{justfile_directory()}}" --output docs/code-api

# Exercise actual consumer calls for source-supplemented dyn-trait methods.
source-details-check:
    cargo test -p zencodec --test source_encoding_details --locked
    cargo clippy -p zencodec --test source_encoding_details --locked -- -D warnings

# Arithmetic controls only; these do not qualify native codec fidelity.
precision-grid-check:
    python3 perf/precision-validation/grid_oracles.py

# Run in a source export through run-heavy; review unrelated resolution changes.
precision-cohort-resolve export results:
    python3 perf/precision-validation/cohort.py "{{export}}" "{{results}}"

precision-evidence-check:
    cargo test -p zencodec --test encode_precision --locked
    cargo test -p zencodec --lib evidence_survives_plain_send_and_send_sync_erased_jobs_and_encoders --locked
    cargo test -p zencodec-io --features std --test planned_row_encode --locked

# Isolated native experiment; never adds its feature to the production graph.
publication-witness-prepare checkout export:
    python3 perf/publication-validation/prepare.py "{{checkout}}" "{{export}}"

publication-witness-check export results:
    python3 perf/publication-validation/check.py "{{export}}" "{{results}}"

docs-site-check:
    TMPDIR="$HOME/tmp" python3 docs/test_code_catalog.py
    node docs/build-site.mjs
    node --test docs/site/site.test.mjs docs/site/source.test.mjs docs/site/code-first.test.mjs docs/site/source-catalog.test.mjs

docs-site-serve:
    python3 -m http.server 3537 --bind 0.0.0.0 --directory docs/_site

docs-site-browser:
    node docs/site/browser.test.mjs

# Parse the documentation workflow and check every embedded shell script.
docs-ci-shell-check:
    #!/usr/bin/env python3
    from pathlib import Path
    import subprocess, yaml
    workflow = yaml.safe_load(Path('.github/workflows/docs.yml').read_text())
    for job in workflow['jobs'].values():
        for step in job.get('steps', []):
            if 'run' in step:
                subprocess.run(['bash', '-n'], input=step['run'], text=True, check=True)

# Start and stop an isolated server for the browser checks.
docs-site-browser-check port="3547": docs-site-check
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p "$HOME/tmp"
    python3 -m http.server {{port}} --bind 127.0.0.1 --directory docs/_site > "$HOME/tmp/zencodec-docs-{{port}}.log" 2>&1 &
    docs_pid=$!
    trap 'kill "$docs_pid"' EXIT
    for attempt in {1..30}; do
        if curl --fail --silent "http://127.0.0.1:{{port}}/" > /dev/null; then break; fi
        sleep 0.2
    done
    SITE_URL="http://127.0.0.1:{{port}}/" just docs-site-browser

# Explicit network check of the frozen crates.io baseline; site builds stay offline.
docs-migration-baseline:
    python3 docs/verify-migration-baseline.py

# Run in the prepared standalone harness (see perf/io-validation/README.md).
docs-site-examples:
    cargo test --manifest-path perf/io-validation/Cargo.toml -p io-validation --test site_examples --test encoder_retention --test policy_resolution --test policy_engine --test policy_execution --test policy_native --test policy_disclosure --test policy_stream --bin policy_media --bin transport_contracts --locked
    cargo clippy --manifest-path perf/io-validation/Cargo.toml -p io-validation --test site_examples --test encoder_retention --test policy_resolution --test policy_engine --test policy_execution --test policy_native --test policy_disclosure --test policy_stream --bin policy_media --bin transport_contracts --locked -- -D warnings

# Sequential documentation-learning study: preserve submissions and native results.
guide-learning-capture url output revision:
    node docs/evaluation/capture.mjs "{{url}}" "{{output}}" "{{revision}}"

guide-learning-grade task submission export output:
    python3 docs/evaluation/run.py "{{task}}" "{{submission}}" "{{export}}" "{{output}}"

guide-learning-grade-session reader export output:
    python3 docs/evaluation/grade_session.py "{{reader}}" "{{export}}" "{{output}}"

guide-learning-audit task submission export output:
    python3 docs/evaluation/run.py "{{task}}" "{{submission}}" "{{export}}" "{{output}}" --audit

guide-learning-deliver task stage packets reader:
    python3 docs/evaluation/deliver.py "{{task}}" "{{stage}}" "{{packets}}" "{{reader}}"

guide-visuals url output:
    node docs/evaluation/visuals.mjs "{{url}}" "{{output}}"

guide-learning-summary artifacts output +cohorts:
    python3 docs/evaluation/summarize.py "{{artifacts}}" "{{output}}" {{cohorts}}

# Controlled protocol, policy adapter and coroutine implementation share one export.
control-sync export:
    just operation-sync "{{export}}"
    just io-core-sync "{{export}}"
    cp src/traits/control.rs src/traits/decoding.rs src/traits/dyn_decoding.rs src/traits/incremental.rs src/traits/mod.rs "{{export}}/src/traits/"
    cp zencodec-coroutine/src/*.rs "{{export}}/zencodec-coroutine/src/"
    cp zencodec-coroutine/tests/control.rs zencodec-coroutine/tests/operation.rs "{{export}}/zencodec-coroutine/tests/"

control-check:
    cargo test -p zencodec-coroutine --tests --locked
    cargo clippy -p zencodec-coroutine -p zencodec-io -p zencodec --all-targets --all-features --locked -- -D warnings

# Shared policy route constraints and compatibility of the no_std core.
transcode-policy-check:
    just operation-policy-check
    cargo clippy -p zencodec --all-targets --locked -- -D warnings
    cargo check -p zencodec --no-default-features --locked

operation-send-check:
    cargo test -p zencodec-testkit --test operation_requests --test operation_policy --test operation_defaults --locked
    cargo clippy -p zencodec --all-targets --locked -- -D warnings
    cargo check -p zencodec --no-default-features --locked

# Preserve checked preparation across erasure and review the additive surface.
operation-send-integration:
    just operation-send-check
    cargo test --workspace --locked
    cargo clippy -p zencodec-testkit --test operation_policy --locked -- -D warnings
    just api-doc

# Execute the public checked-operation guide through the pinned native adapters.
docs-checked-examples:
    JPEG_POLICY_ARTIFACTS="$HOME/tmp/zencodec-jpeg-oracles" cargo test --manifest-path perf/jpeg-policy-validation/Cargo.toml --locked checked_examples -- --nocapture
    cargo clippy --manifest-path perf/jpeg-policy-validation/Cargo.toml --all-targets --locked -- -D warnings

# Resolve the pinned checked guide graph before all native JPEG/CMS scenarios.
checked-guide-integration:
    just jpeg-policy-update-lock
    just jpeg-policy-check

# Pinned source signatures with caller-facing import paths and parameter names.
checked-guide-api checkout revision:
    node docs/capture-integration-api.mjs "{{checkout}}" "{{revision}}"

# Capture a lane-owned git-archive export with adjacent revision/hash receipts.
checked-guide-api-export export revision:
    node docs/capture-integration-api.mjs "{{export}}" "{{revision}}" --export

# Exact conversion signatures from the same pin as the native TIFF/JXL harness.
checked-output-api checkout revision:
    node docs/capture-output-api.mjs "{{checkout}}" "{{revision}}"

# Sink admission is independent of the native decoder's batch size.
sink-capacity-sync export:
    just control-sync "{{export}}"
    cp src/sink.rs "{{export}}/src/"
    cp src/helpers/mod.rs "{{export}}/src/helpers/"
    cp tests/sink_capacity.rs "{{export}}/tests/"

sink-capacity-check:
    cargo test -p zencodec --test sink_capacity --locked
    cargo test -p zencodec-coroutine --tests --locked
    cargo test -p zencodec-testkit --locked
    cargo clippy -p zencodec -p zencodec-coroutine --all-targets --all-features --locked -- -D warnings
    cargo check -p zencodec-coroutine --no-default-features --locked
    just api-doc

# Independent exact-arithmetic oracle; the child probe has a 30-second deadline.
resolution-oracle-check:
    cargo build -p zencodec --example resolution_probe --locked
    python3 scripts/check-resolution-oracle.py "${CARGO_TARGET_DIR:-target}/debug/examples/resolution_probe"
    cargo clippy -p zencodec --example resolution_probe --locked -- -D warnings

resolution-oracle-sync export:
    just operation-sync "{{export}}"
    cp examples/resolution_probe.rs "{{export}}/examples/"
    cp scripts/check-resolution-oracle.py "{{export}}/scripts/"

# Preflight every finish patch before publication or in-memory mutation.
mux-finish-sync export:
    cp zencodec-media/src/container.rs "{{export}}/zencodec-media/src/"
    cp zencodec-media-testkit/src/checks.rs "{{export}}/zencodec-media-testkit/src/"
    cp zencodec-media-testkit/tests/contracts.rs zencodec-media-testkit/tests/checks.rs "{{export}}/zencodec-media-testkit/tests/"
    cp justfile "{{export}}/justfile"

mux-finish-check:
    cargo test -p zencodec-media-testkit --locked
    cargo clippy -p zencodec-media -p zencodec-media-testkit --all-targets --locked -- -D warnings
    cargo check -p zencodec-media --no-default-features --target wasm32-unknown-unknown --locked

# Real AV1 packets through native container output on network/disk destinations.
av1-mux-check artifacts:
    AV1_POLICY_ARTIFACTS="{{artifacts}}" cargo test --manifest-path perf/av1-policy-validation/Cargo.toml --locked mux_native_ -- --nocapture --test-threads=1
    cargo clippy --manifest-path perf/av1-policy-validation/Cargo.toml --all-targets --locked -- -D warnings

av1-timing-oracle-build directory version="8.0.1":
    python3 perf/av1-policy-validation/build_timing_oracle.py "{{directory}}" --version "{{version}}"

# Deferred conversion admission and native route selection, with real AV1 pixels.
conversion-check artifacts:
    cargo test -p zencodec-media-testkit --locked --test storage_ownership --test image_output
    just av1-planar-check "{{artifacts}}"

# Independent color inspection reports disagreement; it sets no acceptance tolerance.
av1-planar-rgb-inspect source output:
    python3 perf/av1-policy-validation/inspect_planar_rgb.py "{{source}}" "{{output}}"

# Inspect required unclipped float artifacts independently alongside integer output.
av1-planar-float-inspect source output:
    python3 perf/av1-policy-validation/inspect_planar_rgb.py "{{source}}" "{{output}}" --float-output

# Fixed numerical gate; retain both float and integer observations and negative controls.
av1-planar-float-verify source output:
    python3 perf/av1-policy-validation/inspect_planar_rgb.py "{{source}}" "{{output}}" --float-output --verify-float

# Resolve both pixel crates together in standalone native validation exports.
native-pixels-resolve:
    #!/usr/bin/env bash
    set -euo pipefail
    test ! -e .git
    test ! -e .jj
    for package in jpeg-policy-validation av1-policy-validation spatial-validation; do
        cp "perf/$package/dependencies.lock" "perf/$package/Cargo.lock"
        cargo update --manifest-path "perf/$package/Cargo.toml" -p zenpixels -p zenpixels-convert
    done

# Full real-codec coverage for changes shared by every native pixel consumer.
native-pixels-check artifacts:
    just jpeg-policy-check "{{artifacts}}/jpeg"
    just av1-policy-check "{{artifacts}}/av1"
    just spatial-ci-check "{{artifacts}}/spatial" cms_oracle,old_jpeg

# Inspect saved executing native traces without importing Rust conversion code.
av1-gamma-verify artifacts report:
    python3 -m unittest discover -s perf/av1-policy-validation -p verify_gamma_test.py
    python3 perf/av1-policy-validation/verify_gamma.py "{{artifacts}}" "{{report}}"

# Coupled conversion and fixed native encoder; shared policy and required oracle.
av1-coupled-check artifacts:
    AV1_POLICY_ARTIFACTS="{{artifacts}}" cargo test --manifest-path perf/av1-policy-validation/Cargo.toml --locked coupled_native_ -- --nocapture --test-threads=1
    cargo clippy --manifest-path perf/av1-policy-validation/Cargo.toml --all-targets --locked -- -D warnings

# Build only the unchanged fixed-work policy probe for retained counter captures.
frame-cost-probe-build target:
    test ! -e "{{target}}"
    CARGO_TARGET_DIR="{{target}}" CARGO_PROFILE_RELEASE_DEBUG=1 CARGO_PROFILE_RELEASE_LTO=thin cargo build --release --example frame_policy_probe --locked

frame-cost-saved-heap executable output count="1000000":
    test ! -e "{{output}}.zst"
    heaptrack --record-only -o "{{output}}" "{{executable}}" combined "{{count}}"
    heaptrack_print -f "{{output}}.zst" > "{{output}}.txt"

frame-cost-saved-sample executable output count="10000000" cpus="2,3":
    test ! -e "{{output}}"
    taskset -c "{{cpus}}" perf record --call-graph dwarf -F 997 -o "{{output}}" "{{executable}}" combined "{{count}}"
    perf report --stdio -i "{{output}}" --percent-limit 2 --sort symbol > "{{output}}.txt"

frame-cost-pair-plan root before after:
    python3 perf/frame-policy-pair.py prepare "{{root}}" "{{before}}" "{{after}}"

frame-cost-pair-run zenbench root:
    "{{zenbench}}" pair "{{root}}/pair-plan.json" --dry-run > "{{root}}/pair-dry-run.json"
    "{{zenbench}}" pair "{{root}}/pair-plan.json" --mode iterate --output "{{root}}/pair-attempt"

# Native retained encoder → exact MP4 destination, without packet-time rewriting.
av1-coupled-mux-check artifacts:
    AV1_POLICY_ARTIFACTS="{{artifacts}}" cargo test --manifest-path perf/av1-policy-validation/Cargo.toml --locked mux_native_coupled_ -- --nocapture --test-threads=1
    cargo clippy --manifest-path perf/av1-policy-validation/Cargo.toml --all-targets --locked -- -D warnings

# Standalone renderer uses the review converter; core codec graphs stay unchanged.
render-check:
    cp zencodec-render/dependencies.lock zencodec-render/Cargo.lock
    cargo check --manifest-path zencodec-render/Cargo.toml --no-default-features --locked
    cargo clippy --manifest-path zencodec-render/Cargo.toml --all-targets --locked -- -D warnings
    cargo test --manifest-path zencodec-render/Cargo.toml --locked

render-msrv-check:
    cp zencodec-render/dependencies.lock zencodec-render/Cargo.lock
    cargo +1.89.0 check --manifest-path zencodec-render/Cargo.toml --no-default-features --locked

# Owned finite encoder/destination path, with real AV1 and an independent reader.
staged-encode-check artifacts:
    cargo test -p zencodec-media --features std --locked
    cargo clippy -p zencodec-media --all-targets --features std --locked -- -D warnings
    AV1_POLICY_ARTIFACTS="{{artifacts}}" cargo test --manifest-path perf/av1-policy-validation/Cargo.toml --locked staged_native_ -- --nocapture --test-threads=1
    cargo clippy --manifest-path perf/av1-policy-validation/Cargo.toml --all-targets --locked -- -D warnings

# Real JPEG scheduling, including tiny/nonstreaming configurations and short writes.
jpeg-output-scheduling-check:
    cargo test --manifest-path perf/jpeg-policy-validation/Cargo.toml --locked output_scheduling -- --nocapture
    cargo clippy --manifest-path perf/jpeg-policy-validation/Cargo.toml --all-targets --locked -- -D warnings

output-schedule-check:
    cargo test -p zencodec --all-features --locked operation::scheduling
    cargo clippy -p zencodec --all-targets --all-features --locked -- -D warnings

row-encode-sync export:
    just jpeg-policy-sync "{{export}}"
    cp zencodec-io/src/destination.rs zencodec-io/src/row_encode.rs zencodec-io/src/lib.rs zencodec-io/src/encode_write.rs "{{export}}/zencodec-io/src/"
    cp zencodec-io/tests/encode_write.rs "{{export}}/zencodec-io/tests/"
    cp zencodec-io/examples/io_contract_probes.rs "{{export}}/zencodec-io/examples/"

row-encode-check artifacts:
    mkdir -p "{{artifacts}}"
    JPEG_POLICY_ARTIFACTS="{{artifacts}}" cargo test --manifest-path perf/jpeg-policy-validation/Cargo.toml --locked row_driver -- --nocapture
    cargo clippy --manifest-path perf/jpeg-policy-validation/Cargo.toml --all-targets --locked -- -D warnings
    cargo clippy -p zencodec-io --all-targets --features std --locked -- -D warnings

compile-adapters-report root log receipt:
    python3 perf/compile-adapters-report.py "{{root}}" "{{log}}" "{{receipt}}"

# One patch representation and actual destination wrappers for raster/media output.
output-patch-sync export:
    just av1-policy-sync "{{export}}"
    cp src/output_patch.rs "{{export}}/src/"
    cp zencodec-io/src/row_encode.rs zencodec-io/src/lib.rs zencodec-io/src/destination.rs "{{export}}/zencodec-io/src/"

output-patch-check artifacts:
    cargo test --workspace --all-features --locked
    cargo clippy -p zencodec -p zencodec-io -p zencodec-media --all-targets --all-features --locked -- -D warnings
    AV1_POLICY_ARTIFACTS="{{artifacts}}" cargo test --manifest-path perf/av1-policy-validation/Cargo.toml --locked mux_native_ -- --nocapture --test-threads=1

raster-output-native-check artifacts:
    AV1_POLICY_ARTIFACTS="{{artifacts}}" cargo test --manifest-path perf/av1-policy-validation/Cargo.toml --locked mux_native_raster_ -- --nocapture
    cargo clippy --manifest-path perf/av1-policy-validation/Cargo.toml --all-targets --locked -- -D warnings

# Final interpretation cannot discharge unrelated required policy obligations.
boundary-preset-native-check artifacts:
    JPEG_POLICY_ARTIFACTS="{{artifacts}}" cargo test --manifest-path perf/jpeg-policy-validation/Cargo.toml --locked native_final_boundary_cannot_bypass_unproven_preset_requirements -- --nocapture

# Replay contract candidate; the native harness is separate and explicitly opt-in.
multipass-sync export:
    test ! -e "{{export}}/.git"
    test ! -e "{{export}}/.jj"
    mkdir -p "{{export}}/perf/multipass-validation/src" "{{export}}/perf/multipass-validation/tests" "{{export}}/perf/multipass-validation/native/src" "{{export}}/perf/multipass-validation/native/tests"
    cp perf/multipass-validation/Cargo.toml perf/multipass-validation/dependencies.lock "{{export}}/perf/multipass-validation/"
    cp perf/multipass-validation/src/*.rs "{{export}}/perf/multipass-validation/src/"
    cp perf/multipass-validation/tests/*.rs "{{export}}/perf/multipass-validation/tests/"
    cp perf/multipass-validation/native/Cargo.toml perf/multipass-validation/native/dependencies.lock "{{export}}/perf/multipass-validation/native/"
    cp perf/multipass-validation/native/src/*.rs "{{export}}/perf/multipass-validation/native/src/"
    cp perf/multipass-validation/native/tests/*.rs "{{export}}/perf/multipass-validation/native/tests/"

# Shared typed/dyn pull defaults, loans and original-error forwarding.
encoder-pull-check:
    cargo test -p zencodec --all-features --locked --lib
    cargo clippy -p zencodec --all-targets --all-features --locked -- -D warnings
    cargo test -p zencodec --all-features --locked --doc

# Joint prepared input/replay/output route admission and existing policy domains.
encoding-route-check:
    cargo test --workspace --all-features --locked
    cargo test -p zencodec --all-features --locked --doc
    cargo clippy --workspace --all-targets --all-features --locked -- -D warnings

# Refuse unknown preparation workspace before native planning, typed and erased.
row-preparation-check:
    cargo test -p zencodec-io --features std --locked --test planned_row_encode
    cargo test -p zencodec-io --features std --locked --lib
    cargo clippy -p zencodec-io --features std --all-targets --locked -- -D warnings

# Final writes may cancel: no later cursor/flush callbacks are permitted.
publication-cancellation-check:
    cargo test -p zencodec-io --features std --locked --lib row_encode::patch_tests
    cargo clippy -p zencodec-io --features std --all-targets --locked -- -D warnings
    just multipass-check

multipass-check:
    cp perf/multipass-validation/dependencies.lock perf/multipass-validation/Cargo.lock
    cargo test --manifest-path perf/multipass-validation/Cargo.toml --locked
    cargo clippy --manifest-path perf/multipass-validation/Cargo.toml --all-targets --locked -- -D warnings

multipass-native-check artifacts:
    mkdir -p "{{artifacts}}"
    cp perf/multipass-validation/native/dependencies.lock perf/multipass-validation/native/Cargo.lock
    CARGO_NET_GIT_FETCH_WITH_CLI=true MULTIPASS_SCRATCH="{{artifacts}}" cargo test --manifest-path perf/multipass-validation/native/Cargo.toml --locked -- --test-threads=1
    CARGO_NET_GIT_FETCH_WITH_CLI=true cargo clippy --manifest-path perf/multipass-validation/native/Cargo.toml --all-targets --locked -- -D warnings

# Mandatory route admission plus native JPEG effect-boundary regression.
policy-admission-check artifacts:
    cargo test --workspace --all-features --locked
    cargo clippy -p zencodec --all-targets --all-features --locked -- -D warnings
    just jpeg-policy-check "{{artifacts}}"

# Complete control-interface migration gate; run from an isolated source export.
pulse-check:
    cargo check --workspace --all-features --all-targets --locked --keep-going

pulse-test:
    cargo test --workspace --all-features --locked
    cargo clippy --workspace --all-features --all-targets --locked -- -D warnings

# Complete declaration preflight, dyn forwarding and startup drift; isolated export.
encode-plan-check:
    cargo test -p zencodec --lib --locked
    cargo clippy -p zencodec --all-targets --locked -- -D warnings

# Capture a pinned converter surface; CHECKOUT may be --github for remote source.
capture-ycbcr-api checkout revision:
    node docs/capture-ycbcr-api.mjs "{{checkout}}" "{{revision}}"

# Public-source checkout smoke test; no codec checkout or local path patches.
guide-starter-check checkout:
    cd "{{checkout}}" && cargo test -p zencodec-testkit --lib --test usage

# Policy admission remains valid through completion and terminal cleanup.
policy-completion-check:
    cargo test --locked -p zencodec-media-testkit --features zencodec-media/std --test policy_completion_review
    cargo test --locked -p zencodec-testkit --test policy_final_facts_review

# Metadata and output schedule must belong to the same executable offer.
transcode-composition-check:
    cargo test --locked -p zencodec operation::transcode::composition_tests
    cargo clippy --locked -p zencodec --all-features --all-targets -- -D warnings

# Source-only guard for portable codec family graphs; no Cargo invocation.
optional-native-audit-check:
    python3 perf/optional-native/test-audit.py

# Build-product cleanup preserves timing evidence and waits for verified exit.
compile-cleanup-check:
    python3 perf/test_compile_cleanup.py
    python3 -m py_compile perf/compile-time.py perf/compile-adapters-report.py

# Refuse unknown required workspace before any wrapped native video operation.
media-workspace-admission-check:
    cargo test --locked -p zencodec-media-testkit --features zencodec-media/std
    cargo clippy --locked -p zencodec-media -p zencodec-media-testkit --features zencodec-media/std --all-targets -- -D warnings

media-native-workspace-check:
    cargo test --locked --manifest-path perf/av1-policy-validation/Cargo.toml native_factory_workspace_admission_precedes_rav1d_construction
    cargo test --locked --manifest-path perf/av1-policy-validation/Cargo.toml native_error_keeps_rav1d_whereat_trace_through_policy_and_erasure

media-staged-admission-check artifacts:
    just media-workspace-admission-check
    cargo test --locked --manifest-path perf/av1-policy-validation/Cargo.toml staged_native_workspace_refusal
    AV1_POLICY_ARTIFACTS="{{artifacts}}" cargo test --locked --manifest-path perf/av1-policy-validation/Cargo.toml staged_native_owned_pipeline_drops_encoder_before_writer_and_preserves_pixels
    cargo test --locked --manifest-path perf/av1-policy-validation/Cargo.toml staged_native_refusal_conversion_drift_and_unwind_never_publish_or_resume

# Native negotiation counterexamples for required one-shot output admission.
jpeg-decode-admission-check:
    cargo test --manifest-path perf/jpeg-policy-validation/Cargo.toml --locked decode_admission:: -- --test-threads=1
    cargo clippy --manifest-path perf/jpeg-policy-validation/Cargo.toml --locked --lib --tests -- -D warnings

# Check pinned oracle download integrity and bounded transport fallback.
av1-timing-source-check:
    python3 -m unittest discover -s perf/av1-policy-validation -p test_timing_source.py

registry-sharing-check:
    cargo test -p zencodec-testkit --locked --test registry_cloning --test codec_set --test operation_requests
    cargo clippy -p zencodec -p zencodec-testkit --locked --all-targets -- -D warnings
    cargo test --manifest-path perf/jpeg-policy-validation/Cargo.toml --locked --lib control:: -- --test-threads=1

# Retained raster pointer identity plus existing admission and native parity.
row-borrow-check artifacts:
    cargo test -p zencodec-io --all-features --locked --test planned_row_encode
    cargo clippy -p zencodec-io --all-features --all-targets --locked -- -D warnings
    JPEG_POLICY_ARTIFACTS="{{artifacts}}" cargo test --manifest-path perf/jpeg-policy-validation/Cargo.toml --locked --lib row_driver -- --test-threads=1
    cargo test --manifest-path perf/jpeg-policy-validation/Cargo.toml --locked --lib precision:: -- --test-threads=1

# Compile every standalone graph participating in the paired pixel rollout.
# Run io-validation-setup first; copy each dependencies.lock to Cargo.lock.
paired-harness-check:
    #!/usr/bin/env bash
    set -euo pipefail
    for manifest in fuzz/Cargo.toml zencodec-render/Cargo.toml perf/api-usability-validation/Cargo.toml perf/av1-policy-validation/Cargo.toml perf/io-validation/Cargo.toml perf/jpeg-policy-validation/Cargo.toml perf/multipass-validation/Cargo.toml perf/multipass-validation/native/Cargo.toml perf/native-contract-validation/Cargo.toml perf/native-contract-validation/spatial-native/Cargo.toml perf/native-contract-validation/video-native/Cargo.toml perf/policy-check/Cargo.toml perf/spatial-validation/Cargo.toml; do
        cargo check --manifest-path "$manifest" --all-targets --locked
    done

# Decoder-controlled collector dimensions must refuse without panicking.
collect-allocation-check:
    cargo test -p zencodec-io --lib --locked
    cargo test -p zencodec-io --features std --test drivers --locked
    cargo clippy -p zencodec-io --lib --tests --all-features --locked -- -D warnings
    cargo check -p zencodec-io --no-default-features --locked

# Ownership bridge only; not a streaming-completeness or admission claim.
decode-details-check:
    cargo test -p zencodec --lib output::tests --locked
    cargo clippy -p zencodec --lib --all-features --locked -- -D warnings
    cargo check -p zencodec --no-default-features --locked
    just jpeg-decode-admission-check

# Optional terminal details: native ownership transport and protocol guards.
terminal-details-check:
    cargo test -p zencodec-coroutine --locked
    cargo clippy -p zencodec-coroutine --all-targets --locked -- -D warnings
    cargo test -p zencodec --lib --locked
    cargo test -p zencodec-io --features tokio --locked
    cargo clippy -p zencodec -p zencodec-io --lib --all-features --locked -- -D warnings
    cargo check -p zencodec --no-default-features --locked
    just jpeg-decode-admission-check

# Profile matched native descriptor refusal and raster ownership with heaptrack.
decode-admission-profile output revision:
    cargo build --manifest-path perf/jpeg-policy-validation/Cargo.toml --locked --bin decode_admission
    python3 perf/jpeg-policy-validation/profile-decode-admission.py "${CARGO_TARGET_DIR:-perf/jpeg-policy-validation/target}/debug/decode_admission" "{{output}}" --source-revision "{{revision}}"

# Checked orientation requirements must refuse before unsupported native startup.
encode-orientation-check:
    cargo test --locked -p zencodec --lib unsupported_orientation_refuses
    cargo check --locked -p zencodec --example orientation
    cargo clippy --locked -p zencodec --lib --example orientation -- -D warnings

# Default-feature release surface, including media policy integration tests.
release-workspace-check:
    cargo test --workspace --all-targets --locked
    cargo test --workspace --doc --locked
    cargo clippy --workspace --all-targets --locked -- -D warnings

# Collector completeness and terminal-state controls, plus all I/O adapters.
collector-coverage-check:
    cargo test --locked -p zencodec-io --all-features
    cargo clippy --locked -p zencodec-io --all-targets --all-features -- -D warnings

# Retain typed sink errors even when an adopted native decoder ignores them.
controlled-sink-errors-check:
    cargo test --locked -p zencodec --lib adopted_decoder_cannot_swallow_destination_failures
    cargo test --locked -p zencodec-io --all-features
    cargo clippy --locked -p zencodec -p zencodec-io --all-features --all-targets -- -D warnings
    just jpeg-decode-admission-check

# Required workspace must refuse before native metadata discovery.
controlled-workspace-check:
    cargo test --manifest-path perf/jpeg-policy-validation/Cargo.toml --locked control:: -- --nocapture
    cargo test --manifest-path perf/jpeg-policy-validation/Cargo.toml --locked decode_admission:: -- --nocapture
    cargo clippy --manifest-path perf/jpeg-policy-validation/Cargo.toml --all-targets --locked -- -D warnings
    cargo test -p zencodec-io --all-features --locked
    cargo clippy -p zencodec-io --all-targets --all-features --locked -- -D warnings

# Compile-report evidence survives removal of completed Cargo targets.
compile-runner-check:
    python3 -m unittest discover -s perf -p test_compile_cleanup.py

# Image/video factories share optional controls and pre/post-construction checks.
controlled-factory-pulse-check: controlled-workspace-check
    cargo test -p zencodec-media-testkit --test frame_policy --locked

# Review-only experiment; does not change application behavior or old assertions.
late-metadata-prepare repository revision output:
    python3 perf/late-metadata-validation/prepare.py "{{repository}}" "{{revision}}" "{{output}}"

# Encoded staging must not retain oversized upstream packet backing owners.
media-staging-owner-check:
    cargo test -p zencodec-media --all-features --locked --lib staging::tests
    cargo clippy -p zencodec-media --all-features --all-targets --locked -- -D warnings

# Registry snapshot semantics and allocation counts (run before/after in exports).
registry-snapshot-check:
    cargo test -p zencodec-testkit --test registry_cloning --test codec_set --locked
    cargo clippy -p zencodec --all-features --all-targets --locked -- -D warnings

registry-allocation-probe:
    cargo run --manifest-path perf/api-usability-validation/Cargo.toml --locked -- --registry-only

# Executable source scopes displayed in the code-first Tokio journeys.
tokio-guide-check:
    cargo test -p zencodec-io --features tokio --test drivers --test work_quota --locked
    cargo clippy -p zencodec-io --features tokio --test drivers --test work_quota --locked -- -D warnings

media-publication-check:
    cargo test -p zencodec-media -p zencodec-media-testkit --all-features --locked
    cargo clippy -p zencodec-media -p zencodec-media-testkit --all-features --all-targets --locked -- -D warnings

# Tokio abort -> operation-local Pulse -> real native JPEG checkpoints.
native-request-cancellation-check artifacts="$HOME/tmp/zencodec-native-request":
    JPEG_POLICY_ARTIFACTS="{{artifacts}}" cargo test --manifest-path perf/jpeg-policy-validation/Cargo.toml --locked request_cancellation:: -- --nocapture
    cargo clippy --manifest-path perf/jpeg-policy-validation/Cargo.toml --all-targets --locked -- -D warnings

# Numeric metadata contracts on both portable and explicit-std paths.
gainmap-portability-check:
    just collection-api-bare-metal
    cargo test -p zencodec --lib gainmap:: --no-default-features --locked
    cargo test -p zencodec --lib gainmap:: --features std --locked

# Native source failures retain causes across readers, erasure and drivers.
io-source-errors-check:
    cargo test -p zencodec-io --features std --test std_source_errors --test read_at_reader --locked
    cargo test -p zencodec-io --features std --lib native_file_errors_retain_kind_and_os_code --locked
    cargo clippy -p zencodec-io --features std --test std_source_errors --test read_at_reader --locked -- -D warnings

# Driver-owned cancellation even when a codec ignores the borrowed Pulse.
async-batch-cancellation-check:
    cargo test -p zencodec-io --features tokio --test drivers --test work_quota --locked
    cargo clippy -p zencodec-io --all-features --all-targets --locked -- -D warnings

# Fail-before-effects workspace admission, plus existing renderer and core controls.
render-workspace-check: render-check
    cargo test -p zencodec --lib --locked
    cargo test -p zencodec --test collection_workspace --locked
    cargo clippy -p zencodec --lib --test collection_workspace --locked -- -D warnings

# Independent sample-membership oracle; native composition remains a separate gate.
placement-oracle-check:
    cargo test -p zencodec --test placement_oracle --locked
    cargo clippy -p zencodec --test placement_oracle --locked -- -D warnings

# Pre-cancellation effect ordering and structured Pulse diagnostic controls.
testkit-pulse-check:
    cargo test --locked -p zencodec-testkit --test sansio_checks
    cargo test --locked -p zencodec-testkit --lib
    cargo clippy --locked -p zencodec-testkit --lib --test sansio_checks -- -D warnings

# Actual JPEG/PNG checkpoints plus shared incremental conformance.
native-pulse-diagnostics-check artifacts="$HOME/tmp/zencodec-native-pulse":
    JPEG_POLICY_ARTIFACTS="{{artifacts}}" cargo test --locked --manifest-path perf/jpeg-policy-validation/Cargo.toml pulse_diagnostics:: -- --nocapture
    cargo clippy --locked --manifest-path perf/jpeg-policy-validation/Cargo.toml --tests -- -D warnings

# Source-only application to a hash-verified PR183 prepared export; no checkout edits.
retained-metadata-reuse-apply export:
    python3 perf/retained-metadata-reuse/apply.py "{{export}}"

# Equal-length plain/required mode names; executable supplied from the frozen graph.
decode-required-output-profile executable output revision profile="release":
    python3 perf/jpeg-policy-validation/profile-decode-admission.py "{{executable}}" "{{output}}" --source-revision "{{revision}}" --compare-required --profile-label "{{profile}}"

# Retained one-shot output facts never bypass actual-result validation.
decode-output-admission-check:
    cargo test -p zencodec --all-features --locked --lib operation::decoding::admission_tests
    cargo clippy -p zencodec --all-features --all-targets --locked -- -D warnings

# Review native output preparation; source export and patch identities are retained.
retained-decode-prepare core_revision jpeg_repository output:
    python3 perf/retained-decode-validation/prepare.py "{{justfile_directory()}}" "{{core_revision}}" "{{jpeg_repository}}" "{{output}}"

retained-decode-native-check:
    cargo test --manifest-path perf/jpeg-policy-validation/Cargo.toml --features retained-output --locked --lib
    cargo clippy --manifest-path perf/jpeg-policy-validation/Cargo.toml --features retained-output --locked --all-targets -- -D warnings

retained-decode-bench-build:
    cargo bench --manifest-path perf/jpeg-policy-validation/Cargo.toml --features retained-output-bench --bench retained_decode --locked --no-run

retained-decode-bench-clippy:
    cargo clippy --manifest-path perf/jpeg-policy-validation/Cargo.toml --features retained-output-bench --locked --all-targets -- -D warnings

retained-decode-bench:
    cargo bench --manifest-path perf/jpeg-policy-validation/Cargo.toml --features retained-output-bench --bench retained_decode --locked -- --mode=confirm --workload-id=retained-jpeg-noise-patches-v1 --rounds=30 --processes=5 --margin=1 --cpu-policy=performance

# Retained output preparation must preserve ordinary results through erasure.
testkit-preparation-check:
    cargo test -p zencodec-testkit --all-targets --locked
    cargo clippy -p zencodec-testkit --all-targets --locked -- -D warnings

# Strict in-flight resource observation; never reinterpret Iterate as Confirm.
compile-time-confirm zenbench root:
    "{{zenbench}}" pair "{{root}}/plan.json" --mode confirm --dry-run > "{{root}}/confirm-dry-run.json"
    "{{zenbench}}" pair "{{root}}/plan.json" --mode confirm --output "{{root}}/confirm-attempt"

# Qualify owned-process lifecycle observation before interpreting compile timings.
compile-observer-prepare root:
    python3 perf/compile-observer-control.py prepare "{{root}}"

compile-observer-run zenbench root:
    "{{zenbench}}" pair "{{root}}/plan.json" --mode confirm --dry-run > "{{root}}/dry-run.json"
    "{{zenbench}}" pair "{{root}}/plan.json" --mode confirm --output "{{root}}/attempt"

# A native success must not hide cancellation raised during the call.
fused-cancellation-check:
    cargo test -p zencodec-testkit --test fused_post_call_cancellation --locked
    cargo clippy -p zencodec-testkit --test fused_post_call_cancellation --locked -- -D warnings

# Final coroutine sink hooks must not turn cancellation into successful Done.
coroutine-sink-cancellation-check:
    cargo test -p zencodec-coroutine --test details --locked
    cargo clippy -p zencodec-coroutine --test details --locked -- -D warnings

# Shared coroutine and I/O effects, errors, cancellation and documentation.
coroutine-io-check:
    cargo test -p zencodec-coroutine -p zencodec-io --all-features --lib --tests --locked
    cargo test -p zencodec-coroutine -p zencodec-io --all-features --doc --locked
    cargo clippy -p zencodec-coroutine -p zencodec-io --all-features --all-targets --locked -- -D warnings

# Native witness: nonzero means retention remains unqualified, not a waived gate.
metadata-retention-probe artifacts:
    cargo run --manifest-path perf/jpeg-policy-validation/Cargo.toml --example metadata_retention --locked -- "{{artifacts}}"

metadata-retention-probe-check:
    cargo clippy --manifest-path perf/jpeg-policy-validation/Cargo.toml --example metadata_retention --locked -- -D warnings

# Shared deterministic one-shot Pulse oracle and its deliberately broken controls.
pulse-checkpoints-check:
    cargo test -p zencodec-testkit --test pulse_checkpoints --locked
    cargo clippy -p zencodec-testkit --test pulse_checkpoints --locked -- -D warnings

# Full testkit plus generated reference/site qualification for shared Pulse controls.
pulse-checkpoints-qualification:
    cargo test -p zencodec-testkit --all-targets --locked
    cargo test -p zencodec-testkit --doc --locked
    cargo clippy -p zencodec-testkit --all-targets --locked -- -D warnings
    python3 docs/generate-code-catalog.py .
    just docs-reference
    just docs-site-check

# Resolve the chosen harness graph before spending CPU on native compilation.
codec-graph-check manifest report="$HOME/tmp/zencodec-codec-graph.json":
    mkdir -p "$(dirname "{{report}}")"
    cargo metadata --manifest-path "{{manifest}}" --locked --format-version 1 > "{{report}}"
    python3 scripts/check-codec-graph.py "{{report}}"

codec-graph-controls:
    python3 scripts/check-codec-graph.test.py

# Native JPEG/PNG adoption of the shared checkpoint oracle, retaining JSON traces.
pulse-native-check artifacts="$HOME/tmp/zencodec-native-pulse":
    just codec-graph-check perf/jpeg-policy-validation/Cargo.toml
    JPEG_POLICY_ARTIFACTS="{{artifacts}}" cargo test --manifest-path perf/jpeg-policy-validation/Cargo.toml --locked pulse_diagnostics:: -- --nocapture --test-threads=1
    cargo clippy --manifest-path perf/jpeg-policy-validation/Cargo.toml --all-targets --locked -- -D warnings

# Required metadata admission: defaults, retained owner, typed causes and operation integration.
metadata-admission-check:
    cargo test --locked -p zencodec --all-features --lib
    cargo test --locked -p zencodec-testkit --test operation_policy
    cargo clippy --locked -p zencodec --all-features --all-targets -- -D warnings

# Generated signatures and the guide must describe the checked metadata candidate.
metadata-admission-docs:
    python3 docs/generate-code-catalog.py .
    just docs-reference
    just docs-site-check

# Exact required metadata readback, independent of capability declarations.
metadata-retention-conformance-check:
    cargo test --locked -p zencodec-testkit --lib metadata_retention
    cargo clippy --locked -p zencodec-testkit --lib --tests -- -D warnings

# Warning migration preserves compatibility tests while exposing checked setters.
metadata-deprecation-check:
    cargo test --locked --workspace --all-features --all-targets
    cargo test --locked --workspace --all-features --doc
    cargo clippy --locked --workspace --all-features --all-targets -- -D warnings
    python3 docs/generate-code-catalog.py .
    just docs-reference
    just docs-site-check

# Continue after compatibility-only annotations; runtime tests use the retained receipt.
metadata-deprecation-followup:
    cargo clippy --locked --workspace --all-features --all-targets -- -D warnings
    python3 docs/generate-code-catalog.py .
    just docs-reference
    just docs-site-check

# Real JPEG retention through the caller recipe; native graph is locked.
checked-metadata-guide-check:
    cargo test --manifest-path perf/jpeg-policy-validation/Cargo.toml --locked checked_metadata_guide -- --nocapture
    cargo clippy --manifest-path perf/jpeg-policy-validation/Cargo.toml --locked --all-targets -- -D warnings
