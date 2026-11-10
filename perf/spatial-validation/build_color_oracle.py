#!/usr/bin/env python3
"""Build the test-only system or pinned-source Little CMS reader with provenance."""
import argparse
import hashlib
import json
from pathlib import Path
import shlex
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--lcms-source", type=Path,
                        help="compile an independently pinned LCMS C source tree")
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    source = Path(__file__).resolve().with_name("color_oracle.c")
    binary = output / "color-oracle"
    source_hashes = None
    if args.lcms_source:
        tree = args.lcms_source.resolve()
        sources = sorted((tree / "src").glob("*.c"))
        if not sources or not (tree / "include/lcms2.h").is_file():
            raise RuntimeError("LCMS source tree is incomplete")
        source_hashes = {str(p.relative_to(tree)): hashlib.sha256(p.read_bytes()).hexdigest()
                         for p in sorted([*sources, *(tree / "src").glob("*.h"),
                                          *(tree / "include").glob("*.h")])}
        flags = ["-I", str(tree / "include"), *map(str, sources), "-lm", "-pthread"]
        package_version = None
    else:
        flags = shlex.split(subprocess.check_output(
            ["pkg-config", "--cflags", "--libs", "lcms2"], text=True, timeout=10))
        package_version = subprocess.check_output(
            ["pkg-config", "--modversion", "lcms2"], text=True, timeout=5).strip()
    command = ["cc", "-std=c11", "-O2", "-Wall", "-Wextra", "-Werror",
               str(source), *flags, "-o", str(binary)]
    subprocess.run(command, check=True, timeout=30)
    runtime = subprocess.check_output([str(binary), "--version"], text=True, timeout=5).strip()
    receipt = {
        "scope": "Independent test-only CMS; no product dependency or encoded output",
        "command": command,
        "runtime": runtime,
        "pkg_config_version": package_version,
        "lcms_source_hashes": source_hashes,
        "source_sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
        "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "linked_libraries": subprocess.check_output(["ldd", str(binary)], text=True, timeout=5),
    }
    (output / "provenance.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps(receipt))


if __name__ == "__main__":
    main()
