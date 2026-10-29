#!/usr/bin/env python3
"""Build a pinned FFprobe for container timing and older-version comparisons."""
import argparse
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile

from timing_source import fetch_source

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("directory", type=Path)
parser.add_argument("--version", choices=["6.1.1", "8.0.1"], default="8.0.1")
args = parser.parse_args()
root = args.directory.resolve()
root.mkdir(parents=True, exist_ok=True)
archive = fetch_source(root, args.version)
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
