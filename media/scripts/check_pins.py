#!/usr/bin/env python3
"""Reject drifting PR pins and duplicate foundational crate versions."""
import json
from pathlib import Path
import tomllib

root=Path(__file__).resolve().parents[1]
manifest=json.loads((root/"integration/repos.lock.json").read_text())
cargo=tomllib.loads((root/"Cargo.toml").read_text())
fuzz_cargo=tomllib.loads((root/"fuzz/Cargo.toml").read_text())
lock=tomllib.loads((root/"Cargo.lock").read_text())
for repo in manifest["repositories"]:
    for name in repo["packages"]:
        patch=cargo["patch"]["crates-io"].get(name)
        if repo["name"] == "zencodec":
            assert (root/patch["path"]).resolve() == root.parent, name
            assert (root/"fuzz"/fuzz_cargo["patch"]["crates-io"][name]["path"]).resolve() == root.parent, name
        elif not repo.get("direct_git", False):
            assert patch["git"]==repo["url"] and patch["rev"]==repo["revision"], name
            if name in fuzz_cargo["patch"]["crates-io"]:
                assert fuzz_cargo["patch"]["crates-io"][name]==patch, f"{name}: fuzz patch drift"
        packages=[p for p in lock["package"] if p["name"]==name]
        if not packages and name in repo.get("optional_packages", []):
            continue
        assert len(packages)==1, f"{name}: expected exactly one locked version"
        source=packages[0].get("source")
        if source is not None:
            assert source.endswith("#"+repo["revision"]), f"{name}: lockfile source differs from manifest"
        elif repo["name"] != "zencodec":
            config=root/".cargo/config.toml"
            assert config.exists(), f"{name}: path dependency without explicit checkout override"
            local=tomllib.loads(config.read_text())["patch"]["crates-io"][name]
            assert "path" in local, f"{name}: expected local path override"
print(f"Verified {len(manifest['repositories'])} coordinated repository pins")
