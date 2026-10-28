#!/usr/bin/env python3
"""Build the eight-crate Rustdoc reference beside the guide from this revision."""
import json
import os
from pathlib import Path
import shutil
import subprocess

root = Path(__file__).resolve().parent.parent
revision = os.environ.get("GITHUB_SHA") or subprocess.check_output(
    ["git", "rev-parse", "HEAD"], cwd=root, text=True).strip()
target = Path(os.environ.get("CARGO_TARGET_DIR", str(root / "target"))).resolve()
env = os.environ.copy()
env["RUSTDOCFLAGS"] = (env.get("RUSTDOCFLAGS", "") + " -D warnings").strip()
# The library root intentionally does not commit Cargo.lock. Preserve an
# existing export lock; resolve once on a fresh checkout, as normal CI does.
if not (root / "Cargo.lock").exists():
    subprocess.run(["cargo", "metadata", "--all-features", "--format-version", "1"],
                   cwd=root, env=env, stdout=subprocess.DEVNULL, check=True)
renderer_lock = root / "zencodec-render/Cargo.lock"
if not renderer_lock.exists():
    shutil.copyfile(root / "zencodec-render/dependencies.lock", renderer_lock)
for selection in [["--workspace"], ["--manifest-path", "zencodec-render/Cargo.toml"]]:
    subprocess.run(["cargo", "doc", *selection, "--all-features", "--no-deps",
                    "--locked", "--target-dir", str(target)],
                   cwd=root, env=env, check=True)
packages = ["zencodec", "zencodec-io", "zencodec-coroutine", "zenmeta",
            "zencodec-testkit", "zencodec-media", "zencodec-media-testkit", "zencodec-render"]
for package in packages:
    path = target / "doc" / package.replace("-", "_") / "index.html"
    if not path.is_file():
        raise SystemExit("Missing Rustdoc crate entry: " + str(path))
site = root / "docs/_site"
site.mkdir(parents=True, exist_ok=True)
destination = site / "reference"
if destination.exists():
    backup = site / ("reference.previous-" + revision[:12])
    if backup.exists():
        raise SystemExit("Reference backup already exists; preserve it before replacing")
    destination.rename(backup)
shutil.copytree(target / "doc", destination)
(site / "reference-build.json").write_text(json.dumps({
    "revision": revision, "packages": packages,
    "features": "all", "target": "generation host; platform-specific items may differ"
}, indent=2) + "\n")
print("Built Rustdoc reference for " + revision)
