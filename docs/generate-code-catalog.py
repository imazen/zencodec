#!/usr/bin/env python3
"""Generate compiler-derived API documentation, outside workspace path patches.

Run through run-heavy. cargo-public-api must already be installed. No dependency
sources are copied; only this repository's exported declarations are retained.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

PACKAGES = [
    ("zencodec", "."),
    ("zencodec-io", "zencodec-io"),
    ("zencodec-coroutine", "zencodec-coroutine"),
    ("zencodec-media", "zencodec-media"),
    ("zencodec-testkit", "zencodec-testkit"),
    ("zencodec-media-testkit", "zencodec-media-testkit"),
    ("zenmeta", "zenmeta"),
    ("zencodec-render", "zencodec-render"),
]


def source_inputs(root, directory):
    base = root / directory
    files = [base / "Cargo.toml", *sorted((base / "src").rglob("*.rs"), key=lambda p: p.as_posix())]
    # Crate docs can include the README (zenmeta does). Treat each crate's
    # README as a documentation input even when it is not embedded by rustdoc.
    if (base / "README.md").is_file():
        files.append(base / "README.md")
    return files


def fingerprint(root, directory):
    digest = hashlib.sha256()
    for file in source_inputs(root, directory):
        digest.update(file.relative_to(root).as_posix().encode() + b"\0")
        digest.update(file.read_bytes() + b"\0")
    return digest.hexdigest()


def source_details(rustdoc, root, directory):
    data = json.loads(rustdoc.read_text())
    index = data["index"]
    paths = {key: "::".join(value["path"]) for key, value in data["paths"].items()
             if value["crate_id"] == 0}
    # Methods are indexed by their defining owner, not necessarily present in
    # rustdoc's exported path map. Preserve all names for re-export lookups.
    for key, item in index.items():
        if key not in paths:
            continue
        inner = item["inner"]
        body = next(iter(inner.values()))
        if not isinstance(body, dict):
            continue
        fields = body.get("fields", [])
        kind = body.get("kind")
        if isinstance(kind, dict):
            fields += kind.get("plain", {}).get("fields", [])
        for member in body.get("items", []) + fields + body.get("variants", []):
            child = index.get(str(member))
            if child and child["name"]:
                paths.setdefault(str(member), paths[key] + "::" + child["name"])
        for implementation in body.get("impls", []):
            impl = index.get(str(implementation), {}).get("inner", {}).get("impl", {})
            for member in impl.get("items", []):
                child = index.get(str(member))
                if child and child["crate_id"] == 0 and child["name"]:
                    paths.setdefault(str(member), paths[key] + "::" + child["name"])
    result = []
    for key, name in paths.items():
        item = index.get(key)
        if not item or not item.get("span"):
            continue
        span = item["span"]
        filename = Path(span["filename"])
        if filename.is_absolute():
            filename = filename.relative_to(root)
        elif directory == "zencodec-render":
            filename = Path(directory) / filename
        source = root / filename
        if not source.is_file():
            raise RuntimeError(f"Missing source for {name}: {source}")
        result.append({"path": name, "file": filename.as_posix(),
                       "line": span["begin"][0], "end": span["end"][0],
                       "docs": item.get("docs") or "",
                       "deprecated": item.get("deprecation")})
    return sorted(result, key=lambda item: item["path"])


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("root", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--source-root", type=Path, help="Verify exported inputs against this checkout")
    args = parser.parse_args()
    root = args.root.resolve()
    source_root = (args.source_root or root).resolve()
    output = (args.output or root / "docs/code-api").resolve()
    output.mkdir(parents=True, exist_ok=True)
    manifest = {"generator": "cargo public-api --all-features -sss -v", "packages": []}
    for package, directory in PACKAGES:
        for source in source_inputs(source_root, directory):
            built = root / source.relative_to(source_root)
            if source.read_bytes() != built.read_bytes():
                raise RuntimeError(f"Export differs from source: {source}")
        print(f"Catalog: {package}", flush=True)
        cargo_manifest = root / directory / "Cargo.toml"
        result = subprocess.run(
            ["cargo", "public-api", "--manifest-path", str(cargo_manifest),
             "-p", package, "--all-features", "-sss", "-v", "--color", "never"],
            cwd=root, check=True, capture_output=True, text=True, timeout=180,
        )
        print(result.stderr, end="", flush=True)
        surface = result.stdout
        if not surface.startswith("pub mod ") or "pub fn " not in surface:
            raise RuntimeError(f"Invalid API output for {package}")
        (output / f"{package}.txt").write_text(surface)
        target = root / ("zencodec-render/target" if package == "zencodec-render" else "target")
        details = source_details(target / "doc" / (package.replace("-", "_") + ".json"), root, directory)
        details_text = json.dumps(details, ensure_ascii=False) + "\n"
        (output / f"{package}.json").write_text(details_text)
        manifest["packages"].append({
            "name": package, "directory": directory, "features": "all",
            "fingerprint": fingerprint(source_root, directory),
            "surface_sha256": hashlib.sha256(surface.encode()).hexdigest(),
            "details_sha256": hashlib.sha256(details_text.encode()).hexdigest(),
        })
    (output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")


if __name__ == "__main__":
    main()
