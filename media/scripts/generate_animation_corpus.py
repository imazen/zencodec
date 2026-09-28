#!/usr/bin/env python3
"""Regenerate/check animation bitstreams produced only by the pinned native codecs."""
import argparse
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
DESTINATION = ROOT / "corpus/native-animation"


def generate(directory):
    subprocess.run(["cargo", "run", "--locked", "--release", "-p", "zencodec-media-codec-tests",
                    "--example", "generate_animation_corpus", "--", str(directory)], cwd=ROOT, check=True)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    if not args.check:
        generate(DESTINATION)
        return
    with tempfile.TemporaryDirectory(prefix="animation-corpus-") as temporary:
        fresh = Path(temporary)
        generate(fresh)
        expected = sorted(p.name for p in DESTINATION.iterdir() if p.is_file())
        actual = sorted(p.name for p in fresh.iterdir() if p.is_file())
        if expected != actual:
            raise SystemExit(f"Fixture file sets differ: {expected} != {actual}")
        for name in expected:
            if (DESTINATION/name).read_bytes() != (fresh/name).read_bytes():
                raise SystemExit(f"Fixture changed: {name}")
        print(f"Verified {len(expected)-1} native animation files and manifest")


if __name__ == "__main__":
    main()
