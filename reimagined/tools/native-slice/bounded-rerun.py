#!/usr/bin/env python3
"""The two owner-requested process-accounting studies; never latency confirmation.

Each study runs once, five shuffled blocks. Duration calibration changes only
the loop count before the plan is frozen. No measured block is dropped or rerun.
"""
import argparse
import datetime
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import random
import shutil
import statistics
import subprocess

spec = importlib.util.spec_from_file_location('cpu_profile', Path(__file__).with_name('cpu-profile.py'))
cpu = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cpu)
ARMS = ['native', 'native_probed', 'borrowed_dyn', 'next_slice', 'older_erased', 'native_AA']
LARGE = ['direct', 'older_tokio_inline', 'next_tokio_inline', 'older_tokio_pool',
         'next_tokio_pool', 'next_buffered_one_hop', 'older_read_at', 'next_read_at']
METRICS = ['duration_time', 'cpu_total_ns', 'user_time', 'system_time', 'task-clock']


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def now():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('study', choices=['tiny-native', 'large-synthetic'])
    parser.add_argument('binary', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    saved = args.output / 'probe'
    shutil.copy2(args.binary, saved)
    saved = saved.resolve()
    repo = Path(__file__).resolve().parents[3]
    tracked = subprocess.check_output(['jj', '--ignore-working-copy', 'file', 'list'], cwd=repo, text=True).splitlines()
    paths = [p for p in tracked if p.endswith(('.rs', '.toml', '.lock', '.py')) and
             (p.startswith(('src/', 'zencodec-io/', 'zencodec-coroutine/', 'reimagined/')))]
    source = {p: digest(repo / p) for p in paths if (repo / p).is_file()}
    record = dict(study=args.study, stage='starting', started=now(), pid=os.getpid(),
                  source_revision=subprocess.check_output(['jj', '--ignore-working-copy', 'log', '-r', '@-', '--no-graph', '-T', 'commit_id'], cwd=repo, text=True).strip(),
                  binary_sha256=digest(saved), script_sha256=digest(Path(__file__)),
                  source_sha256=source, command_line=os.sys.argv, calibration=[], samples=[], rows=[],
                  required_pairs=(36 if args.study == 'tiny-native' else 8) * 5,
                  completed_pairs=0, cpu_affinity=sorted(os.sched_getaffinity(0)),
                  method='Five shuffled blocks; each cell subtracts a zero-extra process with identical fixture, runtime and one warm decode. Direct in-memory function loops; no codec CLI per decode.',
                  limits='External perf diagnostic, not zenbench Confirm. No cleanliness verdict. All blocks, A/A variation, negative adjusted counters and interference observations retained. Profiler overhead remains.',
                  env={k: os.environ.get(k) for k in ['TMPDIR', 'RUSTFLAGS', 'CARGO_BUILD_JOBS', 'RAYON_NUM_THREADS', 'OMP_NUM_THREADS', 'ZENBENCH_CPU_POLICY']})
    path = args.output / 'run.json'

    def save():
        path.write_text(json.dumps(record, indent=2) + '\n')

    def argv(case, count):
        arm = 'native' if case['arm'] == 'native_AA' else case['arm']
        if args.study == 'tiny-native':
            return [str(saved), case['codec'], arm, str(case['size']), str(count)]
        return [str(saved), arm, str(count), '1024', '16', '64']

    def measure(case, count, label):
        output = args.output / (label + '.perf')
        command = ['perf', 'stat', '-x', ';', '-o', str(output), '-e', ','.join(cpu.EVENTS), '--'] + argv(case, count)
        record.update(active_case=case, active_count=count, active_command=command, last_check=now(),
                      load_before=os.getloadavg())
        save()
        with (args.output / (label + '.log')).open('x') as log:
            child = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT)
            record['active_perf_pid'] = child.pid
            save()
            code = child.wait()
        if code:
            raise RuntimeError(f'perf exit {code}: {command}')
        result = cpu.counters(output)
        record['active_perf_pid'] = None
        record['samples'].append(dict(case=case, operations=count, counters=result,
                                      perf_file=output.name, load_after=os.getloadavg()))
        save()
        return result

    def calibrate(case):
        # Fixed doubling rule based only on duration, before the shuffled plan.
        count = 10000 if args.study == 'tiny-native' else 1000
        prefix = f'cal-{case["codec"]}-{case["size"]}'
        baseline = measure(case, 0, prefix + '-0')
        while True:
            result = measure(case, count, prefix + f'-{count}')
            duration = result['duration_time'] - baseline['duration_time']
            record['calibration'].append(dict(case=case, operations=count, adjusted_duration_ns=duration))
            save()
            if duration >= 500_000_000:
                return count
            if count >= 10_240_000:
                raise RuntimeError('Calibration duration unresolved at fixed count ceiling; no study rerun')
            count *= 2

    save()
    try:
        record['stage'] = 'calibrating'
        save()
        if args.study == 'tiny-native':
            shapes = [(codec, size) for codec in ['qoi', 'bmp', 'jpeg'] for size in [8, 63]]
            counts = {(codec, size, arm): calibrate(dict(codec=codec, size=size, arm=arm))
                      for codec, size in shapes for arm in ARMS if arm != 'native_AA'}
            for codec, size in shapes:
                counts[codec, size, 'native_AA'] = counts[codec, size, 'native']
            cases = [dict(codec=codec, size=size, arm=arm, operations=counts[codec, size, arm])
                     for codec, size in shapes for arm in ARMS]
        else:
            cases = [dict(codec='syn1', size=1024, arm=arm,
                          operations=calibrate(dict(codec='syn1', size=1024, arm=arm))) for arm in LARGE]
        rng = random.Random(20261111)
        plan = []
        for block in range(5):
            order = cases.copy()
            rng.shuffle(order)
            for index, case in enumerate(order):
                plan.append(dict(case, block=block,
                                 counts=[0, case['operations']] if (block + index) % 2 == 0 else [case['operations'], 0]))
        record.update(stage='measuring', seed=20261111, plan=plan)
        save()
        for index, case in enumerate(plan):
            reads = {}
            for count in case['counts']:
                reads[count] = measure(case, count, f'{index:03}-{case["codec"]}-{case["size"]}-{case["arm"]}-{count}')
            n = case['operations']
            row = dict(case, adjusted_ns_per_operation={k: (reads[n][k] - reads[0][k]) / n for k in METRICS},
                       context_switches_per_decode=(reads[n]['context-switches'] - reads[0]['context-switches']) / n,
                       cpu_migrations_per_decode=(reads[n]['cpu-migrations'] - reads[0]['cpu-migrations']) / n,
                       loop_duration_ns=reads[n]['duration_time'] - reads[0]['duration_time'])
            record['rows'].append(row)
            record.update(completed_pairs=index + 1, last_check=now())
            save()
            print(json.dumps(row), flush=True)
        grouped = {}
        for row in record['rows']:
            key = f'{row["codec"]}/{row["size"]}/{row["arm"]}'
            grouped.setdefault(key, []).append(row)
        summary = {}
        for key, rows in grouped.items():
            metrics = {k: [row['adjusted_ns_per_operation'][k] for row in rows] for k in METRICS}
            metrics['context_switches_per_decode'] = [r['context_switches_per_decode'] for r in rows]
            metrics['cpu_migrations_per_decode'] = [r['cpu_migrations_per_decode'] for r in rows]
            summary[key] = {k: dict(mean=statistics.mean(v), min=min(v), max=max(v)) for k, v in metrics.items()}
            summary[key]['operations_per_process'] = rows[0]['operations']
            summary[key]['minimum_loop_duration_ns'] = min(r['loop_duration_ns'] for r in rows)
        (args.output / 'summary.json').write_text(json.dumps(dict(summary=summary, rows=record['rows']), indent=2) + '\n')
        if args.study == 'tiny-native':
            record.update(stage='allocation-calls', completed_allocation_cells=0)
            save()
            allocation_rows = []
            for index, case in enumerate(cases):
                captures = []
                for count in [0, 1000]:
                    name = f'heap-{case["codec"]}-{case["size"]}-{case["arm"]}-{count}'
                    prefix = args.output / name
                    record.update(active_case=case, active_count=count, last_check=now())
                    save()
                    with prefix.with_suffix('.heap.log').open('x') as log:
                        subprocess.run(['heaptrack', '--record-only', '-o', str(prefix)] + argv(case, count), stdout=log, stderr=subprocess.STDOUT, check=True)
                    files = list(args.output.glob(name + '*.zst')) + list(args.output.glob(name + '*.gz'))
                    assert len(files) == 1, files
                    stacks = prefix.with_suffix('.stacks')
                    with prefix.with_suffix('.heap.txt').open('x') as log:
                        subprocess.run(['heaptrack_print', '-f', str(files[0]), '-F', str(stacks), '--flamegraph-cost-type', 'allocations', '-n', '15'], stdout=log, stderr=subprocess.STDOUT, check=True)
                    calls = sum(int(line.rsplit(' ', 1)[1]) for line in stacks.read_text().splitlines())
                    captures.append(dict(operations=count, calls=calls, capture=files[0].name, sha256=digest(files[0])))
                row = dict(codec=case['codec'], size=case['size'], arm=case['arm'], captures=captures,
                           allocation_calls_per_decode=(captures[1]['calls'] - captures[0]['calls']) / 1000)
                allocation_rows.append(row)
                record['completed_allocation_cells'] = index + 1
                save()
                print(json.dumps(row), flush=True)
            (args.output / 'allocations.json').write_text(json.dumps(allocation_rows, indent=2) + '\n')
        record['source_unchanged'] = all((repo / p).is_file() and digest(repo / p) == want for p, want in source.items())
        assert record['source_unchanged'], 'Source changed during study'
        record.update(stage='finished', finished=now(), active_perf_pid=None)
        save()
    except BaseException as error:
        record.update(stage='failed', failure=repr(error), last_check=now())
        save()
        raise


if __name__ == '__main__':
    main()
