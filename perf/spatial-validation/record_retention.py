"""Record actual retained payloads and require identical conversion artifacts."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tomllib

baseline, current, log, output = map(Path, sys.argv[1:])
root = Path(__file__).resolve().parents[2]
manifest = tomllib.loads((root / "perf/spatial-validation/Cargo.toml").read_text())


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


run = log.read_text()
assert "129 passed; 0 failed; 0 ignored" in run
wrapper = next(line for line in run.splitlines() if line.startswith("run-heavy: done"))
assert "rc=0 " in wrapper
rows = []
for index in range(6):
    before = baseline / "color" / f"selection-{index}"
    after = current / "color" / f"selection-{index}"
    old = json.loads((before / "publication.json").read_text())
    new = json.loads((after / "publication.json").read_text())
    comparison = json.loads((after / "comparison.json").read_text())
    assert old["source_bytes_before"] == new["source_bytes_before"]
    assert old["destination_bytes"] == new["destination_bytes"]
    assert new["source_bytes_after"] == new["auxiliary_bytes"]
    assert new["source_color_released"] and new["same_auxiliary_addresses"]
    assert comparison["color_error_limit"] == 0.0001
    assert comparison["max_absolute_color_error"] <= comparison["color_error_limit"]
    assert comparison["alpha_mismatch"] == 0 and comparison["nonfinite"] == 0
    hashes = {}
    for name in ["source.f32", "converted.f32", "oracle.f32", "source.icc", "target.icc"]:
        hashes[name] = sha(after / name)
        assert sha(before / name) == hashes[name], (index, name)
    rows.append({"selection": index, "before": old, "after": new,
                 "max_absolute_color_error": comparison["max_absolute_color_error"],
                 "identical_artifacts_sha256": hashes})

sources = ["perf/spatial-validation/src/jxl_planned_color_tests.rs",
           "perf/spatial-validation/src/jxl_color_oracle_tests.rs",
           "perf/spatial-validation/Cargo.toml", "perf/spatial-validation/dependencies.lock",
           "perf/spatial-validation/record_retention.py"]
receipt = {
    "kind": "native retained-payload qualification; not RSS or throughput",
    "source_zencodec_base": subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=root, text=True).strip(),
    "source_zenpixels": manifest["patch"]["crates-io"]["zenpixels"]["rev"],
    "source_zenjxl": manifest["dependencies"]["zenjxl"]["rev"],
    "source_sha256": {name: sha(root / name) for name in sources},
    "baseline": str(baseline), "current": str(current), "log": str(log),
    "log_sha256": sha(log), "resource_wrapper": wrapper,
    "tests": {"passed": 129, "failed": 0, "ignored": 0, "strict_clippy": "passed"},
    "selections": rows,
    "limits": ["Payload charges exclude native/CMS workspace and owner bookkeeping.",
               "Source and destination coexist during conversion.",
               "Original clones and inseparable backends may retain original color.",
               "This neither establishes general ICC/CMYK conversion nor safe encoder backpressure."],
}
output.write_text(json.dumps(receipt, indent=2) + "\n")
print(f"Recorded {len(rows)} unchanged conversions and actual retained payloads in {output}")
