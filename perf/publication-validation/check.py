"""Run the publication experiment; caller supplies run-heavy and TMPDIR."""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess

CORE = "e59b3803fc5b212bf4c92648ead4299c4ff0640a"
FAMILY = ["zencodec", "zencodec-io", "zencodec-coroutine", "zenmeta"]
FEATURES = "std,cms,jpeg,png,checked-transcode"
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("export", type=Path)
parser.add_argument("results", type=Path, help="new directory for full logs")
args = parser.parse_args()
args.results.mkdir(parents=True, exist_ok=False)
env = dict(os.environ, CARGO_TERM_COLOR="never")
steps = []


def run(name, command, seconds=1500):
    with (args.results / (name + ".log")).open("w") as log:
        result = subprocess.run(
            command, cwd=args.export, env=env, stdout=log,
            stderr=subprocess.STDOUT, timeout=seconds, check=False,
        )
    steps.append({"name": name, "command": command, "returncode": result.returncode})
    (args.results / "steps.json").write_text(json.dumps(steps, indent=2) + "\n")
    if result.returncode:
        raise SystemExit(f"{name} failed; see {args.results / (name + '.log')}")
    return (args.results / (name + ".log")).read_text()


for index, manifest in enumerate([
    "Cargo.toml", "fuzz/Cargo.toml", "demo/crate/Cargo.toml",
    "tools/metadata-services/Cargo.toml", "zencodecs/fuzz/Cargo.toml",
]):
    text = run(f"graph-{index}", [
        "cargo", "metadata", "--locked", "--format-version", "1", "--manifest-path", manifest,
    ], 300)
    # Cargo warnings may precede the single JSON record.
    graph = json.loads(next(line for line in text.splitlines() if line.startswith('{"packages":')))
    for name in FAMILY:
        packages = [p for p in graph["packages"] if p["name"] == name]
        # Some fuzz/demo graphs do not use all companion crates. Core must be
        # present; every companion that is resolved must have the same source.
        if ((name == "zencodec" and len(packages) != 1) or len(packages) > 1
                or any(not (p.get("source") or "").endswith("#" + CORE) for p in packages)):
            raise SystemExit(f"{manifest}: missing core or split {name} family at {CORE}")

outcomes = {}
for name, feature, target, expected in [
    ("unit", FEATURES + ",publication-witness", ["--lib", "checked_transcode::publication::tests"], 7),
    ("native", FEATURES + ",publication-witness", ["--test", "checked_transcode"], 23),
    ("ordinary", FEATURES, ["--test", "checked_transcode"], 16),
]:
    text = run(name, ["cargo", "test", "--locked", "-p", "zencodecs", "--no-default-features",
                      "--features", feature, *target, "--", "--test-threads=1"])
    tests = re.findall(r"^test (\S+) \.\.\. (\S+)$", text, re.MULTILINE)
    if len(tests) != expected or any(status != "ok" for _, status in tests):
        raise SystemExit(f"{name}: expected {expected} passing controls, got {tests}")
    outcomes[name] = tests
ordinary = [test for test in outcomes["native"] if not test[0].startswith("publication::")]
if ordinary != outcomes["ordinary"]:
    raise SystemExit("Ordinary outcomes changed under the experiment")
for name, feature in [
    ("ordinary", FEATURES),
    ("witness", FEATURES + ",publication-witness"),
    ("async", FEATURES + ",publication-witness,checked-transcode-async"),
]:
    run("clippy-" + name, ["cargo", "clippy", "--locked", "-p", "zencodecs", "--no-default-features",
                          "--features", feature, "--lib", "--test", "checked_transcode",
                          "--example", "checked_transcode", "--", "-D", "warnings"])
(args.results / "outcomes.json").write_text(json.dumps(outcomes, indent=2) + "\n")
print("PASS: 7 unit, 23 native, 16 unchanged ordinary controls; five unified graphs; strict Clippy")
