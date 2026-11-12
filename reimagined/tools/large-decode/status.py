#!/usr/bin/env python3
"""Inspect durable study progress and the recorded live processes."""
import argparse,json,os
from pathlib import Path
p=argparse.ArgumentParser(description=__doc__);p.add_argument('run',type=Path);a=p.parse_args()
r=json.loads((a.run/'run.json').read_text())
print(json.dumps({k:r.get(k) for k in ['stage','last_check','completed_pairs','required_pairs','active_case','failure','source_unchanged']},indent=2))
for field in ['pid','active_child_pid']:
    pid=r.get(field)
    if not pid:continue
    try:os.kill(pid,0)
    except ProcessLookupError:print(f'{field} {pid}: exited');continue
    command=Path(f'/proc/{pid}/cmdline').read_bytes().replace(b'\0',b' ').decode(errors='replace')
    print(f'{field} {pid}: live; {command}')
