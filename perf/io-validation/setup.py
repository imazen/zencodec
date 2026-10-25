#!/usr/bin/env python3
"""Fetch only text sources at recorded commits; never touch sibling checkouts."""
import concurrent.futures
import hashlib
import json
import os
from pathlib import Path
import urllib.parse
import urllib.request

ROOT = Path(__file__).resolve().parent / "vendor"
PINS = [
    ("imazen/zenavif", "a7638f6e0655914fb15fe4633ac26425533756c8", ["zenmp4"]),
    ("imazen/zenextras", "ae920c1e2ac0fe0e6e2fcb709d99934ba36ea49c", ["zenivf", "zenwebm"]),
    ("imazen/zenbitmaps", "11fbd887985e59769d2aeb48295767b7d04209b9", [""]),
]


def get(url):
    headers = {"User-Agent": "zencodec-source-qualification"}
    # CI runners share the anonymous API quota. Raw source requests need no token.
    if urllib.parse.urlsplit(url).netloc == "api.github.com" and os.environ.get("GH_TOKEN"):
        headers["Authorization"] = "Bearer " + os.environ["GH_TOKEN"]
    request = urllib.request.Request(url, headers=headers)
    with urllib.request.urlopen(request, timeout=90) as r:
        return r.read()


def fetch(job):
    repo, revision, source, target = job
    url = f"https://raw.githubusercontent.com/{repo}/{revision}/{source}"
    data = get(url)
    if target.exists() and target.read_bytes() != data:
        raise RuntimeError(f"Refusing to overwrite changed source: {target}")
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(data)
    return {"url": url, "path": str(target.relative_to(ROOT)),
            "sha256": hashlib.sha256(data).hexdigest()}


def main():
    jobs = []
    for repo, rev, packages in PINS:
        tree = json.loads(get(f"https://api.github.com/repos/{repo}/git/trees/{rev}?recursive=1"))
        assert not tree.get("truncated"), repo
        for entry in tree["tree"]:
            path = entry["path"]
            if entry["type"] != "blob":
                continue
            for package in packages:
                prefix = package + "/" if package else ""
                if not path.startswith(prefix):
                    continue
                relative = path[len(prefix):]
                wanted = relative == "Cargo.toml" or relative.startswith(("src/", "tests/", "examples/", "scripts/"))
                if not package:
                    wanted = relative == "Cargo.toml" or relative.startswith("src/")
                if wanted and Path(path).suffix in (".rs", ".toml", ".sh", ".py"):
                    target = ROOT / (package or "zenbitmaps") / relative
                    jobs.append((repo, rev, path, target))
    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
        records = list(pool.map(fetch, jobs))
    (ROOT / "sources.json").write_text(json.dumps(records, indent=2) + "\n")
    print(f"Fetched and hashed {len(records)} text sources at pinned commits")


if __name__ == "__main__":
    main()
