#!/usr/bin/env python3
"""Cold consumer builds, externally interleaved and observed by zenbench pair."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tarfile


def run(args, **kwargs):
    return subprocess.run(args, check=True, timeout=120, **kwargs)


def prepare(root, main, ideal):
    root.mkdir(parents=True, exist_ok=False)
    script = Path(__file__).resolve()
    main = run(["git", "rev-parse", main], capture_output=True, text=True).stdout.strip()
    ideal = run(["git", "rev-parse", ideal], capture_output=True, text=True).stdout.strip()
    revisions = {"published": "crates.io 0.1.26", "main": main, "ideal": ideal}
    for name, revision in revisions.items():
        if name != "published":
            archive = root / f"{name}.tar"
            with archive.open("wb") as output:
                run(["git", "archive", revision], stdout=output)
            source = root / f"{name}-source"
            source.mkdir()
            with tarfile.open(archive) as bundle:
                bundle.extractall(source, filter="data")
        for feature in ["default", "std"]:
            project = root / name / feature
            (project / "src").mkdir(parents=True)
            (project / "src/lib.rs").write_text("pub use zencodec;\n")
            dep = 'version = "=0.1.26"' if name == "published" else f'path = {json.dumps(str(root / f"{name}-source"))}'
            if feature == "std":
                dep += ', features = ["std"]'
            (project / "Cargo.toml").write_text(
                '[package]\nname = "compile-consumer"\nversion = "0.0.0"\nedition = "2024"\n'
                '[workspace]\n[dependencies]\nzencodec = { ' + dep + ' }\n')
            run(["cargo", "generate-lockfile", "--manifest-path", str(project / "Cargo.toml")])
            run(["cargo", "fetch", "--locked", "--manifest-path", str(project / "Cargo.toml")])
            with (project / "tree.txt").open("w") as output:
                run(["cargo", "tree", "--locked", "--manifest-path", str(project / "Cargo.toml")], stdout=output)
    cases = []
    for profile, feature in [("dev", "default"), ("dev", "std"), ("release", "default")]:
        config = root / f"{profile}-{feature}.json"
        config.write_text(json.dumps({"profile": profile, "feature": feature}) + "\n")
        inputs = {"configuration": str(config), "runner": str(script)}
        for name in revisions:
            inputs[f"{name}_lock"] = str(root / name / feature / "Cargo.lock")
        for name in ["main", "ideal"]:
            inputs[f"{name}_archive"] = str(root / f"{name}.tar")
        cases.append({"id": f"{profile}-{feature}", "inputs": inputs})
    arms = []
    for name, revision in revisions.items():
        arms.append({"id": name, "program": "/usr/bin/python3", "revision": revision,
                     "args": [str(script), "build", str(root / name), "${configuration}", "${output}"],
                     "timeout_seconds": 120,
                     "env": {"CARGO_INCREMENTAL": "0", "CARGO_BUILD_JOBS": "2", "RUSTFLAGS": "",
                             "RUSTC_WRAPPER": None, "RUSTC_WORKSPACE_WRAPPER": None,
                             "TMPDIR": str(Path.home() / "tmp")}})
    plan = {"schema": 1, "baseline": arms[0], "candidates": arms[1:], "cases": cases,
            "metric": {"extract": {"kind": "prefix", "prefix": "compile_ms="},
                       "unit": "ms", "expected": 1, "divide_by": 1, "resolution": 10},
            "check": {"kind": "json", "prefix": "result=", "pointers": ["/compiled"]},
            "warmup": "Dependencies prefetched; empty unique target per invocation; warm OS file cache. Cargo wall time from /usr/bin/time, 0.01-second resolution. Two jobs, no incremental or compiler wrapper.",
            "blocks": 5, "aa": True, "deadline_seconds": 1200}
    (root / "plan.json").write_text(json.dumps(plan, indent=2) + "\n")
    with (root / "toolchain.txt").open("w") as output:
        for command in [["rustc", "-Vv"], ["cargo", "-V"], ["uname", "-a"]]:
            run(command, stdout=output)
    print(root / "plan.json")


def build(project, config_path, output):
    config = json.loads(config_path.read_text())
    project = project / config["feature"]
    output.mkdir(parents=True, exist_ok=True)
    target = output / "target"
    if target.exists():
        raise RuntimeError("target must be new; prior observations cannot be reused")
    command = ["/usr/bin/time", "-f", "%e %M", "-o", str(output / "resources.txt"),
               "cargo", "build", "--offline", "--frozen", "--timings", "--target-dir", str(target),
               "--manifest-path", str(project / "Cargo.toml")]
    if config["profile"] == "release":
        command.append("--release")
    with (output / "cargo.log").open("w") as log:
        run(command, stdout=log, stderr=subprocess.STDOUT)
    wall, rss = (output / "resources.txt").read_text().split()
    (output / "build.json").write_text(json.dumps({"command": command, "wall_seconds": float(wall),
        "maximum_resident_kib": int(rss), "manifest": str(project / "Cargo.toml"),
        "lock_sha256": hashlib.sha256((project / "Cargo.lock").read_bytes()).hexdigest()}, indent=2) + "\n")
    print(f"compile_ms={float(wall) * 1000:.2f}")
    print('result={"compiled":true}')


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="action", required=True)
    p = sub.add_parser("prepare")
    p.add_argument("root", type=Path)
    p.add_argument("main")
    p.add_argument("ideal")
    p = sub.add_parser("build")
    p.add_argument("project", type=Path)
    p.add_argument("configuration", type=Path)
    p.add_argument("output", type=Path)
    args = parser.parse_args()
    if args.action == "prepare":
        prepare(args.root.resolve(), args.main, args.ideal)
    else:
        build(args.project, args.configuration, args.output)
