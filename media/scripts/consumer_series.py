"""Configure downstream consumers against the same ordinary codec checkouts."""
import json
from pathlib import Path
import subprocess
import tomllib


def package_version(path, manifest):
    version = manifest["package"]["version"]
    if isinstance(version, str):
        return version
    if version != {"workspace": True}:
        raise RuntimeError(f"Unsupported package version in {path}")
    for directory in [path, *path.parents]:
        candidate = directory / "Cargo.toml"
        if candidate.exists():
            workspace = tomllib.loads(candidate.read_text()).get("workspace", {})
            if "version" in workspace.get("package", {}):
                return workspace["package"]["version"]
    raise RuntimeError(f"No inherited package version for {path}")


def configure(root, destination, core_repositories, clone, run):
    manifest = json.loads((root / "integration/consumers.lock.json").read_text())
    repositories = list(core_repositories)
    for repo in manifest["repositories"]:
        clone(repo["url"], repo["revision"], destination / repo["name"])
        repositories.append(repo)

    consumer = destination / "zenpipe"
    lock_path = consumer / "Cargo.lock"
    before = tomllib.loads(lock_path.read_text())
    names = {p["name"] for p in before["package"]}
    locked_versions = {(p["name"], p["version"]) for p in before["package"]}
    registry = []
    git = {}
    for repo in repositories:
        # The consumer's direct parser dependency must use the same parser
        # instance as the path-backed AVIF decoder workspace.
        packages = dict(repo["packages"])
        if repo["name"] == "zenavif":
            packages["zenavif-parse"] = "zenavif-parse"
        for name, relative in packages.items():
            if name not in names:
                continue
            path = (destination / repo["name"] / relative).resolve()
            package = tomllib.loads((path / "Cargo.toml").read_text())
            if package.get("package", {}).get("name") != name:
                raise RuntimeError(f"{path} is not package {name}")
            version = package_version(path, package)
            if (name, version) not in locked_versions:
                print(f"Keeping consumer's locked {name}; checkout version {version} is different")
                continue
            line = f"{json.dumps(name)} = {{ path = {json.dumps(str(path))} }}"
            registry.append(line)
            git.setdefault(repo["url"], []).append(line)
    lines = ["[patch.crates-io]", *registry]
    for url, entries in git.items():
        lines.extend([f"[patch.{json.dumps(url)}]", *entries])
    config = consumer / ".cargo/config.toml"
    config.parent.mkdir(exist_ok=True)
    original = config.read_text() if config.exists() else ""
    previous = tomllib.loads(original)
    if "patch" in previous or "paths" in previous:
        raise RuntimeError("Refusing to replace existing consumer source overrides")
    config.write_text(original.rstrip() + "\n\n" + "\n".join(lines) + "\n")
    run("cargo", "metadata", "--format-version", "1", "--all-features", cwd=consumer)
    after = tomllib.loads(lock_path.read_text())
    versions = lambda lock: sorted((p["name"], p["version"]) for p in lock["package"])
    if versions(before) != versions(after):
        removed = set(versions(before)) - set(versions(after))
        added = set(versions(after)) - set(versions(before))
        raise RuntimeError(f"Consumer overrides changed package versions: -{removed} +{added}")
    # Also check the default feature mode under --locked. Cargo sometimes
    # reorders duplicated unused patches when the feature set changes.
    run("cargo", "metadata", "--format-version", "1", "--locked", cwd=consumer)
    return consumer


def check(consumer):
    # External/local corpus features stay in their owning workstation gates.
    # These committed tests exercise actual decoded pixels and metadata.
    subprocess.run([
        "cargo", "test", "--locked", "--release", "-p", "zenpipe",
        "--test", "wide_gamut",
    ], cwd=consumer, check=True)
    subprocess.run([
        "cargo", "test", "--locked", "--release", "-p", "zencodecs",
        "--features", "all,jp2-decode",
        "--test", "jp2_decode", "--test", "transcode_color",
        "--test", "metadata_conformance", "--test", "stop_and_limits",
    ], cwd=consumer, check=True)
    subprocess.run([
        "cargo", "check", "--locked", "--workspace", "--all-features",
    ], cwd=consumer, check=True)
