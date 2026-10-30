#!/usr/bin/env python3
"""Check actual package interoperability; explicit patches do not prove registry availability."""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import tarfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("archives", type=Path)
parser.add_argument("output", type=Path)
args = parser.parse_args()
root = Path(__file__).resolve().parents[1]
output = args.output.resolve()
if output.is_relative_to(root):
    parser.error("use a fresh output outside workspace Cargo overrides")
output.mkdir(parents=True, exist_ok=False)
versions = {"zencodec": "0.2.0", "zencodec-io": "0.1.0", "zencodec-testkit": "0.2.0",
            "zenpixels": "0.3.1", "zenpixels-convert": "0.3.1"}
for name, version in versions.items():
    with tarfile.open(args.archives / f"{name}-{version}.crate") as archive:
        archive.extractall(output, filter="data")
for legacy in (True, False):
    project = output / ("published-testkit" if legacy else "candidate-testkit")
    (project / "src").mkdir(parents=True)
    (project / "src/lib.rs").write_text("// Package interoperability consumer.\n")
    dependencies = dict(versions)
    if legacy:
        dependencies["zencodec-testkit"] = "0.1.0"
    manifest = '[package]\nname="release-cohort-consumer"\nversion="0.0.0"\nedition="2024"\n[dependencies]\n'
    manifest += "\n".join(f'{name} = "={version}"' for name, version in dependencies.items())
    manifest += '\n[patch.crates-io]\n'
    manifest += "\n".join(f'{name} = {{ path = {json.dumps(str(output / f"{name}-{version}"))} }}'
                          for name, version in versions.items() if not (legacy and name == "zencodec-testkit"))
    (project / "Cargo.toml").write_text(manifest + "\n")
    metadata = subprocess.check_output(["cargo", "metadata", "--format-version", "1"], cwd=project)
    (project / "graph.json").write_bytes(metadata)
    graph = json.loads(metadata)
    selected = {node["id"] for node in graph["resolve"]["nodes"]}
    family = {name: sorted(p["version"] for p in graph["packages"] if p["id"] in selected and p["name"] == name)
              for name in versions}
    print(project.name, json.dumps(family), flush=True)
    if legacy:
        assert family["zencodec"] == ["0.1.26", "0.2.0"], family
        assert len(family["zenpixels"]) == 2, family
        print("Published testkit reproduces two incompatible core/pixel identities.", flush=True)
    else:
        assert family == {name: [version] for name, version in versions.items()}, family
        (project / "tests").mkdir()
        shutil.copyfile(root / "zencodec-testkit/tests/usage.rs", project / "tests/usage.rs")
        subprocess.run(["cargo", "test", "--locked", "--test", "usage"], cwd=project, check=True)
        subprocess.run(["cargo", "clippy", "--locked", "--all-targets", "--", "-D", "warnings"], cwd=project, check=True)
patches = output / "patches.toml"
patches.write_text('[patch.crates-io]\n' + "\n".join(
    f'{name} = {{ path = {json.dumps(str(output / f"{name}-{version}"))} }}'
    for name, version in versions.items()) + "\n")
for name in ("zencodec-io", "zencodec-testkit"):
    package = output / f"{name}-{versions[name]}"
    for command in (["test", "--all-features", "--all-targets"],
                    ["test", "--all-features", "--doc", "--locked"],
                    ["clippy", "--all-features", "--all-targets", "--locked"]):
        argv = ["cargo", *command, "--config", str(patches)]
        if command[0] == "clippy":
            argv += ["--", "-D", "warnings"]
        subprocess.run(argv, cwd=package, check=True)
print("Candidate package cohort passes; registry publication remains a separate gate.", flush=True)
