"""Export a pinned zenpipe object and apply the isolated publication experiment."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tarfile

BASE = "a478b91e641eb9d9f605b547b6ff00c76ee15e1d"
TREE = "363a08a00961689ef501537991046880ddb756d8"
PATCHES = [
    "01-core-family-e59b3803.patch",
    "02-publication-witness.patch",
    "03-zensimx-edges-from-725d05c2.patch",
]

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("checkout", type=Path, help="local zenpipe repository containing the pinned object")
parser.add_argument("output", type=Path, help="new export directory outside a workspace with Cargo patches")
args = parser.parse_args()
tree = subprocess.check_output(
    ["git", "-C", str(args.checkout), "rev-parse", BASE + "^{tree}"], text=True
).strip()
if tree != TREE:
    raise SystemExit("Pinned source tree mismatch")
args.output.mkdir(parents=True, exist_ok=False)
archive = subprocess.Popen(
    ["git", "-C", str(args.checkout), "archive", BASE], stdout=subprocess.PIPE
)
try:
    with tarfile.open(fileobj=archive.stdout, mode="r|") as source:
        source.extractall(args.output, filter="data")
finally:
    archive.stdout.close()
if archive.wait():
    raise SystemExit("Source export failed; partial output retained for inspection")
hashes = {}
for name in PATCHES:
    patch = Path(__file__).resolve().parent / name
    hashes[name] = hashlib.sha256(patch.read_bytes()).hexdigest()
    subprocess.run(["git", "apply", "--check", str(patch)], cwd=args.output, check=True)
    subprocess.run(["git", "apply", str(patch)], cwd=args.output, check=True)
(args.output / "publication-source.json").write_text(
    json.dumps({"base": BASE, "tree": TREE, "patch_sha256": hashes}, indent=2) + "\n"
)
print(args.output)
