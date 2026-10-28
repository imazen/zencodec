#!/bin/bash
# Replay the API usability probes against an exported zencodec revision.
#
# usage: head-replay.sh <new-run-dir> <rev>
#
# The run directory must not exist: the runner creates it and never replaces
# or deletes an earlier run. Every step is bounded by REPLAY_STEP_TIMEOUT
# seconds (default 900) and logs stdout+stderr to <run-dir>/<step>.log;
# steps.tsv records each exit status. The runner exits nonzero if any step
# fails. Heavy: run under run-heavy only in a granted build slot.
#
# The export receives this crate with dependencies.lock as its Cargo.lock and
# builds --locked, so the replay tests the recorded dependency graph. The
# default tests target the current Pulse core; the base-6b0caf34 receipts
# are reproduced in-tree at that base with --features baseline-6b0caf34.
set -uo pipefail

if [ $# -ne 2 ]; then
    echo "usage: $0 <new-run-dir> <rev>" >&2
    exit 2
fi
RUN=$1
REV=$2
GIT_DIR=${ZENCODEC_GIT_DIR:-$HOME/work/zen/zencodec/.git}
STEP_TIMEOUT=${REPLAY_STEP_TIMEOUT:-900}
HERE=$(cd "$(dirname "$0")" && pwd) || exit 2

mkdir -p "$(dirname "$RUN")" || exit 2
if ! mkdir "$RUN"; then
    echo "refusing to reuse existing run directory: $RUN" >&2
    exit 2
fi
RUN=$(cd "$RUN" && pwd) || exit 2
SRC=$RUN/src
CRATE=$SRC/perf/api-usability-validation
export TMPDIR=${TMPDIR:-$HOME/tmp/zfw-scratch}
export CARGO_TARGET_DIR=$RUN/target
mkdir -p "$TMPDIR" || exit 2

failed=0
RESOLVED=
printf 'step\trc\tseconds\tlog\n' > "$RUN/steps.tsv"
step() {
    local name=$1
    shift
    local start rc
    start=$(date +%s)
    "$@" > "$RUN/$name.log" 2>&1
    rc=$?
    printf '%s\t%s\t%s\t%s\n' "$name" "$rc" "$(($(date +%s) - start))" \
        "$RUN/$name.log" >> "$RUN/steps.tsv"
    [ "$rc" -eq 0 ] || failed=1
    return "$rc"
}
bounded() { timeout --kill-after=30 "$STEP_TIMEOUT" "$@"; }

export_tree() {
    mkdir "$SRC" || return 1
    bounded git --git-dir="$GIT_DIR" archive --format=tar "$RESOLVED" | tar -x -C "$SRC" || return 1
    mkdir -p "$CRATE" || return 1
    cp -r "$HERE/Cargo.toml" "$HERE/dependencies.lock" "$HERE/src" "$HERE/tests" "$CRATE/" || return 1
    cp "$HERE/dependencies.lock" "$CRATE/Cargo.lock"
}
record_sources() {
    local resolved
    resolved=$(bounded git --git-dir="$GIT_DIR" rev-parse --verify "$REV^{commit}") || return 1
    RESOLVED=$resolved
    {
        printf 'requested_rev\t%s\n' "$REV"
        printf 'resolved_rev\t%s\n' "$resolved"
        printf 'crate\t%s\n' "$HERE"
        printf 'host\t%s\n' "$(hostname)"
        printf 'started\t%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
        printf 'dependencies.lock sha256\t%s\n' "$(sha256sum < "$HERE/dependencies.lock" | cut -d' ' -f1)"
        printf 'crate files sha256\t%s\n' "$(cd "$HERE" && find Cargo.toml dependencies.lock src tests -type f -print0 |
            sort -z | xargs -0 sha256sum | sha256sum | cut -d' ' -f1)"
    } > "$RUN/SOURCES.tsv"
}
# The replayed issue-173 tests must equal the baseline ones except for STOP.
same_assertions() {
    diff <(sed -n '/^mod common;/,$p' "$CRATE/tests/policy_proposed.rs" | grep -v '^const STOP') \
        <(sed -n '/^mod common;/,$p' "$CRATE/tests/head_proposed.rs" | grep -v '^static STOP')
}
# --locked must leave the recorded graph untouched.
lock_unchanged() { cmp "$HERE/dependencies.lock" "$CRATE/Cargo.lock"; }

if step sources record_sources && step export export_tree && step same-assertions same_assertions; then
    (
        cd "$CRATE" || exit 1
        step tests bounded cargo test --release --locked --test head_present --test head_proposed --no-fail-fast
        step probes bounded cargo run --release --locked
        step lock-unchanged lock_unchanged
        exit "$failed"
    ) || failed=1
fi

if [ "$failed" -ne 0 ]; then
    echo "replay FAILED; see $RUN/steps.tsv" >&2
    exit 1
fi
echo "replay passed; see $RUN/steps.tsv"
