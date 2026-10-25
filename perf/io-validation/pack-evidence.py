#!/usr/bin/env python3
"""Manifest, archive and independently verify a collected experiment directory.

Inputs must be a deliberately selected evidence tree. This does not select or
upload files, and refuses to overwrite an existing manifest/archive.
"""
import argparse
import hashlib
import json
from pathlib import Path
import tarfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("evidence", type=Path)
parser.add_argument("archive", type=Path)
args = parser.parse_args()
root = args.evidence.resolve(strict=True)
archive = args.archive.resolve()
assert root.is_dir()
assert not archive.is_relative_to(root), "archive must be outside its input tree"
assert not archive.exists(), "preserve existing artifacts"
manifest = root / "MANIFEST.json"
assert not manifest.exists(), "preserve existing manifest"

def digest(path):
    with path.open("rb") as f:
        return hashlib.file_digest(f, "sha256").hexdigest()

records = []
for path in sorted(root.rglob("*")):
    assert not path.is_symlink(), f"symlink not admitted: {path}"
    if path.is_file():
        records.append({"path": str(path.relative_to(root)),
                        "bytes": path.stat().st_size, "sha256": digest(path)})
assert records, "empty evidence tree"
with manifest.open("x") as f:
    json.dump({"files": records}, f, indent=2)
    f.write("\n")
expected = {r["path"]: r["sha256"] for r in records}
expected[manifest.name] = digest(manifest)
with archive.open("xb") as raw:
    with tarfile.open(fileobj=raw, mode="w:gz", compresslevel=1) as tar:
        for relative in sorted(expected):
            tar.add(root / relative, arcname=relative, recursive=False)

# Re-read the resulting archive; verify every decompressed file without
# extracting it or relying on tar headers alone.
observed = set()
with tarfile.open(archive, "r:gz") as tar:
    for member in tar:
        assert member.isfile() and member.name in expected
        assert member.name not in observed
        with tar.extractfile(member) as f:
            assert hashlib.file_digest(f, "sha256").hexdigest() == expected[member.name]
        observed.add(member.name)
assert observed == set(expected)
checksum = digest(archive)
with Path(str(archive) + ".sha256").open("x") as f:
    f.write(f"{checksum}  {archive.name}\n")
print(json.dumps({"archive": str(archive), "bytes": archive.stat().st_size,
                  "sha256": checksum, "files": len(records),
                  "payload_bytes": sum(r["bytes"] for r in records)}))
