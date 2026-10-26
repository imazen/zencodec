#!/usr/bin/env python3
"""Build the test-only system Little CMS reader and retain its exact identity."""
import hashlib
import json
from pathlib import Path
import shlex
import subprocess
import sys


def main():
    output = Path(sys.argv[1]).resolve()
    output.mkdir(parents=True, exist_ok=False)
    source = Path(__file__).resolve().with_name("color_oracle.c")
    binary = output / "color-oracle"
    flags = shlex.split(subprocess.check_output(
        ["pkg-config", "--cflags", "--libs", "lcms2"], text=True, timeout=10))
    command = ["cc", "-std=c11", "-O2", "-Wall", "-Wextra", "-Werror",
               str(source), *flags, "-o", str(binary)]
    subprocess.run(command, check=True, timeout=30)
    runtime = subprocess.check_output([str(binary), "--version"], text=True, timeout=5).strip()
    receipt = {
        "scope": "Independent test-only CMS; no product dependency or encoded output",
        "command": command,
        "runtime": runtime,
        "pkg_config_version": subprocess.check_output(
            ["pkg-config", "--modversion", "lcms2"], text=True, timeout=5).strip(),
        "source_sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
        "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "linked_libraries": subprocess.check_output(["ldd", str(binary)], text=True, timeout=5),
    }
    (output / "provenance.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps(receipt))


if __name__ == "__main__":
    main()
