#!/usr/bin/env python3
"""Summarize the two frozen studies without collecting or filtering observations."""
import argparse
import hashlib
import json
from pathlib import Path
import re


def load(path):
    return json.loads(path.read_text())


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('root', type=Path)
    p.add_argument('report', type=Path)
    p.add_argument('receipt', type=Path)
    a = p.parse_args()
    assert not a.report.exists() and not a.receipt.exists(), 'Preserve earlier reports'
    native = load(a.root / 'tiny-native/summary.json')
    large = load(a.root / 'large-synthetic/summary.json')
    runs = {s: load(a.root / s / 'run.json') for s in ['tiny-native', 'large-synthetic']}
    for s, r in runs.items():
        assert r['stage'] == 'finished' and r['source_unchanged']
        assert r['completed_pairs'] == r['required_pairs']
    assert len(native['rows']) == 180 and len(large['rows']) == 40
    allocations = load(a.root / 'tiny-native/allocations.json')
    assert len(allocations) == 36
    allocation = {f'{r["codec"]}/{r["size"]}/{r["arm"]}': r['allocation_calls_per_decode'] for r in allocations}
    tiny_rows, controls, differences, synthetic_rows = [], [], [], []
    for codec in ['qoi', 'bmp', 'jpeg']:
        for size in [8, 63]:
            prefix = f'{codec}/{size}'
            aa = {}
            for metric, label in [('duration_time', 'wall'), ('cpu_total_ns', 'cpu')]:
                by = {arm: {r['block']: r['adjusted_ns_per_operation'][metric] for r in native['rows']
                            if r['codec'] == codec and r['size'] == size and r['arm'] == arm}
                      for arm in ['native', 'native_AA']}
                aa[label + '_spread_ns'] = max(abs(by['native'][b] - by['native_AA'][b]) for b in range(5))
                aa[label + '_absolute_mean_difference_ns'] = abs(native['summary'][prefix + '/native'][metric]['mean'] -
                                                               native['summary'][prefix + '/native_AA'][metric]['mean'])
            controls.append(dict(codec=codec, size=size, **aa))
            assert allocation[prefix + '/native'] == allocation[prefix + '/native_AA']
            for arm in ['native', 'native_probed', 'borrowed_dyn', 'next_slice', 'older_erased', 'native_AA']:
                value = native['summary'][prefix + '/' + arm]
                tiny_rows.append(dict(codec=codec, size=size, arm=arm,
                                      extra_decodes_per_process=value['operations_per_process'],
                                      wall_us=value['duration_time']['mean'] / 1000,
                                      cpu_us=value['cpu_total_ns']['mean'] / 1000,
                                      minimum_loop_ms=value['minimum_loop_duration_ns'] / 1e6,
                                      allocation_calls_per_decode=allocation[prefix + '/' + arm]))
            for label, left, right in [('boxed_step_path', 'next_slice', 'native_probed'), ('extra_header_probe', 'native_probed', 'native')]:
                entry = dict(codec=codec, size=size, contrast=label)
                for metric, name in [('duration_time', 'wall'), ('cpu_total_ns', 'cpu')]:
                    delta = native['summary'][prefix + '/' + left][metric]['mean'] - native['summary'][prefix + '/' + right][metric]['mean']
                    entry[name + '_delta_ns'] = delta
                    entry[name + '_AA_spread_ns'] = aa[name + '_spread_ns']
                    entry[name + '_status'] = 'unresolved' if abs(delta) <= aa[name + '_spread_ns'] else 'exceeds observed A/A spread; diagnostic only'
                differences.append(entry)
    for arm in ['direct', 'older_tokio_inline', 'next_tokio_inline', 'older_tokio_pool', 'next_tokio_pool', 'next_buffered_one_hop', 'older_read_at', 'next_read_at']:
        value = large['summary']['syn1/1024/' + arm]
        synthetic_rows.append(dict(arm=arm, extra_decodes_per_process=value['operations_per_process'],
                                   wall_us=value['duration_time']['mean'] / 1000,
                                   cpu_us=value['cpu_total_ns']['mean'] / 1000,
                                   kernel_us=value['system_time']['mean'] / 1000,
                                   context_switches_per_decode=value['context_switches_per_decode']['mean'],
                                   minimum_loop_ms=value['minimum_loop_duration_ns'] / 1e6))
    wrapper = {p.name: re.findall(r'run-heavy: done.*', p.read_text()) for p in (a.root / 'logs').glob('*.log')}
    receipt = dict(source_revision=runs['tiny-native']['source_revision'],
                   runtime_baseline='3bfa754857020586c9d88cbeb11259186b5ea4d1',
                   native_compiled_source='739bc0dfa3add545dcac2a9f58f43176b4dc76f9',
                   synthetic_probe_compiled_source=runs['tiny-native']['calibration_repair']['previous_source'],
                   artifact_root=str(a.root), binary_identities=load(a.root / 'binaries.json'),
                   method=runs['tiny-native']['method'], limits=runs['tiny-native']['limits'],
                   cpu_placement='zenbench exec performance: enforced_uniform hard cpuset CPUs 0-23, capacity1024_l332768K; legacy rendezvous plus retained published exclusive lease',
                   blocks=5, pairs={'tiny-native':180, 'large-synthetic':40},
                   allocation_cells=36, heaptrack_extra_decodes=1000,
                   AA_definition=load(a.root / 'analysis-plan.json')['AA_spread'],
                   calibration_repair=runs['tiny-native']['calibration_repair'],
                   minimum_measured_loop_ms=min(r['minimum_loop_ms'] for r in tiny_rows + synthetic_rows),
                   source_unchanged={k:r['source_unchanged'] for k,r in runs.items()},
                   tiny_native=tiny_rows, AA_controls=controls, derived_differences=differences,
                   large_synthetic=synthetic_rows, run_heavy=wrapper,
                   raw_summary_sha256={s:sha(a.root / s / 'summary.json') for s in runs})
    lines = ['# Two bounded repaired-source measurement reruns', '',
             '## Missing and limits first', '',
             'Confirmed latency, a zenbench cleanliness verdict, full native conformance and the later PR211 changes remain unmeasured. These are external perf diagnostics, including counter overhead. Allocation calls do not establish peak native workspace. The old erased bitmap adapter copies decoded RGB pixels; its difference from native cannot be assigned to trait design.', '',
             'No requested cells are missing: tiny native completed 180/180 paired cells and 36/36 allocation cells; large synthetic completed 40/40 paired cells. No measured block was dropped or repeated.', '',
             'The first controller stopped during calibration on an arm-omitting filename collision, before any measured pair or plan. Its record and all 11 counter captures remain. The guarded repair reused the completed native calibration and then ran the frozen plan once. No failed timing was promoted or retried for acceptance.', '',
             '## Identity and method', '',
             f'Measurement controller source: `{receipt["source_revision"]}` on review/reimagined-documents ([PR210](https://github.com/imazen/zencodec/pull/210)). Codec/driver implementations remain those of `{receipt["runtime_baseline"]}`; no later [PR211](https://github.com/imazen/zencodec/pull/211) changes were imported.', '',
             'Native probe: retained 739bc0df compiled inputs, unchanged bytes. Synthetic probe: 283510da shape/qualification CLI over unchanged repaired codec/driver code. Dependencies remain locked. No target-cpu=native. Source hashes stayed unchanged during both measured studies.', '',
             'Host: Ryzen 9 7900X. Both studies acquired the legacy benchmark rendezvous and retained published zenbench exclusive lease; zenbench exec verified a homogeneous performance hard cpuset, CPUs 0-23. run-heavy limited memory to 4G and jobs to 2; Rayon/OMP pools were explicitly 2.', '',
             'Each process creates the same fixture/runtime and performs one warm decode. Subtract a separately launched zero-extra process from the extra-decode process, then divide by the recorded count. CPU is inherited user + system time across threads. Five blocks use seed 20261111 and shuffled cells with alternating zero/extra order. All counters, negative adjusted values, loads, failures and observations survive.', '',
             'Calibrate each cell by fixed doubling until adjusted external duration reaches 500 ms; the identical A/A arm uses its native count. Counts freeze before the five blocks. Heaptrack uses independent zero/1,000-extra captures with the same warm operation, summing allocation-cost stack leaves over all threads.', '',
             'A/A spread below is the maximum absolute native-versus-identical-A/A difference among the five matching blocks, independently for wall and CPU. It is an observed variation bound, not a confidence interval. An absolute contrast no larger than that spread is unresolved. Exceeding it is still diagnostic evidence, not confirmed latency acceptance.', '',
             '## Measurement 1: tiny native RGB8', '',
             'Wall / CPU and A/A spread are microseconds per decode. N is extra decodes per process in each of five blocks. Allocation calls are per decode from zero/1,000-extra heaptrack subtraction. Borrowed control uses &NoPulse. AA rows execute the unchanged native arm.', '',
             '| Codec / size | Arm | N | Wall / CPU µs | Calls / decode | A/A spread wall / CPU µs |',
             '|---|---|---:|---:|---:|---:|']
    for r in tiny_rows:
        aa = next(c for c in controls if c['codec'] == r['codec'] and c['size'] == r['size'])
        lines.append(f'| {r["codec"]} {r["size"]}² | {r["arm"]} | {r["extra_decodes_per_process"]:,} | {r["wall_us"]:.6f} / {r["cpu_us"]:.6f} | {r["allocation_calls_per_decode"]:g} | {aa["wall_spread_ns"]/1000:.6f} / {aa["cpu_spread_ns"]/1000:.6f} |')
    lines += ['', 'Derived differences, in **nanoseconds** per decode. Each bullet gives wall / CPU, followed by the corresponding A/A spread and classification. The boxed-step contrast includes admission, sink/protocol checks and dispatch as well as its box; it does not isolate a single allocation.', '']
    for codec in ['qoi', 'bmp', 'jpeg']:
        for size in [8, 63]:
            entries = [d for d in differences if d['codec'] == codec and d['size'] == size]
            text = []
            for d in entries:
                text.append(f'{d["contrast"]}: {d["wall_delta_ns"]:+.3f} / {d["cpu_delta_ns"]:+.3f} ns; A/A {d["wall_AA_spread_ns"]:.3f} / {d["cpu_AA_spread_ns"]:.3f} ns; wall {d["wall_status"]}, CPU {d["cpu_status"]}')
            lines.append(f'- **{codec} {size}²:** ' + '; '.join(text) + '.')
    lines += ['', '## Measurement 2: 1024×1024 RGBA8, 16-row strips, quota 64', '',
              'Means across all five shuffled blocks. Times are microseconds per decode; CPU sums worker threads. These process-profile means are not uninstrumented latency estimates or a confirmation verdict.', '',
              '| Route | N | Wall µs | CPU µs | Kernel µs | Context switches / decode |',
              '|---|---:|---:|---:|---:|---:|']
    for r in synthetic_rows:
        lines.append(f'| {r["arm"]} | {r["extra_decodes_per_process"]:,} | {r["wall_us"]:.3f} | {r["cpu_us"]:.3f} | {r["kernel_us"]:.3f} | {r["context_switches_per_decode"]:.6f} |')
    lines += ['', f'The shortest retained measured extra-operation loop was {receipt["minimum_measured_loop_ms"]:.3f} ms. The receipt includes per-cell minima. Raw summary.json files retain ranges and all measured blocks.', '',
              '## Commands and resources', '', 'Executed from reimagined/:', '', '```sh']
    for name in ['tiny-native.log', 'tiny-native-resume.log', 'large-synthetic.log']:
        command = (a.root / 'logs' / name).read_text().splitlines()[1]
        lines.append(command)
    lines += ['```', '', 'The initial tiny-native command stopped in calibration; the resume command continued that same study. No extra measured study was launched.', '', '```text']
    for name in ['build.log', 'build-final.log', 'clippy.log', 'tiny-native.log', 'tiny-native-resume.log', 'large-synthetic.log']:
        lines += [name + ': ' + value for value in wrapper[name]]
    lines += ['```', '', 'These wrapper peaks cover the profiled process trees, not a codec workspace bound.', '', 'Frozen binary SHA256:', '', '```text']
    lines += [name + ' ' + item['sha256'] for name, item in receipt['binary_identities'].items()]
    lines += ['```', '', f'Raw artifacts: {a.root}', '',
              'Sources, locked dependency identities and complete process counters are retained in run.json, binaries.json, the native prior receipts and the captured logs. Original calibration failure and counters remain in run.before-calibration-repair.json and the original tiny-native log.', '',
              'The requested two studies are complete. No merge, codec migration, new design work, browser measurement or optional page update was performed.']
    a.report.write_text('\n'.join(line.rstrip() for line in lines) + '\n')
    a.receipt.write_text(json.dumps(receipt, indent=2) + '\n')
    print(json.dumps({'report':str(a.report), 'receipt':str(a.receipt), 'receipt_bytes':a.receipt.stat().st_size,
                      'minimum_loop_ms':receipt['minimum_measured_loop_ms']}))


if __name__ == '__main__':
    main()
