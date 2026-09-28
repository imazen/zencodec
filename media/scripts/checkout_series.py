#!/usr/bin/env python3
"""Create ordinary pinned clones without touching existing working directories.

The media checkout is cloned at the commit containing this script. Dependency
pins come from integration/repos.lock.json. A generated Cargo config overrides
git pins with the checked-out package paths; its lockfile is regenerated locally.
No worktrees, reset, stash, force checkout, merge or publishing is performed.
"""
import argparse
import json
from pathlib import Path
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parents[1]


def run(*args, cwd=None):
    return subprocess.check_output(args, cwd=cwd, text=True).strip()


def clone(url, revision, destination):
    # git init + fetching exactly the revision works for draft-PR commits too.
    destination.mkdir()
    run("git", "init", "-q", str(destination))
    run("git", "remote", "add", "origin", url, cwd=destination)
    run("git", "fetch", "--depth", "1", "origin", revision, cwd=destination)
    run("git", "checkout", "--detach", "FETCH_HEAD", cwd=destination)
    actual = run("git", "rev-parse", "HEAD", cwd=destination)
    if actual != revision:
        raise RuntimeError(f"{destination}: fetched revision {actual} != {revision}")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("directory", type=Path, help="new, nonexistent destination")
    parser.add_argument("--check", action="store_true", help="run the integration checks after cloning")
    args = parser.parse_args()
    if run("git", "status", "--porcelain", "--untracked-files=no", cwd=ROOT):
        raise SystemExit("Commit the media checkout first; the runner tests its exact committed revision")
    destination = args.directory.resolve()
    destination.mkdir()  # Refuse an existing directory instead of guessing ownership.
    revision = run("git", "rev-parse", "HEAD", cwd=ROOT)
    # Local ordinary clone is independent and includes an unpushed review commit.
    repository = destination / "zencodec"
    clone(str(ROOT.parent), revision, repository)
    media = repository / "media"
    manifest = json.loads((ROOT / "integration/repos.lock.json").read_text())
    patches = ["[patch.crates-io]"]
    git_patches = {}
    for repo in manifest["repositories"]:
        if repo["name"] == "zencodec":
            path = repository
        else:
            path = destination / repo["name"]
            clone(repo["url"], repo["revision"], path)
        for package, relative in repo["packages"].items():
            package_manifest = tomllib.loads((path/relative/"Cargo.toml").read_text())
            if package_manifest.get("package", {}).get("name") != package:
                raise SystemExit(f"{repo['name']}: {relative} is not package {package}")
            # JSON quoting is also valid TOML basic-string quoting here; this
            # string goes to a file, never through shell interpolation.
            patches.append(f"{json.dumps(package)} = {{ path = {json.dumps(str((path/relative).resolve()))} }}")
        # Crates reached through explicit git dependencies need an override
        # as well as registry patches. Group by URL: serializer and decoder
        # packages can intentionally live at different revisions of one repo.
        entries = git_patches.setdefault(repo["url"], [])
        for package, relative in repo["packages"].items():
            entries.append(f"{json.dumps(package)} = {{ path = {json.dumps(str((path/relative).resolve()))} }}")
    for url, entries in git_patches.items():
        patches.append(f"[patch.{json.dumps(url)}]")
        patches.extend(entries)
    config = media / ".cargo/config.toml"
    config.parent.mkdir(exist_ok=True)
    if config.exists():
        raise SystemExit("Refusing to overwrite Cargo config")
    config.write_text("\n".join(patches)+"\n")
    # Resolve the changed source identities while retaining locked dependency
    # versions. Do not regenerate the lock from scratch against today's registry.
    before = tomllib.loads((media / "Cargo.lock").read_text())
    run("cargo", "metadata", "--format-version", "1", "--all-features", cwd=media)
    after = tomllib.loads((media / "Cargo.lock").read_text())
    versions = lambda lock: sorted((p["name"],p["version"]) for p in lock["package"])
    if versions(before) != versions(after):
        raise SystemExit("Local path overrides unexpectedly changed locked package versions")
    if args.check:
        subprocess.run(["bash", "scripts/check.sh"], cwd=media, check=True)
    print(f"Pinned media revision {revision} at {media}")


if __name__ == "__main__":
    main()
