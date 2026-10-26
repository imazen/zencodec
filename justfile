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
    cargo semver-checks check-release -p zencodec --baseline-version 0.1.26 --all-features

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

# Check the independently versioned testkit against its published baseline.
# Version 0.2.0 carries the approved implementor-only breaking changes.
testkit-semver:
    cargo semver-checks check-release -p zencodec-testkit --baseline-version 0.1.0

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
    mkdir -p "{{export}}/src/operation"
    cp src/operation.rs src/lib.rs src/set.rs src/info.rs src/frame.rs src/frame_storage.rs src/image.rs src/resolution.rs "{{export}}/src/"
    cp src/operation/*.rs "{{export}}/src/operation/"
    cp src/traits/encoding.rs src/traits/dyn_encoding.rs src/traits/mod.rs "{{export}}/src/traits/"
    cp zencodec-testkit/tests/operation_*.rs "{{export}}/zencodec-testkit/tests/"
    cp justfile "{{export}}/justfile"

# Native frame facts and policy publication guard share the still-image resolver.
frame-policy-sync export:
    just operation-sync "{{export}}"
    cp zencodec-media/src/*.rs "{{export}}/zencodec-media/src/"
    cp zencodec-media-testkit/tests/frame_policy.rs zencodec-media-testkit/tests/image_output.rs "{{export}}/zencodec-media-testkit/tests/"

frame-policy-check:
    cargo test -p zencodec-media-testkit --test frame_policy --locked
    cargo clippy -p zencodec -p zencodec-media -p zencodec-media-testkit --all-targets --locked -- -D warnings
    cargo check -p zencodec -p zencodec-media --no-default-features --locked

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
    cp perf/av1-policy-validation/Cargo.toml "{{export}}/perf/av1-policy-validation/"
    cp perf/av1-policy-validation/src/*.rs "{{export}}/perf/av1-policy-validation/src/"
    mkdir -p "{{export}}/docs/examples"
    cp docs/examples/checked_video.rs "{{export}}/docs/examples/"

av1-policy-resolve:
    cargo generate-lockfile --manifest-path perf/av1-policy-validation/Cargo.toml

av1-policy-setup:
    cp perf/av1-policy-validation/dependencies.lock perf/av1-policy-validation/Cargo.lock

av1-policy-check artifacts:
    AV1_POLICY_ARTIFACTS="{{artifacts}}" cargo test --manifest-path perf/av1-policy-validation/Cargo.toml --locked -- --nocapture --test-threads=1
    cargo clippy --manifest-path perf/av1-policy-validation/Cargo.toml --all-targets --locked -- -D warnings

av1-policy-guide-check:
    cargo test --manifest-path perf/av1-policy-validation/Cargo.toml --locked guide_ -- --nocapture
    cargo clippy --manifest-path perf/av1-policy-validation/Cargo.toml --all-targets --locked -- -D warnings

av1-output-check artifacts:
    AV1_POLICY_ARTIFACTS="{{artifacts}}" cargo test --manifest-path perf/av1-policy-validation/Cargo.toml --locked output_native_ -- --nocapture --test-threads=1
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
    just io-core-sync "{{export}}"
    cp zencodec-media/src/*.rs "{{export}}/zencodec-media/src/"
    test ! -e "{{export}}/.git"
    test ! -e "{{export}}/.jj"
    mkdir -p "{{export}}/perf/spatial-validation/src"
    cp perf/spatial-validation/Cargo.toml "{{export}}/perf/spatial-validation/"
    cp perf/spatial-validation/oracle.py "{{export}}/perf/spatial-validation/"
    cp perf/spatial-validation/jxl_oracle.c "{{export}}/perf/spatial-validation/"
    cp perf/spatial-validation/src/*.rs "{{export}}/perf/spatial-validation/src/"
    mkdir -p "{{export}}/docs/examples"
    cp docs/examples/checked_source.rs "{{export}}/docs/examples/"
    cp src/error.rs "{{export}}/src/error.rs"
    cp justfile "{{export}}/justfile"

spatial-validation-resolve:
    cargo metadata --manifest-path perf/spatial-validation/Cargo.toml --format-version 1 > /dev/null

spatial-validation-setup:
    cp perf/spatial-validation/dependencies.lock perf/spatial-validation/Cargo.lock

spatial-oracle-install python:
    uv pip install --python "{{python}}" --only-binary :all: Pillow==13.0.0

spatial-validation-check artifacts python:
    mkdir -p "{{artifacts}}-oracle"
    cc -std=c11 -Wall -Wextra -Werror perf/spatial-validation/jxl_oracle.c -ljxl -o "{{artifacts}}-oracle/jxl-oracle"
    SPATIAL_ARTIFACTS="{{artifacts}}" SPATIAL_GENERATED_ARTIFACTS="{{artifacts}}-generated" SPATIAL_ORACLE_PYTHON="{{python}}" JXL_SELECTION_ORACLE="{{artifacts}}-oracle/jxl-oracle" cargo test --manifest-path perf/spatial-validation/Cargo.toml --locked -- --nocapture --test-threads=1
    cargo clippy --manifest-path perf/spatial-validation/Cargo.toml --all-targets --locked -- -D warnings

spatial-consumer-check:
    cargo test --manifest-path perf/spatial-validation/Cargo.toml --locked common_ -- --nocapture
    cargo clippy --manifest-path perf/spatial-validation/Cargo.toml --all-targets --locked -- -D warnings

spatial-cancellation-check filter="cancellation_":
    cargo test --manifest-path perf/spatial-validation/Cargo.toml --locked "{{filter}}" -- --nocapture --test-threads=1
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

# Real TIFF sample carriage through the common pixel owner and row converter.
spatial-samples-check:
    cargo test --manifest-path perf/spatial-validation/Cargo.toml --locked strip_tests -- --nocapture
    cargo test --manifest-path perf/spatial-validation/Cargo.toml --locked tiff_pixels -- --nocapture
    cargo test --manifest-path perf/spatial-validation/Cargo.toml --locked common_ -- --nocapture
    cargo clippy --manifest-path perf/spatial-validation/Cargo.toml --all-targets --locked -- -D warnings

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

coroutine-cost-check:
    cargo clippy -p zencodec-io --bench coroutine_admission --locked -- -D warnings

coroutine-cost-bench cpu="4":
    taskset -c {{cpu}} cargo bench -p zencodec-io --bench coroutine_admission --locked -- --cpu-policy=unrestricted --median-of-passes=3

# Refresh an owned validation export without inheriting parent workspace patches.
io-core-sync export:
    test ! -e "{{export}}/.git"
    test ! -e "{{export}}/.jj"
    test -f "{{export}}/zencodec-io/Cargo.toml"
    cp src/lib.rs src/storage.rs src/frame.rs src/frame_storage.rs src/image.rs "{{export}}/src/"
    cp zencodec-io/src/*.rs "{{export}}/zencodec-io/src/"
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

jpeg-policy-check:
    JPEG_POLICY_ARTIFACTS="$HOME/tmp/zencodec-jpeg-oracles" cargo test --manifest-path perf/jpeg-policy-validation/Cargo.toml --locked -- --nocapture --test-threads=1
    cargo clippy --manifest-path perf/jpeg-policy-validation/Cargo.toml --all-targets --locked -- -D warnings

jpeg-policy-sync export:
    test ! -e "{{export}}/.git"
    test ! -e "{{export}}/.jj"
    mkdir -p "{{export}}/perf/jpeg-policy-validation/src"
    cp perf/jpeg-policy-validation/Cargo.toml perf/jpeg-policy-validation/dependencies.lock "{{export}}/perf/jpeg-policy-validation/"
    cp perf/jpeg-policy-validation/src/*.rs "{{export}}/perf/jpeg-policy-validation/src/"
    mkdir -p "{{export}}/docs/examples"
    cp docs/examples/jpeg_policy.rs "{{export}}/docs/examples/"
    cp docs/examples/pixel_conversion.rs docs/examples/checked_operations.rs "{{export}}/docs/examples/"
    cp justfile "{{export}}/justfile"

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
docs-site-check:
    node docs/build-site.mjs
    node --test docs/site/site.test.mjs docs/site/source.test.mjs

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
