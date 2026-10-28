#!/usr/bin/env python3
"""Build pinned, independent test decoders. Requires Linux, CMake and Ninja.

Invoke through run-heavy locally. CI supplies the same command/job deadlines.
System dependencies: Highway >= 1.0.7, Brotli and lcms2 development packages.
"""
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tarfile
from urllib.request import urlopen


VERSION = "0.11.1"
URL = f"https://codeload.github.com/libjxl/libjxl/tar.gz/refs/tags/v{VERSION}"
SHA256 = "1492dfef8dd6c3036446ac3b340005d92ab92f7d48ee3271b5dac1d36945d3d9"


def main():
    output = Path(sys.argv[1]).resolve()
    output.mkdir(parents=True, exist_ok=False)
    commands = []

    def run(command, seconds=120):
        commands.append(command)
        (output / "commands.json").write_text(json.dumps(commands, indent=2) + "\n")
        subprocess.run(["timeout", "--kill-after=10s", f"{seconds}s", *command], check=True)

    archive = output / f"libjxl-{VERSION}.tar.gz"
    with urlopen(URL, timeout=30) as response:
        data = response.read(32 * 1024 * 1024 + 1)
    if len(data) > 32 * 1024 * 1024 or hashlib.sha256(data).hexdigest() != SHA256:
        raise RuntimeError("libjxl archive does not match the pinned source")
    archive.write_bytes(data)
    with tarfile.open(archive) as tar:
        tar.extractall(output, filter="data")
    source = output / f"libjxl-{VERSION}"
    build = output / "build"
    library = build / "lib"
    disabled = ["TOOLS", "DEVTOOLS", "DOXYGEN", "MANPAGES", "BENCHMARK", "EXAMPLES",
                "JPEGLI", "JNI", "SJPEG", "OPENEXR", "SKCMS", "TCMALLOC", "PLUGINS"]
    run(["cmake", "-S", str(source), "-B", str(build), "-G", "Ninja",
         "-DCMAKE_BUILD_TYPE=Release", "-DBUILD_TESTING=OFF", "-DBUILD_SHARED_LIBS=ON",
         f"-DCMAKE_LIBRARY_OUTPUT_DIRECTORY={library}", "-DJPEGXL_BUNDLE_LIBPNG=OFF",
         *[f"-DJPEGXL_ENABLE_{feature}=OFF" for feature in disabled],
         "-DJPEGXL_FORCE_SYSTEM_HWY=ON", "-DJPEGXL_FORCE_SYSTEM_LCMS2=ON",
         "-DJPEGXL_FORCE_SYSTEM_BROTLI=ON"])
    run(["cmake", "--build", str(build), "--target", "jxl_dec", "--parallel", "2"], 480)
    here = Path(__file__).resolve().parent
    for helper, executable in [("jxl_oracle.c", "jxl-oracle"), ("layer_oracle.c", "layer-oracle")]:
        run(["cc", "-std=c11", "-Wall", "-Wextra", "-Werror",
             "-I", str(build / "lib/include"), "-I", str(source / "lib/include"),
             str(here / helper), "-L", str(library), f"-Wl,-rpath,{library}",
             "-ljxl_dec", "-o", str(output / executable)])
    runtime = subprocess.check_output([str(output / "layer-oracle"), "--version"],
                                      text=True, timeout=5).strip()
    if runtime != "11001":
        raise RuntimeError(f"linked unexpected libjxl: {runtime}")
    receipt = {"source": URL, "source_sha256": SHA256, "runtime_version": int(runtime),
               "scope": "Independent test-only decoders; not a product dependency",
               "helpers": {name: hashlib.sha256((output / name).read_bytes()).hexdigest()
                           for name in ["jxl-oracle", "layer-oracle"]}}
    (output / "provenance.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps(receipt))


if __name__ == "__main__":
    main()
