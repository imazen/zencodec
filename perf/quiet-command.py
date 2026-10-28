#!/usr/bin/env python3
"""Bounded, explicitly coordinated suspension of named idle interface processes.

This does not bypass a benchmark resource gate or select/kill processes by name.
Callers must verify that each supplied PID is idle and may be suspended. Linux
pidfds retain identity; a separate watchdog resumes them on parent death or at
the deadline. Only this invocation's child process group is terminated on timeout.
"""
import argparse
import json
import os
from pathlib import Path
import select
import signal
import subprocess
import time


def resume(handles):
    for handle in handles:
        try:
            signal.pidfd_send_signal(handle, signal.SIGCONT)
        except ProcessLookupError:
            pass


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pid", type=int, action="append", required=True)
    parser.add_argument("--seconds", type=float, required=True)
    parser.add_argument("--progress", type=Path, required=True)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if not command or not 0 < args.seconds <= 600 or len(set(args.pid)) != len(args.pid):
        parser.error("require a command, unique PIDs and a deadline in (0, 600] seconds")
    args.progress.parent.mkdir(parents=True, exist_ok=True)
    state = {"command": command, "pids": args.pid, "deadline_seconds": args.seconds}

    def record(stage, **extra):
        state.update(stage=stage, checked_at=time.time(), **extra)
        args.progress.write_text(json.dumps(state, indent=2) + "\n")

    handles = []
    for pid in args.pid:
        # Acquire identity before reading mutable /proc metadata. A reused PID
        # cannot retarget an already-open handle.
        handles.append(os.pidfd_open(pid))
        status = Path(f"/proc/{pid}/status").read_text()
        fields = dict(line.split(":", 1) for line in status.splitlines() if ":" in line)
        if pid == os.getpid() or fields["State"].strip().startswith(("T", "t")):
            raise RuntimeError("refusing self or an already suspended process")
        if int(fields["Uid"].split()[0]) != os.getuid():
            raise RuntimeError("refusing another user's process")
    record("prepared")
    read_end, write_end = os.pipe()
    watchdog = os.fork()
    if watchdog == 0:
        os.close(write_end)
        os.setsid()
        try:
            # Readability includes EOF when the parent dies, even by SIGKILL.
            select.select([read_end], [], [], args.seconds)
        finally:
            resume(handles)
            try:
                state.update(json.loads(args.progress.read_text()))
            except (OSError, ValueError):
                pass
            record("watchdog_restored", timing_valid=False)
            os._exit(0)
    os.close(read_end)
    child = None
    result = 1

    def interrupted(signum, _frame):
        raise InterruptedError(f"signal {signum}")

    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGINT, interrupted)
    try:
        for handle in handles:
            signal.pidfd_send_signal(handle, signal.SIGSTOP)
        record("suspended")
        child = subprocess.Popen(command, start_new_session=True)
        record("running", child_pid=child.pid)
        # Finish before the independent watchdog must restore the interfaces.
        result = child.wait(timeout=max(0.01, args.seconds - 2))
    except (subprocess.TimeoutExpired, InterruptedError) as error:
        result = 124
        record("interrupted", failure=str(error))
    finally:
        if child is not None and child.poll() is None:
            os.killpg(child.pid, signal.SIGTERM)
            try:
                child.wait(timeout=1)
            except subprocess.TimeoutExpired:
                os.killpg(child.pid, signal.SIGKILL)
                child.wait()
        resume(handles)
        os.close(write_end)
        os.waitpid(watchdog, 0)
        for handle in handles:
            os.close(handle)
        record("restored", exit_code=result)
    return result


if __name__ == "__main__":
    raise SystemExit(main())
