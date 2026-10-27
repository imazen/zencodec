#!/usr/bin/env python3
"""Build an isolated native-contract validation export from committed source.

The export holds the committed zencodec tree at --revision, this directory's
working files, the unpatched multi-pass prototype (baseline witnesses) at
perf/multipass-validation, and a patched copy at
perf/multipass-validation-candidate: the pass-plan patch, the candidate tests,
and imij pinned to a zqi revision with BoundedLossySession::passes (or a local
imij checkout for unpushed testing). Nothing outside the export is modified.
"""
import argparse
import hashlib
import io
import json
from pathlib import Path
import shutil
import subprocess
import tarfile

REVISION = "f73a7e0e01c6d6eefb9f08c61f2bb71100e25f54"
IMIJ_PIN = 'rev = "70401e2487f5cb2e33ea55f6b96dfc009025f473"'
IMIJ_GIT = 'git = "https://github.com/imazen/zqi", ' + IMIJ_PIN


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("export", type=Path, help="new directory outside any checkout")
    parser.add_argument("--git-dir", type=Path, required=True,
                        help="a zencodec clone holding --revision (read with git archive)")
    parser.add_argument("--revision", default=REVISION)
    pin = parser.add_mutually_exclusive_group(required=True)
    pin.add_argument("--imij-rev", help="imazen/zqi commit providing BoundedLossySession::passes")
    pin.add_argument("--imij-path", type=Path, help="local imij crate directory (unpushed testing)")
    args = parser.parse_args()

    here = Path(__file__).resolve().parent
    owned = here.parent
    export = args.export.resolve()
    if export.exists():
        raise SystemExit(f"{export} exists; use a fresh export")
    for parent in [export, *export.parents]:
        if (parent / ".git").exists() or (parent / ".jj").exists():
            raise SystemExit(f"{export} is inside a checkout ({parent}); export elsewhere")
    export.mkdir(parents=True)
    archive = subprocess.run(["git", "-C", str(args.git_dir), "archive", args.revision],
                             check=True, capture_output=True, timeout=300).stdout
    with tarfile.open(fileobj=io.BytesIO(archive)) as bundle:
        bundle.extractall(export, filter="data")

    harness = export / "perf/native-contract-validation"
    if harness.exists():
        shutil.rmtree(harness)
    shutil.copytree(owned, harness, ignore=shutil.ignore_patterns("target", "Cargo.lock"))
    baseline = export / "perf/multipass-validation"
    candidate = export / "perf/multipass-validation-candidate"
    shutil.copytree(baseline, candidate)
    patch = here / "multipass-pass-plan.patch"
    for check in (["--check"], []):
        subprocess.run(["git", "apply", *check, "-p2", str(patch)], cwd=candidate,
                       check=True, timeout=60)
    shutil.copy(here / "tests/pass_plan.rs", candidate / "tests/pass_plan.rs")
    shutil.copy(here / "native-tests/pass_plan_native.rs",
                candidate / "native/tests/pass_plan_native.rs")

    manifest = candidate / "native/Cargo.toml"
    text = manifest.read_text()
    if text.count(IMIJ_GIT) != 1:
        raise SystemExit("expected exactly one pinned imij dependency")
    if args.imij_rev:
        text = text.replace(IMIJ_PIN, f'rev = "{args.imij_rev}"')
        imij = {"git": "https://github.com/imazen/zqi", "rev": args.imij_rev}
    else:
        path = (args.imij_path / "Cargo.toml").resolve().parent
        text = text.replace(IMIJ_GIT, "path = " + json.dumps(str(path)))
        imij = {"path": str(path), "note": "local checkout; not reproducible elsewhere"}
    manifest.write_text(text)

    # Committed locks seed every build; the witness crate starts from the
    # prototype's lock until it has its own.
    for crate in [baseline, baseline / "native", candidate, candidate / "native", harness]:
        lock = crate / "dependencies.lock"
        if lock.exists():
            shutil.copy(lock, crate / "Cargo.lock")
    if not (harness / "Cargo.lock").exists():
        shutil.copy(baseline / "dependencies.lock", harness / "Cargo.lock")
    # The native spatial crate pins the spatial harness's TIFF parser.
    spatial = harness / "spatial-native"
    if (spatial / "dependencies.lock").exists():
        shutil.copy(spatial / "dependencies.lock", spatial / "Cargo.lock")
    else:
        shutil.copy(export / "perf/spatial-validation/dependencies.lock", spatial / "Cargo.lock")
    # The native video crate pins the AV1 harness's encoder; seed from its lock.
    video = harness / "video-native"
    if (video / "dependencies.lock").exists():
        shutil.copy(video / "dependencies.lock", video / "Cargo.lock")
    else:
        shutil.copy(export / "perf/av1-policy-validation/dependencies.lock", video / "Cargo.lock")

    (export / "native-contract-validation.json").write_text(json.dumps({
        "zencodec_revision": args.revision,
        "patch_sha256": sha256(patch),
        "candidate_tests_sha256": {
            "pass_plan.rs": sha256(here / "tests/pass_plan.rs"),
            "pass_plan_native.rs": sha256(here / "native-tests/pass_plan_native.rs"),
        },
        "imij": imij,
        "scope": "multi-pass pass plan: admission by planned maximum, final-pass output, "
                 "final-pass timing, Pulse progress; no memory bound is claimed",
    }, indent=2) + "\n")
    print(export)


if __name__ == "__main__":
    main()
