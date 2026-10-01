#!/usr/bin/env python3
"""Compare a cargo-read release artifact with a pinned git revision.

Writes the complete src/ and original-manifest diff outside the repository.
Fails if the release's source differs from its declared VCS commit.
"""

import argparse
import difflib
import hashlib
import json
from pathlib import Path
import subprocess


def git(*args):
    return subprocess.check_output(["git", *args])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("published_dir", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--revision", default="origin/main")
    args = parser.parse_args()
    revision = git("rev-parse", args.revision).decode().strip()
    release = json.loads((args.published_dir / ".cargo_vcs_info.json").read_text())
    baseline = release["git"]["sha1"]
    if release.get("path_in_vcs", ""):
        parser.error("this root-package audit does not support a VCS subdirectory")

    def source_paths(rev):
        return set(git("ls-tree", "-r", "--name-only", rev, "src").decode().splitlines())

    published = {
        p.relative_to(args.published_dir).as_posix(): p.read_bytes()
        for p in (args.published_dir / "src").rglob("*")
        if p.is_file()
    }
    if set(published) != source_paths(baseline):
        raise RuntimeError("published source file set differs from VCS baseline")
    for path, data in published.items():
        if data != git("show", f"{baseline}:{path}"):
            raise RuntimeError(f"published source differs from VCS baseline: {path}")
    manifest = (args.published_dir / "Cargo.toml.orig").read_bytes()
    if manifest != git("show", f"{baseline}:Cargo.toml"):
        raise RuntimeError("original published manifest differs from VCS baseline")
    published["Cargo.toml"] = manifest
    current_paths = source_paths(revision) | {"Cargo.toml"}
    diffs = []
    changed = []
    for path in sorted(set(published) | current_paths):
        old = published.get(path, b"")
        new = git("show", f"{revision}:{path}") if path in current_paths else b""
        if old == new:
            continue
        changed.append(path)
        diffs.extend(difflib.unified_diff(
            old.decode().splitlines(keepends=True),
            new.decode().splitlines(keepends=True),
            fromfile=f"published/{path}", tofile=f"{revision}/{path}",
        ))
    patch = "".join(diffs).encode()
    args.output.mkdir(parents=True, exist_ok=True)
    patch_path = args.output / "source.diff"
    # Never overwrite a different audit artifact.
    if patch_path.exists() and patch_path.read_bytes() != patch:
        raise RuntimeError("output already contains a different diff; choose a new directory")
    patch_path.write_bytes(patch)
    summary = {
        "baseline_commit": baseline,
        "main_commit": revision,
        "published_src_matches_vcs": True,
        "published_original_manifest_matches_vcs": True,
        "changed_paths": changed,
        "diff_sha256": hashlib.sha256(patch).hexdigest(),
        "diff_bytes": len(patch),
    }
    (args.output / "provenance.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
