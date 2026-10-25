#!/usr/bin/env python3
"""Deliver exactly one captured view to an idle reader, without grading feedback."""
import argparse
import json
from pathlib import Path

p = argparse.ArgumentParser()
p.add_argument('task', choices=[f't{i}' for i in range(1, 7)])
p.add_argument('stage', choices=['visible', 'expanded'])
p.add_argument('packets', type=Path)
p.add_argument('reader', type=Path)
a = p.parse_args()
tasks = json.loads(Path(__file__).with_name('tasks.json').read_text())
task = next(t for t in tasks if t['id'] == a.task)
name = f'{a.task}-{a.stage}'
assert not (a.reader / f'{name}.md').exists(), 'Preserve the existing response'
if a.stage == 'expanded':
    assert (a.reader / f'{a.task}-visible.md').exists(), 'Collect the first attempt first'
files = f'{name}.rs and {name}.md' if task['kind'] == 'code' else f'{name}.md'
instruction = (
    f'\n\nRead the updated packet.md. Write {files}. Preserve earlier attempts. '
    'Follow brief.md: no links, other domain sources, builds or compiler feedback. '
    'Give the substantive findings directly in chat, then wait for the next packet.\n'
)
(a.reader / 'packet.md').write_text((a.packets / f'{name}.md').read_text())
(a.reader / 'task.md').write_text(task['task'] + instruction)
print(f'Delivered {name}; prompt the idle reader to read task.md and packet.md.')
