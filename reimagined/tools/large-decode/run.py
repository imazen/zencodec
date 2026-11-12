#!/usr/bin/env python3
"""Finite 4096/8192 native and synthetic route diagnostic, preserving every pair."""
import argparse
from datetime import datetime, timezone
from decimal import Decimal
import hashlib
import json
import os
from pathlib import Path
import random
import re
import shutil
import statistics
import subprocess

EVENTS = ['task-clock', 'duration_time', 'user_time', 'system_time', 'context-switches', 'cpu-migrations']
METRICS = EVENTS + ['cpu_total_ns']

def now():
    return datetime.now(timezone.utc).isoformat()

def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b''):
            h.update(chunk)
    return h.hexdigest()

def counters(path):
    result = {}
    for line in path.read_text().splitlines():
        fields = line.split(';')
        if line.startswith('#') or len(fields) < 5 or fields[2] not in EVENTS:
            continue
        assert fields[2] not in result and fields[4] == '100.00', fields
        result[fields[2]] = int(Decimal(fields[0]) * (1000000 if fields[1] == 'msec' else 1))
    assert set(result) == set(EVENTS), result
    result['cpu_total_ns'] = result['user_time'] + result['system_time']
    return result

def arms(path):
    text = path.read_text().split('pub const ARMS:', 1)[1].split('];', 1)[0]
    return re.findall(r'"([A-Za-z_]+)"', text)

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('native', type=Path)
    parser.add_argument('synthetic', type=Path)
    parser.add_argument('output', type=Path)
    a = parser.parse_args()
    a.output.mkdir(parents=True, exist_ok=False)
    root = Path(__file__).resolve().parents[3]
    bins = {}
    for family in ['native', 'synthetic']:
        dest = a.output / (family + '-probe')
        shutil.copy2(getattr(a, family), dest)
        bins[family] = dest.resolve()
    tracked = subprocess.check_output(['jj', '--ignore-working-copy', 'file', 'list'], cwd=root, text=True).splitlines()
    source = {p: digest(root / p) for p in tracked if p.endswith(('.rs', '.toml', '.lock', '.py')) and
              (root / p).is_file()}
    for p in ['reimagined/Cargo.lock', 'reimagined/tools/native-slice/Cargo.lock', 'reimagined/tools/ablation/Cargo.lock']:
        source[p] = digest(root / p)
    record = dict(stage='starting', started=now(), last_check=now(), pid=os.getpid(),
                  source_revision=subprocess.check_output(['jj', '--ignore-working-copy', 'log', '-r', '@-', '--no-graph', '-T', 'commit_id'], cwd=root, text=True).strip(),
                  source_sha256=source, binaries={k: dict(path=str(v), sha256=digest(v)) for k, v in bins.items()},
                  command=os.sys.argv, cpu_affinity=sorted(os.sched_getaffinity(0)),
                  fixtures=[], qualified=[], calibration=[], samples=[], rows=[], completed_pairs=0,
                  strip_rows=64, work_quota=64, blocks=5, seed=20261112,
                  method='Five shuffled blocks, paired zero-extra process with identical file load/runtime/one warm decode. Fixed doubling calibration from one decode until adjusted loop >=500ms. Native AA and synthetic direct_AA unchanged. None Pulse on every path.',
                  limits='External perf diagnostic; no zenbench Confirm verdict. Every pair and negative counter retained. Memory-backed Streamed and Cursor sources, no disk or network throughput claim. InPlace called from Runtime::block_on outside a worker task; no fairness/worker-replacement claim.',
                  env={k: os.environ.get(k) for k in ['TMPDIR', 'RUSTFLAGS', 'RAYON_NUM_THREADS', 'OMP_NUM_THREADS', 'CARGO_BUILD_JOBS']})
    dest = a.output / 'run.json'
    def save():
        record['last_check'] = now()
        dest.write_text(json.dumps(record, indent=2) + '\n')
    def command(argv, label):
        record['active_command'] = list(map(str, argv))
        save()
        with (a.output / (label + '.log')).open('x') as log:
            child = subprocess.Popen(argv, stdout=log, stderr=subprocess.STDOUT)
            record['active_child_pid'] = child.pid
            save()
            code = child.wait()
        record['active_child_pid'] = None
        save()
        if code:
            raise RuntimeError(f'exit {code}, {label}; see retained log')
    try:
        record['stage'] = 'fixtures-and-correctness'
        save()
        fixtures = {}
        cases = []
        for size in [4096, 8192]:
            for codec in ['qoi', 'bmp', 'jpeg', 'syn1']:
                family = 'synthetic' if codec == 'syn1' else 'native'
                path = (a.output / f'{codec}-{size}.input').resolve()
                argv = [str(bins[family]), 'fixture']
                argv += [str(size), str(path)] if family == 'synthetic' else [codec, str(size), str(path)]
                command(argv, f'fixture-{codec}-{size}')
                fixtures[codec, size] = path
                record['fixtures'].append(dict(codec=codec, size=size, bytes=path.stat().st_size, path=path.name, sha256=digest(path)))
                argv = [str(bins[family])]
                argv += ['qualify', '0', '@' + str(path), '64', '64'] if family == 'synthetic' else [codec, 'qualify', '@' + str(path), '0']
                command(argv, f'qualify-{codec}-{size}')
                record['qualified'].append(dict(codec=codec, size=size, strided=(family == 'synthetic')))
                selected = arms(root / 'reimagined/tools' / ('ablation' if family == 'synthetic' else 'native-slice') / 'src/lib.rs')
                if family == 'native':
                    selected += ['native_AA']
                cases += [dict(family=family, codec=codec, size=size, arm=arm) for arm in selected]
                save()
        def argv(case, count):
            arm = 'native' if case['arm'] == 'native_AA' else case['arm']
            args = [str(bins[case['family']])]
            path = '@' + str(fixtures[case['codec'], case['size']])
            return args + ([arm, str(count), path, '64', '64'] if case['family'] == 'synthetic' else [case['codec'], arm, path, str(count)])
        def measure(case, count, label):
            output = a.output / (label + '.perf')
            record.update(active_case=case, active_count=count, load_before=os.getloadavg())
            command(['perf', 'stat', '-x', ';', '-o', str(output), '-e', ','.join(EVENTS), '--'] + argv(case, count), label)
            result = counters(output)
            record['samples'].append(dict(case=case, operations=count, counters=result, perf_file=output.name, load_after=os.getloadavg()))
            save()
            return result
        record['stage'] = 'calibrating'
        save()
        for case in cases:
            if case['arm'] == 'native_AA':
                case['operations'] = next(c['operations'] for c in cases if c['codec'] == case['codec'] and c['size'] == case['size'] and c['arm'] == 'native')
                continue
            if case['arm'] == 'direct_AA':
                case['operations'] = next(c['operations'] for c in cases if c['codec'] == case['codec'] and c['size'] == case['size'] and c['arm'] == 'direct')
                continue
            prefix = f'cal-{case["codec"]}-{case["size"]}-{case["arm"]}'
            baseline = measure(case, 0, prefix + '-0')
            count = 1
            while True:
                result = measure(case, count, prefix + f'-{count}')
                duration = result['duration_time'] - baseline['duration_time']
                record['calibration'].append(dict(case=case.copy(), operations=count, adjusted_duration_ns=duration))
                save()
                if duration >= 500_000_000:
                    case['operations'] = count
                    break
                if count >= 8192:
                    raise RuntimeError('Unresolved 500ms calibration at 8192 decodes; study stops')
                count *= 2
            print(f'calibrated {case} loop={duration}ns', flush=True)
        rng = random.Random(record['seed'])
        plan = []
        for block in range(record['blocks']):
            order = cases.copy()
            rng.shuffle(order)
            for index, case in enumerate(order):
                plan.append(dict(case, block=block, counts=[0, case['operations']] if (block + index) % 2 == 0 else [case['operations'], 0]))
        record.update(stage='measuring', plan=plan, required_pairs=len(plan))
        save()
        for index, case in enumerate(plan):
            reads = {}
            for count in case['counts']:
                reads[count] = measure(case, count, f'{index:03}-{case["codec"]}-{case["size"]}-{case["arm"]}-{count}')
            n = case['operations']
            row = dict(case, per_decode={k: (reads[n][k] - reads[0][k]) / n for k in METRICS}, loop_duration_ns=reads[n]['duration_time'] - reads[0]['duration_time'])
            record['rows'].append(row)
            record['completed_pairs'] = index + 1
            save()
            print(json.dumps(row), flush=True)
        grouped = {}
        for row in record['rows']:
            grouped.setdefault(f'{row["codec"]}/{row["size"]}/{row["arm"]}', []).append(row)
        summary = {key: {metric: dict(mean=statistics.mean(r['per_decode'][metric] for r in rows), min=min(r['per_decode'][metric] for r in rows), max=max(r['per_decode'][metric] for r in rows)) for metric in METRICS} | dict(operations=rows[0]['operations'], min_loop_ns=min(r['loop_duration_ns'] for r in rows)) for key, rows in grouped.items()}
        (a.output / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
        record['source_unchanged'] = all(digest(root / p) == want for p, want in source.items())
        assert record['source_unchanged'], 'Source changed during study'
        record.update(stage='finished', finished=now())
        save()
    except BaseException as e:
        record.update(stage='failed', failure=repr(e))
        save()
        raise

if __name__ == '__main__':
    main()
