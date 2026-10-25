#!/usr/bin/env bash
# Fixed-work profiles. Run serially under the host's resource limiter.
# Usage: profile-disk.sh NEW_OUTPUT_DIR FIXTURE [CPU_SET]
set -euo pipefail
out=$(realpath -m "$1")
fixture=$(realpath "$2")
cpus=${3:-2,3}
test ! -e "$out"
test -f "$fixture"
mkdir -p "$out"
binary="${CARGO_TARGET_DIR:?set CARGO_TARGET_DIR}/release/disk_probe"
test -x "$binary"
sha256sum "$binary" "$fixture" > "$out/inputs.sha256"
for mode in seek file; do
    taskset -c "$cpus" perf stat -r 3 -x, \
        -e instructions,cycles,cache-misses,context-switches \
        -o "$out/$mode-perf.csv" \
        "$binary" "$mode" 10000 "$fixture" > "$out/$mode-perf.stdout"
    for repetitions in 1 10000; do
        prefix="$out/$mode-$repetitions"
        taskset -c "$cpus" heaptrack -o "$prefix" \
            "$binary" "$mode" "$repetitions" "$fixture" > "$prefix.log" 2>&1
        # The installed heaptrack chooses gzip or zstd compression.
        capture="$prefix.zst"
        if [ ! -f "$capture" ]; then capture="$prefix.gz"; fi
        test -f "$capture"
        heaptrack_print -f "$capture" -n 5 > "$prefix.txt"
    done
done
