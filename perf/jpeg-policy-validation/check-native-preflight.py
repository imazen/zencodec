#!/usr/bin/env python3
"""Execute the effective-job preflight scenarios and all JPEG controls in an
isolated export.

The scenarios (`src/native_preflight.rs`) run on the pinned zenjpeg's shared
planning hooks (`EncodeJob::plan_encode`, `Encoder::begin_declared`). They
once required applying a patch to an older zenjpeg; the pinned revision now
implements them, so nothing is patched here.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
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
    env = dict(os.environ)
    scratch = directory / "scratch"
    scratch.mkdir()
    env["TMPDIR"] = str(scratch)
    env["CARGO_TARGET_DIR"] = str(args.target.resolve() if args.target else directory / "target")
    env["JPEG_POLICY_ARTIFACTS"] = str(directory / "oracle")
    (directory / "experiment.json").write_text(json.dumps({
        "native_source": native[0]["source"],
        "export": str(export),
        "scope": "effective JPEG row-output planning through the shared hooks; "
                 "target-zq and MPF not qualified",
    }, indent=2) + "\n")
    for command in [
        ["test", "--", "--nocapture", "--test-threads=1"],
        ["clippy", "--all-targets", "--", "-D", "warnings"],
    ]:
        subprocess.run([
            "cargo", command[0], "--manifest-path", str(manifest),
            "--features", "native-preflight", "--locked", *command[1:],
        ], cwd=export, env=env, check=True, timeout=300)
    print("Native preflight scenarios and existing JPEG controls passed", flush=True)


if __name__ == "__main__":
    main()
