#!/usr/bin/env python3
"""Reproduce the native patch and execute all JPEG controls in an isolated export."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--target", type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[2]
    directory = args.directory.resolve()
    directory.mkdir(parents=True, exist_ok=False)
    export = directory / "framework"
    export.mkdir()
    archive = directory / "framework.tar"
    with archive.open("wb") as stream:
        subprocess.run(["git", "archive", "HEAD"], cwd=root, stdout=stream,
                       check=True, timeout=60)
    with tarfile.open(archive) as bundle:
        bundle.extractall(export, filter="data")
    harness = export / "perf/jpeg-policy-validation"
    (harness / "Cargo.lock").write_bytes((harness / "dependencies.lock").read_bytes())
    manifest = harness / "Cargo.toml"
    metadata = json.loads(subprocess.check_output([
        "cargo", "metadata", "--locked", "--format-version", "1",
        "--manifest-path", str(manifest),
    ], cwd=export, timeout=120))
    native = [p for p in metadata["packages"] if p["name"] == "zenjpeg"]
    if len(native) != 1 or not (native[0]["source"] or "").startswith("git+"):
        raise ValueError("Expected one immutable git-sourced native JPEG baseline")
    native_repository = Path(native[0]["manifest_path"]).parent.parent
    env = dict(os.environ)
    scratch = directory / "scratch"
    scratch.mkdir()
    env["TMPDIR"] = str(scratch)
    env["CARGO_TARGET_DIR"] = str(args.target.resolve() if args.target else directory / "target")
    env["JPEG_POLICY_ARTIFACTS"] = str(directory / "oracle")
    subprocess.run([
        sys.executable, str(Path(__file__).with_name("prepare-native-preflight.py")),
        str(export), str(native_repository), str(directory / "native"),
    ], cwd=root, env=env, check=True, timeout=180)
    for command in [
        ["test", "--", "--nocapture", "--test-threads=1"],
        ["clippy", "--all-targets", "--", "-D", "warnings"],
    ]:
        subprocess.run([
            "cargo", command[0], "--manifest-path", str(harness / "Cargo.toml"),
            "--features", "native-preflight", "--locked", *command[1:],
        ], cwd=export, env=env, check=True, timeout=300)
    print("Native preflight implementation and existing JPEG controls passed", flush=True)


if __name__ == "__main__":
    main()
