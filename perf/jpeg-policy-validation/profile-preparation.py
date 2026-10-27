#!/usr/bin/env python3
"""Heaptrack native preparation without touching pixels or using wall-clock claims.

Run through run-heavy. The preload library writes a regular raw file here, then
the standard interpreter processes it. This avoids heaptrack's hard-coded /tmp
FIFO and preserves every raw capture for reanalysis.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess


def run(args, log, **kwargs):
    with log.open("wb") as output:
        subprocess.run(args, stdout=output, stderr=subprocess.STDOUT,
                       check=True, timeout=120, **kwargs)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("executable", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--source-revision", required=True)
    parser.add_argument("--heaptrack-lib", type=Path,
                        default=Path("/usr/lib/heaptrack/libheaptrack_preload.so"))
    parser.add_argument("--interpreter", type=Path,
                        default=Path("/usr/lib/heaptrack/libexec/heaptrack_interpret"))
    args = parser.parse_args()
    executable = args.executable.resolve()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    report = {"source_revision": args.source_revision,
              "binary_sha256": hashlib.sha256(executable.read_bytes()).hexdigest(),
              "profile": "debug; heaptrack; no pixel reads or output publication",
              "dimensions": [2048, 2048], "format": "RGB8_SRGB",
              "native_revision": "4535ec499d88b04f58882df03e61761526c35756",
              "started": datetime.now(timezone.utc).isoformat(), "cells": []}
    (output / "progress.json").write_text(json.dumps(report, indent=2) + "\n")
    for configuration in ["fixed", "progressive"]:
        for stage, count in [(s, n) for s in ["prepare", "construct", "begin"]
                             for n in [1, 4]] + [("refuse", 1)]:
            name = f"{configuration}-{stage}-{count}"
            print(name, flush=True)
            raw = output / f"{name}.raw"
            env = dict(os.environ, LD_PRELOAD=str(args.heaptrack_lib.resolve()),
                       DUMP_HEAPTRACK_OUTPUT=str(raw))
            run([str(executable), stage, str(count), configuration],
                output / f"{name}.run.log", env=env)
            trace = output / f"{name}.heaptrack"
            with raw.open("rb") as source, trace.open("wb") as target, \
                    (output / f"{name}.interpret.log").open("wb") as errors:
                subprocess.run([str(args.interpreter)], stdin=source, stdout=target,
                               stderr=errors, check=True, timeout=120)
            massif = output / f"{name}.massif"
            run(["heaptrack_print", "-f", str(trace), "-M", str(massif),
                 "--print-allocators", "0", "--print-temporary", "0"],
                output / f"{name}.analysis.txt")
            samples = [int(n) for n in re.findall(r"^mem_heap_B=(\d+)$",
                                                 massif.read_text(), re.M)]
            if not samples:
                raise RuntimeError(f"No heap measurements: {name}")
            report["cells"].append({"configuration": configuration, "stage": stage,
                                    "candidates": count, "peak_heap_bytes": max(samples),
                                    "raw_sha256": hashlib.sha256(raw.read_bytes()).hexdigest()})
            report["updated"] = datetime.now(timezone.utc).isoformat()
            (output / "progress.json").write_text(json.dumps(report, indent=2) + "\n")
    report["completed"] = datetime.now(timezone.utc).isoformat()
    (output / "summary.json").write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    main()
