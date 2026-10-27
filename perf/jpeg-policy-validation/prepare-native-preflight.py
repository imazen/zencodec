#!/usr/bin/env python3
"""Apply the pinned native planning experiment to a new, isolated source export."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tarfile
import tomllib

REVISION = "4535ec499d88b04f58882df03e61761526c35756"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("export", type=Path)
    parser.add_argument("native_repository", type=Path)
    parser.add_argument("native_directory", type=Path)
    args = parser.parse_args()
    export = args.export.resolve()
    native = args.native_directory.resolve()
    if (export / ".git").exists() or (export / ".jj").exists():
        raise ValueError("Use a standalone export, never a working checkout")
    harness = export / "perf/jpeg-policy-validation"
    manifest = harness / "Cargo.toml"
    text = manifest.read_text()
    original = f'git = "https://github.com/imazen/zenjpeg", rev = "{REVISION}"'
    if text.count(original) != 2:
        raise ValueError("Expected exactly the pinned dependency and registry patch")
    root = Path(__file__).resolve().parents[2]
    for directory in ["src", "zencodec-io/src"]:
        for source in (root / directory).rglob("*.rs"):
            if source.read_bytes() != (export / source.relative_to(root)).read_bytes():
                raise ValueError(f"Stale framework source: {source.relative_to(root)}")
    patch = Path(__file__).with_name("native-preflight.patch").resolve()
    native.mkdir(parents=True, exist_ok=False)
    archive = native / "pinned-source.tar"
    with archive.open("wb") as stream:
        subprocess.run(["git", "-C", str(args.native_repository.resolve()), "archive", REVISION],
                       stdout=stream, check=True, timeout=60)
    with tarfile.open(archive) as bundle:
        bundle.extractall(native, filter="data")
    for action in [["--check"], []]:
        subprocess.run(["git", "apply", *action, str(patch)], cwd=native,
                       check=True, timeout=30)
    replacement = "path = " + json.dumps(str(native / "zenjpeg"))
    text = text.replace(original, replacement)
    text += '\n[patch."https://github.com/imazen/zenjpeg"]\nzenjpeg = { ' + replacement + ' }\n'
    manifest.write_text(text)
    lock = harness / "Cargo.lock"
    if lock.exists():
        preserved = harness / "Cargo.lock.before-native-preflight"
        if preserved.exists():
            raise ValueError("Existing saved lock; use a fresh experiment export")
        preserved.write_bytes(lock.read_bytes())
    # Resolve only the patched graph before the locked build. Keep the complete
    # graph for attribution; do not update the committed registry lock.
    with (native / "resolved-graph.json").open("wb") as stream:
        subprocess.run(["cargo", "metadata", "--format-version", "1",
                        "--manifest-path", str(manifest)], stdout=stream,
                       check=True, timeout=120)
    if lock.exists() and (harness / "Cargo.lock.before-native-preflight").exists():
        def unchanged_packages(path):
            packages = tomllib.loads(path.read_text())["package"]
            return sorted((p["name"], p["version"], p.get("source", ""), p.get("checksum", ""))
                          for p in packages if p["name"] not in {"zenjpeg", "zenyuv"})
        if unchanged_packages(lock) != unchanged_packages(harness / "Cargo.lock.before-native-preflight"):
            raise ValueError("Native planning setup changed unrelated locked packages")
    (native / "experiment.json").write_text(json.dumps({
        "native_revision": REVISION,
        "native_patch_sha256": hashlib.sha256(patch.read_bytes()).hexdigest(),
        "export": str(export),
        "scope": "effective JPEG row-output timing; target-zq and MPF not qualified",
        "dependency_note": "disabled sibling zenpredict path replaced by its pinned git source",
    }, indent=2) + "\n")
    print(native)


if __name__ == "__main__":
    main()
