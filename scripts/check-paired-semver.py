"""Compare the actual patched workspace API with an unpatched registry baseline.

cargo-semver-checks builds an isolated placeholder for the current crate, which
does not inherit workspace [patch] entries. While the paired zenpixels release
is unpublished, generate only the current rustdoc in the real workspace. The
baseline still comes from the registry, with its original dependency graph.
No compatibility lint or release-type inference is disabled.
"""

import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import urllib.request

# cargo-semver-checks 0.49 supports this toolchain's rustdoc JSON schema. Keep
# both sides on the same compiler; ordinary CI still tests current stable.
RUSTDOC_TOOLCHAIN = "1.98.1"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("package", choices=["zencodec", "zencodec-testkit"])
    parser.add_argument("baseline")
    parser.add_argument("features", choices=["all", "none", "default"])
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"],
        cwd=root, text=True, timeout=120,
    ))
    package = next(p for p in metadata["packages"] if p["name"] == args.package)
    library = next(t for t in package["targets"] if "lib" in t["kind"])
    current = Path(metadata["target_directory"]) / "doc" / (library["name"] + ".json")
    # A failed rustdoc must never leave a stale successful input for comparison.
    current.unlink(missing_ok=True)
    rustdoc_features = {"all": ["--all-features"], "none": ["--no-default-features"], "default": []}
    env = dict(os.environ, RUSTC_BOOTSTRAP="1")
    rustdoc_args = ["--", "-Z", "unstable-options", "--output-format", "json",
                    "--document-private-items", "--document-hidden-items"]
    subprocess.run(
        ["cargo", f"+{RUSTDOC_TOOLCHAIN}", "rustdoc", "--lib", "-p", args.package,
         *rustdoc_features[args.features], *rustdoc_args],
        cwd=root, env=env, check=True, timeout=1200,
    )
    document = json.loads(current.read_text())
    if document["crate_version"] != package["version"]:
        raise RuntimeError("Current rustdoc version does not match the workspace package")

    # JSON input mode requires JSON for BOTH sides. Fetch the exact published
    # archive, verify its registry checksum, and build it outside any ancestor
    # workspace. Do not patch its dependencies to the unpublished current graph.
    headers = {"User-Agent": "zencodec-paired-semver (https://github.com/imazen/zencodec)"}

    def fetch(url):
        request = urllib.request.Request(url, headers=headers)
        with urllib.request.urlopen(request, timeout=120) as response:
            return response.read()

    version = json.loads(fetch(
        f"https://crates.io/api/v1/crates/{args.package}/{args.baseline}"
    ))["version"]
    archive = fetch(
        f"https://static.crates.io/crates/{args.package}/{args.package}-{args.baseline}.crate"
    )
    if hashlib.sha256(archive).hexdigest() != version["checksum"]:
        raise RuntimeError("Published baseline archive checksum mismatch")
    scratch = Path(os.environ.get("TMPDIR", Path.home() / "tmp"))
    scratch.mkdir(parents=True, exist_ok=True)
    baseline_root = Path(tempfile.mkdtemp(prefix="zencodec-semver-", dir=scratch))
    # Keep source/provenance on failure as well as success; no binaries archived.
    with tarfile.open(fileobj=io.BytesIO(archive)) as source:
        source.extractall(baseline_root, filter="data")
    baseline_source = baseline_root / f"{args.package}-{args.baseline}"
    baseline_target = Path(metadata["target_directory"]) / "paired-semver" / args.package / args.features
    baseline_env = dict(env, CARGO_TARGET_DIR=str(baseline_target))
    subprocess.run(
        ["cargo", f"+{RUSTDOC_TOOLCHAIN}", "rustdoc", "--lib", "-p", args.package,
         *rustdoc_features[args.features], *rustdoc_args],
        cwd=baseline_source, env=baseline_env, check=True, timeout=1200,
    )
    baseline_doc = baseline_target / "doc" / (library["name"] + ".json")
    if json.loads(baseline_doc.read_text())["crate_version"] != args.baseline:
        raise RuntimeError("Baseline rustdoc version does not match the published version")
    # Keep both exact inputs and registry provenance beside the comparison.
    current_copy = baseline_target / "current.json"
    shutil.copyfile(current, current_copy)
    (baseline_target / "sources.json").write_text(json.dumps({
        "package": args.package, "current": package["version"],
        "baseline": args.baseline, "baseline_checksum": version["checksum"],
        "features": args.features, "baseline_source": str(baseline_source),
        "rustdoc_toolchain": RUSTDOC_TOOLCHAIN,
    }, indent=2) + "\n")
    subprocess.run(
        ["cargo", "semver-checks", "check-release", "-p", args.package,
         "--current-rustdoc", str(current_copy), "--baseline-rustdoc", str(baseline_doc)],
        cwd=root, check=True, timeout=1200,
    )


if __name__ == "__main__":
    main()
