#!/bin/bash
# Controls for head-replay.sh with fake `git` and `cargo` on PATH: the runner
# must fail on every failed or hung step, refuse an occupied run directory,
# and succeed only when every step succeeds. No real build or export.
#
# usage: head-replay-controls.sh    (exits nonzero if any control fails)
# REPLAY_SCRIPT=<path> runs the controls against another runner, e.g. an older
# revision, to show that they detect its false successes.
set -uo pipefail

HERE=$(cd "$(dirname "$0")" && pwd) || exit 2
mkdir -p "$HOME/tmp" || exit 2
WORK=$(mktemp -d "$HOME/tmp/head-replay-controls.XXXXXX") || exit 2
echo "controls workspace: $WORK"

# A private copy of the crate, so a control can alter it.
CRATE=$WORK/crate
mkdir -p "$CRATE" || exit 2
cp -r "$HERE/Cargo.toml" "$HERE/dependencies.lock" "$HERE/head-replay.sh" \
    "$HERE/src" "$HERE/tests" "$CRATE/" || exit 2

BIN=$WORK/bin
mkdir -p "$BIN" "$WORK/tree" || exit 2
echo fixture > "$WORK/tree/README"
cat > "$BIN/git" <<'EOF'
#!/bin/bash
# Fake git: FAKE_GIT_MODE=ok|archive-fails|archive-fails-after-output|rev-parse-fails
[ "${1#--git-dir=}" != "$1" ] && shift
case "$1" in
rev-parse)
    [ "${FAKE_GIT_MODE:-ok}" = rev-parse-fails ] && exit 128
    echo 0123456789abcdef0123456789abcdef01234567 ;;
archive)
    case "${FAKE_GIT_MODE:-ok}" in
    archive-fails) exit 128 ;;
    archive-fails-after-output) tar -c -C "$FAKE_TREE" . ; exit 1 ;;
    *) tar -c -C "$FAKE_TREE" . ;;
    esac ;;
*) exit 2 ;;
esac
EOF
cat > "$BIN/cargo" <<'EOF'
#!/bin/bash
# Fake cargo: FAKE_CARGO_TEST_RC, FAKE_CARGO_RUN_RC, FAKE_CARGO_SLEEP, FAKE_CARGO_EDIT_LOCK
echo "cargo $*" >> "$FAKE_CALLS"
[ -n "${FAKE_CARGO_SLEEP:-}" ] && sleep "$FAKE_CARGO_SLEEP"
[ -n "${FAKE_CARGO_EDIT_LOCK:-}" ] && echo "# changed" >> Cargo.lock
case "$1" in
test) exit "${FAKE_CARGO_TEST_RC:-0}" ;;
run) exit "${FAKE_CARGO_RUN_RC:-0}" ;;
*) exit 2 ;;
esac
EOF
chmod +x "$BIN/git" "$BIN/cargo" || exit 2

pass=0
fail=0
# control <name> <expected: pass|fail> [VAR=value...]
control() {
    local name=$1 expect=$2
    shift 2
    local run=$WORK/runs/$name rc
    env "$@" PATH="$BIN:$PATH" FAKE_TREE="$WORK/tree" FAKE_CALLS="$WORK/$name.calls" \
        ZENCODEC_GIT_DIR="$WORK/none.git" TMPDIR="$WORK/scratch" \
        "${REPLAY_SCRIPT:-$CRATE/head-replay.sh}" "$run" fake-rev > "$WORK/$name.out" 2>&1
    rc=$?
    if { [ "$expect" = pass ] && [ "$rc" -eq 0 ]; } || { [ "$expect" = fail ] && [ "$rc" -ne 0 ]; }; then
        printf 'ok\t%s\trc=%s\n' "$name" "$rc"
        pass=$((pass + 1))
    else
        printf 'FAILED\t%s\texpected %s, rc=%s (%s)\n' "$name" "$expect" "$rc" "$WORK/$name.out"
        fail=$((fail + 1))
    fi
}
check() {
    if eval "$2"; then
        printf 'ok\t%s\n' "$1"
        pass=$((pass + 1))
    else
        printf 'FAILED\t%s\t%s\n' "$1" "$2"
        fail=$((fail + 1))
    fi
}

control all-steps-pass pass
check all-steps-pass/both-cargo-steps-ran "[ \$(wc -l < '$WORK/all-steps-pass.calls') -eq 2 ]"

control tests-fail fail FAKE_CARGO_TEST_RC=101
check tests-fail/probes-still-ran "grep -q '^cargo run' '$WORK/tests-fail.calls'"
check tests-fail/recorded "grep -qP '^tests\t101\t' '$WORK/runs/tests-fail/steps.tsv'"

control probes-fail fail FAKE_CARGO_RUN_RC=1
check probes-fail/recorded "grep -qP '^probes\t1\t' '$WORK/runs/probes-fail/steps.tsv'"

control export-fails fail FAKE_GIT_MODE=archive-fails
check export-fails/no-cargo "[ ! -e '$WORK/export-fails.calls' ]"

control export-fails-after-output fail FAKE_GIT_MODE=archive-fails-after-output
check export-fails-after-output/no-cargo "[ ! -e '$WORK/export-fails-after-output.calls' ]"

control unresolved-rev fail FAKE_GIT_MODE=rev-parse-fails
check unresolved-rev/no-cargo "[ ! -e '$WORK/unresolved-rev.calls' ]"

control step-timeout fail REPLAY_STEP_TIMEOUT=1 FAKE_CARGO_SLEEP=5
check step-timeout/recorded "grep -qP '^tests\t124\t' '$WORK/runs/step-timeout/steps.tsv'"

control lock-changed fail FAKE_CARGO_EDIT_LOCK=1
check lock-changed/recorded "grep -qP '^lock-unchanged\t1\t' '$WORK/runs/lock-changed/steps.tsv'"

mkdir -p "$WORK/runs/occupied" && echo keep > "$WORK/runs/occupied/sentinel"
control occupied fail
check occupied/untouched "[ \"\$(ls '$WORK/runs/occupied')\" = sentinel ] && [ \"\$(cat '$WORK/runs/occupied/sentinel')\" = keep ]"
check occupied/no-cargo "[ ! -e '$WORK/occupied.calls' ]"

# The replayed issue-173 tests must not drift from the baseline assertions.
sed -i 's/assert_eq!(seen, \[1\]/assert_eq!(seen, [0]/' "$CRATE/tests/head_proposed.rs"
control assertion-drift fail
check assertion-drift/no-cargo "[ ! -e '$WORK/assertion-drift.calls' ]"

echo "controls: $pass passed, $fail failed"
echo "preserved control logs: $WORK"
if [ "$fail" -eq 0 ]; then
    exit 0
fi
echo "kept for inspection: $WORK"
exit 1
