#!/usr/bin/env python3
"""Serial heaptrack captures; preserve the binary, full reports and stacks."""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys

binary, output = map(Path, sys.argv[1:3])
binary = binary.resolve(strict=True)
reanalyze = sys.argv[3:] == ["--reanalyze"]
if not reanalyze:
    output.mkdir(parents=True, exist_ok=False)
saved = output / "policy_probe"
if not reanalyze:
    shutil.copy2(binary, saved)
else:
    assert saved.read_bytes() == binary.read_bytes(), "analysis binary changed"
    for prior in output.iterdir():
        if prior.suffix in (".json", ".txt", ".stacks"):
            backup = prior.with_name(prior.name + ".bak")
            if backup.exists():
                raise RuntimeError(f"Backup already exists: {backup}")
            prior.rename(backup)
modes = ["direct-preset", "closure-fallback", "dyn-preset", "erased-success",
         "decline", "callback-error", "no-alternative"]
results = []
for mode, count in [("direct-preset", 0)] + [(mode, 10000) for mode in modes]:
    label = mode if count else "startup-control"
    prefix = output / label
    if not reanalyze:
        with prefix.with_suffix(".log").open("w") as log:
            subprocess.run(["heaptrack", "--record-only", "-o", str(prefix), str(saved), mode, str(count)],
                           check=True, stdout=log, stderr=subprocess.STDOUT)
    captures = list(output.glob(label + "*.gz")) + list(output.glob(label + "*.zst"))
    if len(captures) != 1:
        raise RuntimeError(f"Expected one capture for {label}, got {captures}")
    stacks = prefix.with_suffix(".stacks")
    with prefix.with_suffix(".txt").open("w") as report:
        subprocess.run(["heaptrack_print", "-f", str(captures[0]), "-F", str(stacks),
                        "--flamegraph-cost-type", "allocations", "-n", "30"],
                       check=True, stdout=report, stderr=subprocess.STDOUT)
    total = operation = 0
    demangled = subprocess.run(["rustfilt"], input=stacks.read_text(), text=True,
                              capture_output=True, check=True).stdout
    prefix.with_suffix(".demangled.stacks").write_text(demangled)
    for line in demangled.splitlines():
        frames, allocations = line.rsplit(" ", 1)
        allocations = int(allocations)
        total += allocations
        if "policy_probe::workload::operation" in frames:
            operation += allocations
    if mode in ("decline", "callback-error", "no-alternative") and operation == 0:
        raise RuntimeError(f"Missing operation allocation stacks for {mode}")
    results.append({"mode": label, "operations": count, "total_allocation_calls": total,
                    "allocation_calls_under_operation": operation})
summary = {"binary_sha256": hashlib.sha256(saved.read_bytes()).hexdigest(),
           "count_method": "sum of allocation-cost flamegraph leaves; operation symbol filter",
           "results": results}
(output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
print(json.dumps(summary, indent=2))
