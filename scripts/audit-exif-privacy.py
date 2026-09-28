#!/usr/bin/env python3
"""Read-only corpus EXIF audit against ExifTool. Writes only a fresh /tmp directory.

Selects manifest-listed images, prioritizing metadata-related repros. Does not
sanitize containers or assess embedded ICC profiles, auxiliary images or pixels.
No actual metadata values are printed or copied into the repository.
"""
import argparse
import base64
import collections
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile


def exiftool(paths, flags):
    # Argument files avoid ARG_MAX and shell interpretation of corpus names.
    data = "\n".join(map(str, paths)) + "\n"
    result = subprocess.run(
        ["exiftool", "-j", *flags, "-@", "-"], input=data,
        text=True, capture_output=True, timeout=180,
    )
    if result.returncode not in (0, 1):
        raise RuntimeError(f"ExifTool failed: exit {result.returncode}")
    return json.loads(result.stdout)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("--limit", type=int, default=3000)
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    scratch = Path(tempfile.mkdtemp(prefix="zencodec-exif-privacy-"))
    source = scratch / "source"
    source.mkdir()
    output = scratch / "filtered"
    formats = {"jpeg", "png", "webp", "tiff", "bigtiff", "heic", "heif", "avif", "jxl"}
    candidates = {}
    for line in args.manifest.open():
        row = json.loads(line)
        if row.get("format") not in formats or row.get("file_size", 0) > 32 * 1024 * 1024:
            continue
        path = Path(row["path"])
        if "\n" in str(path) or "\r" in str(path) or not path.is_file():
            continue
        title = row.get("issue", {}).get("title", "").lower()
        priority = not any(s in title for s in ("exif", "metadata", "privacy", "serial", "orientation", "icc", "color profile"))
        candidates.setdefault(row["sha256"], (priority, row))
    selected = [item[1] for _, item in sorted(candidates.items(), key=lambda pair: (pair[1][0], pair[0]))[:args.limit]]
    records = []
    counts = collections.Counter()
    seen = set()
    for start in range(0, len(selected), 64):
        batch = selected[start:start + 64]
        rows = exiftool([r["path"] for r in batch], ["-b", "-EXIF", "-FileType", "-Warning", "-Error"])
        for row in rows:
            counts["scanned"] += 1
            counts["warning_files"] += int("Warning" in row)
            counts["error_files"] += int("Error" in row)
            encoded = row.get("EXIF", "")
            if encoded.startswith("base64:"):
                counts["exif_files"] += 1
                blob = base64.b64decode(encoded[7:])
            elif row.get("FileType") in {"TIFF", "DNG", "BTF"}:
                # TIFF is itself the EXIF tree: -EXIF does not extract it.
                # Bound scratch copies; raw pixels are never interpreted here.
                path = Path(row["SourceFile"])
                if path.stat().st_size > 1024 * 1024:
                    counts["large_tiff_skipped"] += 1
                    continue
                counts["raw_tiff_inputs"] += 1
                blob = path.read_bytes()
            else:
                continue
            if len(blob) > 16 * 1024 * 1024:
                counts["oversize_exif_skipped"] += 1
                continue
            digest = hashlib.sha256(blob).hexdigest()
            if digest in seen:
                continue
            seen.add(digest)
            (source / f"{digest}.exif").write_bytes(blob)
            records.append({"exif_sha256": digest, "path": row["SourceFile"], "format": row.get("FileType")})
        print(f"extracted {min(start + 64, len(selected))}/{len(selected)} files; {len(seen)} unique EXIF blobs", flush=True)
    (scratch / "sources.json").write_text(json.dumps(records, indent=2))
    subprocess.run(["cargo", "run", "--quiet", "--example", "audit_exif_privacy", "--", str(source), str(output)], cwd=root, check=True)
    # Independent reader: reject every output field outside the documented
    # publication allowlist, including unknown tags and any nested metadata.
    allowed = {
        "IFD0:Orientation", "IFD0:Artist", "IFD0:Copyright",
        "ExifIFD:Photographer", "ExifIFD:ImageEditor", "ExifIFD:ColorSpace", "ExifIFD:Gamma",
        "InteropIFD:InteropIndex", "InteropIFD:InteropVersion",
    }
    files = sorted(output.iterdir())
    failures = []
    for start in range(0, len(files), 64):
        for row in exiftool(files[start:start + 64], ["-G1", "-s", "-n", "-u", "-EXIF:all", "-XMP:all", "-IPTC:all", "-MakerNotes:all", "-Warning", "-Error"]):
            counts["oracle_outputs"] += 1
            for key in row:
                if key == "SourceFile":
                    continue
                if key == "ExifTool:Warning":
                    counts["output_warnings"] += 1
                    continue
                if key not in allowed or (row["SourceFile"].endswith("-1.exif") and key in {"IFD0:Artist", "IFD0:Copyright", "ExifIFD:Photographer", "ExifIFD:ImageEditor"}):
                    failures.append({"file": Path(row["SourceFile"]).name, "unexpected_tag": key})
    counts["unique_exif"] = len(seen)
    counts["failures"] = len(failures)
    report = {"exiftool_version": subprocess.check_output(["exiftool", "-ver"], text=True).strip(),
              "counts": dict(counts), "formats": dict(collections.Counter(r["format"] for r in records)), "failures": failures}
    (scratch / "report.json").write_text(json.dumps(report, indent=2))
    print(json.dumps(report, indent=2))
    print(f"Audit artifacts: {scratch}")
    if failures or not seen or counts["oracle_outputs"] == 0:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
