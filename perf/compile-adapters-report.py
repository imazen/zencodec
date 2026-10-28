#!/usr/bin/env python3
"""Report the io/media cold-build experiment, retaining all observations."""
import argparse
import gzip
import hashlib
import json
from pathlib import Path
import re
import statistics

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("root", type=Path)
parser.add_argument("log", type=Path)
parser.add_argument("receipt", type=Path)
args = parser.parse_args()
root = args.root.resolve()
packet_path = root / "attempt/packet.json.gz"
with gzip.open(packet_path, "rt") as stream:
    packet = json.load(stream)
assert packet["complete"] and packet["decision"]["exit_code"] == 0
assert packet["mode"] == "iterate"
assert len(packet["invocations"]) == 45
plan = packet["plan"]
arms = [plan["baseline"], *plan["candidates"], {"id": "__aa"}]
assert [a["id"] for a in arms] == ["main", "ideal", "__aa"]
groups = {}
for invocation in packet["invocations"]:
    assert invocation["status"] == "completed" and invocation["correctness"] == [True]
    directory = Path(invocation["output"])
    build = json.loads((directory / "build.json").read_text())
    pages = list((directory / "target/cargo-timings").glob("cargo-timing-*.html"))
    assert len(pages) == 1
    match = re.search(r"const UNIT_DATA = (\[.*?\]);", pages[0].read_text(), re.S)
    units = json.loads(match[1])
    names = ["zencodec", "zencodec-io", "zencodec-media"]
    selected = {name: [u["duration"] for u in units if u["name"] == name] for name in names}
    assert all(len(values) == 1 for values in selected.values()), selected
    key = (plan["cases"][invocation["case"]]["id"], arms[invocation["arm"]]["id"])
    groups.setdefault(key, []).append({"wall_seconds": build["wall_seconds"],
        "cargo_peak_rss_kib": build["maximum_resident_kib"],
        **{name + "_unit_seconds": values[0] for name, values in selected.items()}})


def stats(values):
    return {"median": statistics.median(values), "min": min(values), "max": max(values)}


measurements = [{"case": case, "arm": arm, "n": len(values),
    **{field: stats([value[field] for value in values]) for field in values[0]}}
    for (case, arm), values in sorted(groups.items())]
assert all(m["n"] == 5 for m in measurements)
lines = [line for line in args.log.read_text().splitlines() if line.startswith("run-heavy: done")]
assert len(lines) == 1 and "rc=0" in lines[0]
receipt = {"schema": 1, "source_main": plan["baseline"]["revision"],
    "source_ideal": plan["candidates"][0]["revision"], "artifact_directory": str(root),
    "measurements": measurements,
    "quality": {"confirmed": False, "mode": "iterate", "invocations": 45,
        "cpu_load_flags": sum(any(o["cpu_load"] > .05 for o in i["observations"]) for i in packet["invocations"]),
        "heavy_process_flags": sum(any(o["heavy_process_count"] > 0 for o in i["observations"]) for i in packet["invocations"]),
        "placement": packet["placement"]["status"]},
    "scope": "core/io/media re-export consumer; no codecs, render/CMS, tests or application monomorphization; empty target, warm OS cache, prefetched dependencies, two jobs, no incremental/compiler wrapper; std enables std on each package that exposes it: main media has no std feature, review media includes its new std drivers",
    "resource_wrapper": lines[0], "log": str(args.log),
    "manifests": {f"{name}/{feature}": (root / name / feature / "Cargo.toml").read_text()
                  for name in ["main", "ideal"] for feature in ["default", "std"]},
    "hashes": {str(p): hashlib.sha256(p.read_bytes()).hexdigest() for p in
               [packet_path, root / "plan.json", root / "runner-measured.py", root / "toolchain.txt", args.log]}}
args.receipt.write_text(json.dumps(receipt, indent=2) + "\n")
print(json.dumps({"measurements": measurements, "quality": receipt["quality"]}, indent=2))
