#!/usr/bin/env python3
"""Compare native refusal and collection heap usage; reuse the checked heap parser."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import runpy
import subprocess


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('executable', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--source-revision', required=True)
    args = parser.parse_args()
    tools = runpy.run_path(str(Path(__file__).with_name('profile-preparation.py')))
    run = tools['run']
    executable = args.executable.resolve()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    report = dict(source_revision=args.source_revision,
                  binary_sha256=hashlib.sha256(executable.read_bytes()).hexdigest(),
                  native_revision=tools['native_revision'](), dimensions=[1024, 768],
                  profile='debug; full-process heap allocations; not throughput or workspace bound',
                  started=datetime.now(timezone.utc).isoformat(), cells=[])
    def save():
        (output / 'progress.json').write_text(json.dumps(report, indent=2) + '\n')
    save()
    for configuration in ['baseline', 'progressive']:
        fixture = output / f'{configuration}.jpg'
        run([str(executable), 'fixture', configuration, str(fixture)], output / f'{configuration}.fixture.log')
        source_hash = hashlib.sha256(fixture.read_bytes()).hexdigest()
        successful = []
        for mode in ['one-shot-refuse', 'controlled-refuse', 'one-shot-ok', 'controlled-copy', 'controlled-own']:
            name = f'{configuration}-{mode}'
            report['stage'] = name
            save()
            raw, trace = output / f'{name}.raw', output / f'{name}.heaptrack'
            env = dict(os.environ, LD_PRELOAD='/usr/lib/heaptrack/libheaptrack_preload.so',
                       DUMP_HEAPTRACK_OUTPUT=str(raw))
            run([str(executable), mode, configuration, str(fixture)], output / f'{name}.run.log', env=env)
            if not mode.endswith('refuse'):
                lines = (output / f'{name}.run.log').read_text().splitlines()
                hashes = [line for line in lines if line.startswith('pixels ')]
                if len(hashes) != 1:
                    raise ValueError(f'Missing pixel checksum: {name}')
                successful.append(hashes[0])
            with raw.open('rb') as source, trace.open('wb') as target, (output / f'{name}.interpret.log').open('wb') as errors:
                subprocess.run(['/usr/lib/heaptrack/libexec/heaptrack_interpret'],
                               stdin=source, stdout=target, stderr=errors, check=True, timeout=120)
            analysis = output / f'{name}.analysis.txt'
            run(['heaptrack_print', '-f', str(trace), '--print-allocators', '0', '--print-temporary', '0'], analysis)
            measured = tools['peak_heap'](trace.read_text())
            tools['verify_peak'](measured['peak_heap_bytes'], analysis.read_text())
            report['cells'].append(dict(configuration=configuration, mode=mode,
                                        input_sha256=source_hash, **measured))
            save()
        if len(successful) != 3 or len(set(successful)) != 1:
            raise ValueError(f'Pixel paths disagree: {configuration}: {successful}')
    report['stage'] = 'complete'
    report['completed'] = datetime.now(timezone.utc).isoformat()
    save()
    (output / 'summary.json').write_text(json.dumps(report, indent=2) + '\n')


if __name__ == '__main__':
    main()
