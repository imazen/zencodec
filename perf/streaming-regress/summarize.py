#!/usr/bin/env python3
"""Tabulate streaming-regress logs: per group, the PR-vs-main 95% CI from
each run, then the last run's means and its pr+begin CI.

    python3 summarize.py run1.log run2.log run3.log
"""
import re
import sys


def load(path):
    group, out = None, {}
    for line in open(path, encoding="utf-8"):
        line = re.sub(r"\x1b\[[0-9;]*m", "", line.rstrip("\n"))
        m = re.match(r"\s{2}(\S+/\S+)\s+\d+ rounds", line)
        if m:
            group = m.group(1)
            continue
        m = re.match(r"\s+[├╰]─ (main|pr\+begin|pr)\s+([\d.]+) ±[\d.]+(\w+)\s+\[([^\]]+)\]", line)
        if m and group:
            out.setdefault(group, {})[m.group(1)] = (m.group(2) + m.group(3), m.group(4))
    return out


runs = [load(p) for p in sys.argv[1:]]
last = runs[-1]
heads = " | ".join(f"run {i + 1} pr vs main" for i in range(len(runs)))
print(f"| group | {heads} | last run main → pr | last run pr+begin vs main |")
print("|" + "---|" * (len(runs) + 3))
for g, row in last.items():
    cis = " | ".join(r.get(g, {}).get("pr", ("", ""))[1] for r in runs)
    means = f"{row['main'][0]} → {row['pr'][0]}"
    begin = row.get("pr+begin", ("", ""))[1]
    print(f"| {g} | {cis} | {means} | {begin} |")
