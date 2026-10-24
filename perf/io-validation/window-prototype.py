#!/usr/bin/env python3
"""Minimal active-window reuse experiment; deliberately incomplete for partial overlaps."""
import argparse
from pathlib import Path
p = argparse.ArgumentParser()
p.add_argument("export", type=Path)
args = p.parse_args()
root = args.export.resolve()
assert not (root/".git").exists() and not (root/".jj").exists(), "use an export"
path = root/"zencodec-io/src/tokio_io.rs"
text = path.read_text()
before = """            Request::Range(r) => {
                if let Some(i) = fetched"""
after = """            Request::Range(r) => {
                // Experiment only: the entire requested range is already held.
                if r.offset >= window.base() && r.end() <= window.end() {
                    window.consume((r.offset - window.base()) as usize);
                    return Ok(());
                }
                if let Some(i) = fetched"""
assert text.count(before) == 1
path.write_text(text.replace(before, after))
