#!/usr/bin/env bash
# Run in an exported zencodec tree, under the caller's resource wrapper.
# Usage: bash perf/io-validation/run-batching.sh NEW_OUTPUT_DIR [CPU_SET] [synthetic|qoi]
set -euo pipefail
root=$(pwd)
out=$(realpath -m "$1")
cpus=${2:-2,3}
suite=${3:-synthetic}
test ! -e "$root/.git"
test ! -e "$out"
mkdir -p "$out"
export TMPDIR=${TMPDIR:?set TMPDIR to disk-backed scratch}
export CARGO_INCREMENTAL=0
export CARGO_TARGET_DIR="$root/target"
bench_args=(-p zencodec-io --features tokio --bench transcode)
bench_name=transcode
if [ "$suite" = qoi ]; then
    bench_args=(--manifest-path perf/io-validation/Cargo.toml -p io-validation --bench qoi)
    bench_name=qoi
elif [ "$suite" != synthetic ]; then
    echo "unknown suite: $suite" >&2
    exit 1
fi
trap 'python3 "$root/perf/io-validation/prototype.py" "$root" baseline' EXIT
for variant in baseline rows bounded; do
    python3 perf/io-validation/prototype.py "$root" "$variant"
    cargo run --locked -p zencodec-io --features tokio --example io_contract_probes > "$out/$variant-probes.log" 2>&1
    cargo bench --locked "${bench_args[@]}" --no-run --message-format=json > "$out/$variant-build.jsonl" 2> "$out/$variant-build.log"
    binary=$(python3 - "$out/$variant-build.jsonl" "$bench_name" <<'PY'
import json,sys
for line in open(sys.argv[1]):
    d=json.loads(line)
    if d.get("target",{}).get("name")==sys.argv[2] and d.get("executable"):
        print(d["executable"])
PY
)
    cp "$binary" "$out/transcode-$variant"
    sha256sum "$out/transcode-$variant"
done
# Reverse the order on the second pass. No cross-binary significance claim:
# each zenbench file describes only its own interleaved inline/pool arms.
for pass in 1 2; do
    variants="baseline rows bounded"
    if [ "$pass" = 2 ]; then variants="bounded rows baseline"; fi
    for variant in $variants; do
        taskset -c "$cpus" "$out/transcode-$variant" --cpu-policy=unrestricted > "$out/$variant-$pass.log" 2>&1
        echo "completed $variant pass $pass"
    done
done
