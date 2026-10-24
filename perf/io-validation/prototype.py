#!/usr/bin/env python3
"""Apply a PRIVATE batching prototype to an exported source tree, never a checkout.
Usage: prototype.py /path/to/export rows|bounded|baseline
No public types/signatures change. This is an experiment, not a proposed default.
"""
import argparse
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument("export", type=Path)
parser.add_argument("variant", choices=["rows", "bounded", "baseline"])
args = parser.parse_args()
root = args.export.resolve()
assert not (root / ".git").exists() and not (root / ".jj").exists(), "use an export"
path = root / "zencodec-io/src/tokio_io.rs"
backup = path.with_suffix(".io-baseline")
if not backup.exists():
    backup.write_bytes(path.read_bytes())
base = backup.read_text()
if args.variant == "baseline":
    path.write_text(base)
    raise SystemExit(0)

def replace(s, before, after, count=1):
    assert s.count(before) == count, (before, s.count(before), count)
    return s.replace(before, after)

s = replace(base, "    Need,\n    Done(OutputInfo),", "    Need,\n    Yield,\n    Done(OutputInfo),")
s = replace(s, "    pulse: Option<SharedPulse>,\n) -> Batch<D, S>", "    pulse: Option<SharedPulse>,\n    should_yield: impl Fn(&S, u32) -> bool,\n) -> Batch<D, S>")
s = replace(s, "    let stop = progress.stop();\n    let outcome = loop", "    let stop = progress.stop();\n    let mut produced = 0u32;\n    let outcome = loop")
s = replace(s, "            Event::Done(info) => break Ok(BatchEnd::Done(info)),\n            _ => continue,",
    "            Event::Done(info) => break Ok(BatchEnd::Done(info)),\n"
    "            Event::Rows { rows, .. } => {\n"
    "                produced = produced.saturating_add(rows);\n"
    "                if should_yield(&sink, produced) { break Ok(BatchEnd::Yield); }\n"
    "            }\n            _ => continue,")
needle = "run_batch(decoder, sink, window, pulse_for_batch)"
assert s.count(needle) == 2
s = s.replace(needle, "run_batch(decoder, sink, window, pulse_for_batch, |_, _| false)", 1)
condition = "rows >= 1" if args.variant == "rows" else "rows >= 64 || sink.pending_output().len() >= 65536"
s = s.replace(needle, f"run_batch(decoder, sink, window, pulse_for_batch, |sink, rows| {{ let _ = sink; {condition} }})", 1)
s = replace(s, "                Ok(BatchEnd::Need) => {", "                Ok(BatchEnd::Yield) => {}\n                Ok(BatchEnd::Need) => {", 2)
path.write_text(s)
print("Applied experimental variant:", args.variant)
