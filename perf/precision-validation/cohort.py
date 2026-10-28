#!/usr/bin/env python3
"""Resolve the paired precision graph in an isolated export, then verify identity.

Run through run-heavy. Existing dependency locks seed resolution; unrelated
package movements are recorded for review, never silently declared acceptable.
This does not execute codec or pixel tests.
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys
import tomllib

REVISION = "283655c5424e0ad7ebc6d2acd94670fac1e29158"
GRAPHS = [
    ".", "fuzz", "zencodec-render", "perf/api-usability-validation",
    "perf/native-contract-validation", "perf/native-contract-validation/spatial-native",
    "perf/native-contract-validation/video-native", "perf/multipass-validation",
    "perf/multipass-validation/native", "perf/policy-check",
    "perf/av1-policy-validation", "perf/jpeg-policy-validation",
    "perf/io-validation", "perf/spatial-validation",
]


def packages(path):
    if not path.exists():
        return set()
    return {(p["name"], p["version"], p.get("source", "path"))
            for p in tomllib.loads(path.read_text()).get("package", [])
            if p["name"] not in ("zenpixels", "zenpixels-convert")}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("export", type=Path)
    parser.add_argument("results", type=Path)
    parser.add_argument("--graphs", nargs="+", choices=GRAPHS, default=GRAPHS)
    args = parser.parse_args()
    root = args.export.resolve()
    if (root / ".git").exists() or (root / ".jj").exists():
        raise ValueError("Use an isolated source export, not a checkout")
    args.results.mkdir(parents=True, exist_ok=False)
    results = []
    for graph in args.graphs:
        directory = root / graph
        label = graph.replace("/", "_").replace(".", "root")
        if graph == "perf/io-validation" and not (directory / "vendor/sources.json").exists():
            with (args.results / "io-source-setup.log").open("w") as log:
                subprocess.run([sys.executable, str(directory / "setup.py")], cwd=root,
                               stdout=log, stderr=subprocess.STDOUT, check=True, timeout=300)
        seed = directory / "dependencies.lock"
        if graph == "perf/io-validation":
            seed = directory / "locks/harness.lock"
        lock = directory / "Cargo.lock"
        if seed.exists():
            lock.write_bytes(seed.read_bytes())
        before = packages(lock)
        command = ["cargo", "metadata", "--format-version", "1", "--all-features",
                   "--manifest-path", str(directory / "Cargo.toml")]
        with (args.results / (label + ".json")).open("w") as out, \
                (args.results / (label + ".log")).open("w") as err:
            subprocess.run(command, cwd=root, stdout=out, stderr=err,
                           check=True, timeout=300)
        metadata = json.loads((args.results / (label + ".json")).read_text())
        identities = {}
        for name in ("zenpixels", "zenpixels-convert"):
            matches = [p for p in metadata["packages"] if p["name"] == name]
            if name == "zenpixels" and not matches:
                raise ValueError(f"{graph}: missing {name}")
            if len(matches) > 1:
                raise ValueError(f"{graph}: duplicated {name}")
            for package in matches:
                source = package["source"] or ""
                if not source.startswith("git+https://github.com/imazen/zenpixels?") or not source.endswith("#" + REVISION):
                    raise ValueError(f"{graph}: unexpected {name} source: {source}")
                identities[name] = source
        after = packages(lock)
        result = {"graph": graph, "pixels": identities,
                  "other_removed": sorted(before - after),
                  "other_added": sorted(after - before), "seeded": seed.exists()}
        results.append(result)
        (args.results / "resolution.json").write_text(json.dumps(results, indent=2) + "\n")
        if seed.exists():
            seed.write_bytes(lock.read_bytes())
        print(f"{graph}: one paired pixel source; "
              f"{len(before - after)} other removals, {len(after - before)} additions", flush=True)


if __name__ == "__main__":
    main()
