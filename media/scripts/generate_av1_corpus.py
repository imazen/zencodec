#!/usr/bin/env python3
"""Original source pixels -> zenrav1e -> rav1d-safe -> exact source comparison.

No external codec tools or downloaded images are used. Generation requires a
clean committed source tree; all byte/code provenance is retained. --check only
verifies committed artifacts and the independent Python source generator.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parents[1]
DEST = ROOT / "corpus/av1"
MANIFEST = ROOT / "corpus/av1-manifest.json"


def sha(data):
    return hashlib.sha256(data).hexdigest()


def source(case):
    bits, w, h = case["bits"], case["width"], case["height"]
    full = case["range"] == "full"
    sx, sy = {"420": (2, 2), "422": (2, 1), "444": (1, 1), "mono": (1, 1)}[case["sampling"]]
    maximum, k = 2**bits-1, 2**(bits-8)
    output = bytearray()
    for frame in range(case["frames"]):
        count = 1 if case["sampling"] == "mono" else 3
        for plane in range(count):
            pw, ph = (w, h) if plane == 0 else ((w+sx-1)//sx, (h+sy-1)//sy)
            lo, hi = (0, maximum) if full else (16*k, (235 if plane == 0 else 240)*k)
            for y in range(ph):
                for x in range(pw):
                    level = (17*x + 29*y + 43*frame + 71*plane) % 257
                    if x == 0 and y == 0:
                        level = 0 if frame % 2 == 0 else 256
                    value = lo + ((hi-lo)*level + 128)//256
                    output.extend(struct.pack("<H", value) if bits > 8 else bytes([value]))
    return bytes(output)


def output(*args):
    return subprocess.check_output(args, cwd=ROOT, text=True).strip()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    if args.check:
        manifest = json.loads(MANIFEST.read_text())
        assert len(manifest["cases"]) == 48, "corpus must not silently shrink"
        for case in manifest["cases"]:
            assert sha(source(case)) == case["source_sha256"], case["id"]
            assert case["decoded_sha256"] == case["source_sha256"], case["id"]
            assert sha((ROOT/"corpus"/case["path"]).read_bytes()) == case["sha256"], case["id"]
        print(f'Verified {len(manifest["cases"])} AV1 cases and source hashes')
        return
    if output("git", "status", "--porcelain", "--untracked-files=no") or output("git", "ls-files", "--others", "--exclude-standard", "--", "."):
        raise SystemExit("Commit source changes first: corpus provenance requires a clean revision")
    revision = output("git", "rev-parse", "HEAD")
    command = ["cargo", "build", "--locked", "--release", "--all-features", "--example", "generate_av1_corpus"]
    subprocess.run(command, cwd=ROOT, check=True)
    metadata = json.loads(output("cargo", "metadata", "--no-deps", "--format-version", "1"))
    binary = Path(metadata["target_directory"])/"release/examples/generate_av1_corpus"
    generated = ROOT/"corpus/generated"
    generated.mkdir(exist_ok=True)
    stage = Path(tempfile.mkdtemp(prefix="native-av1-", dir=generated))
    print(f"Staging artifacts and preserving earlier data at {stage}", flush=True)
    cases = []
    for bits in (8, 10, 12):
        for sampling in ("420", "422", "444", "mono"):
            for full in (False, True):
                for w, h in ((32, 24), (17, 13)):
                    identifier = f'av1-{bits}-{sampling}-{"full" if full else "narrow"}-{w}x{h}'
                    case = dict(id=identifier, bits=bits, sampling=sampling,
                                range="full" if full else "narrow", width=w, height=h,
                                frames=4, time_base=[1001, 30000], presentation_ticks=[0,1,2,3],
                                color=dict(primaries=1, transfer=1, matrix=1), chroma_position=0)
                    encoded, decoded = stage/(identifier+".ivf"), stage/(identifier+".decoded.yuv")
                    arguments = [str(bits), sampling, case["range"], str(w), str(h), "4"]
                    subprocess.run([str(binary), *arguments, str(encoded), str(decoded)], cwd=ROOT, check=True, timeout=120)
                    raw = source(case)
                    if decoded.read_bytes() != raw:
                        raise RuntimeError(f"{identifier}: native decoded samples differ from independent Python source")
                    case.update(path="av1/"+encoded.name, sha256=sha(encoded.read_bytes()),
                                source_sha256=sha(raw), decoded_sha256=sha(decoded.read_bytes()),
                                generator_arguments=arguments)
                    cases.append(case)
                    print(identifier, flush=True)
    lock = tomllib.loads((ROOT/"Cargo.lock").read_text())
    codecs = {p["name"]:dict(version=p["version"], source=p["source"])
              for p in lock["package"] if p["name"] in ("zenrav1e", "rav1d-safe", "zenpixels")}
    files = ["scripts/generate_av1_corpus.py", "examples/generate_av1_corpus.rs",
             "crates/zencodec-media-testkit/src/synthetic.rs", "src/av1_encode.rs", "src/av1.rs", "Cargo.lock"]
    manifest = dict(schema=2, purpose="validation-only synthetic media fixtures",
                    generator="scripts/generate_av1_corpus.py", build_commit=revision,
                    generator_sources={p:sha((ROOT/p).read_bytes()) for p in files},
                    binary_sha256=sha(binary.read_bytes()), build_command=command,
                    rustc=output("rustc", "-vV"), codec_sources=codecs,
                    encoder=dict(name="zenrav1e", preset=10, quantizer=0, lookahead=1,
                                 low_latency=True, keyframe_interval=2, threads=1),
                    reference_decoder="rav1d-safe strict; independent Python source-byte comparison",
                    cases=cases)
    # Retain previous bytes before replacing generated, owned artifacts.
    if DEST.exists(): shutil.copytree(DEST, stage/"previous-av1")
    if MANIFEST.exists(): shutil.copy2(MANIFEST, stage/"previous-manifest.json")
    DEST.mkdir(exist_ok=True)
    for case in cases: shutil.copy2(stage/Path(case["path"]).name, ROOT/"corpus"/case["path"])
    MANIFEST.write_text(json.dumps(manifest,indent=2)+"\n")
    print(f"Generated and source-verified {len(cases)} native cases; staged files retained")


if __name__ == "__main__":
    main()
