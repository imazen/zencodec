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
    ("imazen/zenavif", "94116d266299f347cfd10a75d440a5bc3b598d2c", ["zenmp4"]),
    ("imazen/zenextras", "403d46b9973a58b212def0602f4dd8cdee9560dc", ["zenivf", "zenwebm"]),
    ("imazen/zenbitmaps", "b446f830077a7153fe741bca3c9ad03da54c89ef", [""]),
]


# These frozen adapters intentionally carry only the legacy scalar edit offset.
# Keep their runtime code unchanged while the full timeline migration is reviewed.
# Each allowance is function-scoped, recorded in sources.json, and count-checked.
TIMELINE_COMPATIBILITY = {
    "zenmp4/src/demux.rs": ["fn parse_track("],
    "zenmp4/src/mux.rs": [
        "    fn trak(&self, i: usize, t: &MuxTrack, tables: bool) -> Res<Vec<u8>> {",
        "    fn begin(&mut self, tracks: &[TrackInfo], hint: MuxHint) -> Res<MuxTiming> {",
    ],
    "zenwebm/src/mux.rs": [
        "    fn track_entry(&mut self, i: usize, t: &TrackInfo) -> Res<(Vec<u8>, bool)> {",
    ],
}


def timeline_compatibility(relative, data):
    source = data.decode()
    for signature in TIMELINE_COMPATIBILITY.get(relative, []):
        if source.count(signature) != 1:
            raise RuntimeError(f"Frozen timeline compatibility site changed: {relative}: {signature}")
        indent = signature[:len(signature) - len(signature.lstrip())]
        attribute = indent + '#[allow(deprecated, reason = "Frozen scalar timeline adapter; full edit programs remain unqualified (issue187)")]\n'
        source = source.replace(signature, attribute + signature)
    return source.encode()


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
    upstream = get(url)
    data = timeline_compatibility(str(target.relative_to(ROOT)), upstream)
    if target.exists() and target.read_bytes() != data:
        raise RuntimeError(f"Refusing to overwrite changed source: {target}")
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(data)
    return {"url": url, "path": str(target.relative_to(ROOT)),
            "sha256": hashlib.sha256(data).hexdigest(),
            "upstream_sha256": hashlib.sha256(upstream).hexdigest(),
            "timeline_compatibility": str(target.relative_to(ROOT)) in TIMELINE_COMPATIBILITY}


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
