#!/usr/bin/env python3
"""Test the actual normalized crate; paired-source mode is not registry proof."""
import argparse
import json
import pathlib
import subprocess
import tarfile
import tomllib

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("output", type=pathlib.Path, help="new directory outside the workspace")
parser.add_argument("--paired-source", action="store_true", help="explicitly use the workspace's pinned unpublished pixel dependency")
args = parser.parse_args()
root = pathlib.Path(__file__).resolve().parent.parent
output = args.output.resolve()
if output.is_relative_to(root):
    parser.error("output must be outside the workspace and its Cargo overrides")
output.mkdir(parents=True, exist_ok=False)
manifest = tomllib.loads((root / "Cargo.toml").read_text())
metadata = json.loads(subprocess.check_output(
    ["cargo", "metadata", "--no-deps", "--format-version", "1"], cwd=root))
name = f"zencodec-{manifest['package']['version']}"
archive = pathlib.Path(metadata["target_directory"]) / "package" / f"{name}.crate"
with tarfile.open(archive) as package:
    package.extractall(output, filter="data")
config = []
if args.paired_source:
    pin = manifest["patch"]["crates-io"]["zenpixels"]
    for key in ("git", "rev"):
        config += ["--config", f"patch.crates-io.zenpixels.{key}={json.dumps(pin[key])}"]
    print("PAIRED SOURCE: validates crate contents, NOT registry availability", flush=True)
for command in (
    ["test", "--all-features", "--all-targets"],
    ["test", "--all-features", "--doc"],
    ["clippy", "--all-features", "--all-targets"],
):
    argv = ["cargo", *command, *config]
    if command[0] == "clippy":
        argv += ["--", "-D", "warnings"]
    print("Running:", " ".join(argv), flush=True)
    subprocess.run(argv, cwd=output / name, check=True)
