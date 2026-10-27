#!/usr/bin/env python3
"""Build a pinned FFprobe for container timing and older-version comparisons."""
import argparse
import hashlib
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("directory", type=Path)
parser.add_argument("--version", choices=["6.1.1", "8.0.1"], default="8.0.1")
args = parser.parse_args()
root = args.directory.resolve()
root.mkdir(parents=True, exist_ok=True)
archive = root / f"ffmpeg-{args.version}.tar.xz"
if not archive.exists():
    # A timed-out transfer must not poison the archive cache. Keep failed
    # transfers as evidence, and retry transient network failures within a cap.
    with tempfile.NamedTemporaryFile(prefix=archive.name + ".partial-", dir=root,
                                     delete=False) as partial:
        download = Path(partial.name)
    print(f"downloading pinned source to {download}", flush=True)
    subprocess.run([
        "curl", "--fail", "--location", "--connect-timeout", "15",
        "--max-time", "60", "--retry", "2", "--retry-max-time", "180",
        f"https://ffmpeg.org/releases/{archive.name}", "-o", str(download),
    ], check=True, timeout=245)
    download.rename(archive)
digest = hashlib.sha256(archive.read_bytes()).hexdigest()
expected = {
    "6.1.1": "8684f4b00f94b85461884c3719382f1261f0d9eb3d59640a1f4ac0873616f968",
    "8.0.1": "05ee0b03119b45c0bdb4df654b96802e909e0a752f72e4fe3794f487229e5a41",
}[args.version]
if digest != expected:
    raise ValueError(f"source checksum mismatch: {digest}, expected {expected}")
(root / "source.sha256").write_text(f"{digest}  {archive.name}\n")
source = root / f"ffmpeg-{args.version}"
if source.exists() and not (source / "configure").is_file():
    # Rust's target cache can retain directory/object entries while omitting
    # this external source tree. Directory existence is not extraction proof.
    # Preserve partial contents and restore from the checksum-verified archive.
    preserved = Path(tempfile.mkdtemp(prefix=f"incomplete-{args.version}-", dir=root))
    source.rename(preserved / source.name)
    print(f"preserved incomplete source in {preserved}", flush=True)
if not source.exists():
    with tarfile.open(archive) as bundle:
        bundle.extractall(root, filter="data")
subprocess.run([
    "./configure", "--disable-everything", "--disable-autodetect",
    "--disable-doc", "--disable-network", "--disable-x86asm",
    "--disable-ffmpeg", "--disable-ffplay", "--enable-ffprobe",
    "--enable-protocol=file", "--enable-demuxer=matroska,mov,ivf",
    "--enable-parser=av1", "--extra-cflags=-O1",
], cwd=source, check=True, timeout=90)
subprocess.run([
    "make", "-j", os.environ.get("CARGO_BUILD_JOBS", "2"), "ffprobe",
], cwd=source, check=True, timeout=300)
print(source / "ffprobe", flush=True)
