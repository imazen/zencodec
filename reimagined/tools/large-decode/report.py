#!/usr/bin/env python3
"""Render the retained diagnostic without dropping blocks or unresolved contrasts."""
import argparse
import json
import re
from pathlib import Path


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('run', type=Path)
    p.add_argument('report', type=Path)
    p.add_argument('receipt', type=Path)
    a = p.parse_args()
    r = json.loads((a.run / 'run.json').read_text())
    s = json.loads((a.run / 'summary.json').read_text())
    assert r['stage'] == 'finished' and r['source_unchanged']
    assert r['completed_pairs'] == r['required_pairs'] == 670
    assert len(s) == 134
    rows = {(x['codec'], x['size'], x['arm'], x['block']): x['per_decode'] for x in r['rows']}
    def key(codec, size, arm):
        return f'{codec}/{size}/{arm}'
    def aa(codec, size, metric):
        base = 'direct' if codec == 'syn1' else 'native'
        return max(abs(rows[codec, size, base, b][metric] - rows[codec, size, base + '_AA', b][metric]) for b in range(5))
    def mean(codec, size, arm, metric):
        return s[key(codec, size, arm)][metric]['mean']
    compact = dict(source=r['source_revision'], binaries=r['binaries'], artifacts=str(a.run),
                   pairs=r['completed_pairs'], blocks=5, units='times in microseconds/decode; switches/decode',
                   columns=['codec','size','arm','operations','wall_us','cpu_us','kernel_us','switches','AA_wall_us','AA_cpu_us','min_loop_ms'], cells=[])
    repo = Path(__file__).resolve().parents[3]
    route_order = {}
    for family, folder in [('native', 'native-slice'), ('synthetic', 'ablation')]:
        section = (repo / 'reimagined/tools' / folder / 'src/lib.rs').read_text().split('pub const ARMS:', 1)[1].split('];', 1)[0]
        route_order[family] = re.findall(r'"([A-Za-z_]+)"', section) + (['native_AA'] if family == 'native' else [])
    lines = ['# Optional Pulse and 4096² / 8192² decode drivers', '',
             'Missing: current-head allocation counts, native metadata/general-error conformance, tile or ROI browsing, actual disk/network throughput and concurrent-runtime fairness. The native bindings remain RGB8 experiments. No production codec is migrated or released.', '',
             'The borrowed slice path and optional borrowed observer are implemented. Decoders still require `Send + \'static`; a decoder that retains a source borrow between steps, or is thread-bound, remains outside this contract. `None` disables observation; `Some(&pulse)` retains reporting and cancellation. Worker paths accept `Option<SharedPulse>`. The borrowed, unshareable observer control reaches native checks and JPEG reporting. Default fixture paths exercise `None` without constructing a shared observer.', '',
             'Exact pixels, dimensions and descriptors pass through all 15 native paths at both sizes for QOI, BMP and baseline JPEG. Synthetic RGBA8 passes all 19 paths at both sizes with tightly packed and 16-byte-padded rows, exact row coverage, completion and unchanged padding. Native output extraction/adoption retains pointer identity.', '',
             '## Method and limits', '',
             r['method'], '', r['limits'], '',
             'Each value is the arithmetic mean of five baseline-subtracted blocks. A/A spread is the maximum absolute same-block reference-versus-identical-control difference; it is an observed variation bound, not a confidence interval. Differences within that spread are unresolved. Exceeding it does not establish a clean latency acceptance result. Negative kernel counters remain in the evidence.', '',
             'Both native and synthetic probes use Tokio multi-thread runtimes with two workers. The older and newer arms within each binary share one dependency graph. These are different probe binaries; compare paths within a binary rather than attributing cross-binary differences to the contract. The synthetic shape uses 64-row strips and a 64-step quota at both sizes. This differs from the earlier 1024² study’s 16-row strips.', '',
             'Native `next_slice` drives the boxed protocol into a minimal owned-frame sink. The user-facing `memory::decode` wrapper additionally collects frames and clones info; its complete end-to-end overhead is not timed here. `borrowed_dyn` is the box-free measurement entry. JPEG still parses its header twice on the whole-image binding (`Header=()`); the hook permits reuse but this binding does not demonstrate it. The older erased BMP/QOI adapters copy the decoded output; their complete delta is not trait overhead.', '',
             '## Native results', '',
             'Times in milliseconds/decode. N is extra decodes/process. Spread is wall / CPU milliseconds. Streamed and Cursor sources are memory-backed. `native_AA` executes native unchanged.', '',
             '| Codec / size | Route | N | Wall | CPU | Kernel | Switches | A/A wall / CPU |',
             '|---|---|---:|---:|---:|---:|---:|---:|']
    for codec in ['qoi', 'bmp', 'jpeg', 'syn1']:
        if codec == 'syn1':
            lines += ['', '## Synthetic results', '',
                      'Times in milliseconds/decode; source is RGBA8 with 64-row strips and quota 64. `direct_AA` executes direct unchanged.', '',
                      '| Size | Route | N | Wall | CPU | Kernel | Switches | A/A wall / CPU |',
                      '|---|---|---:|---:|---:|---:|---:|---:|']
        for size in [4096, 8192]:
            selected = route_order['synthetic' if codec == 'syn1' else 'native']
            for arm in selected:
                cell = s[key(codec, size, arm)]
                w, c, k = [mean(codec, size, arm, m) for m in ['duration_time', 'cpu_total_ns', 'system_time']]
                sw = mean(codec, size, arm, 'context-switches')
                aw, ac = aa(codec, size, 'duration_time'), aa(codec, size, 'cpu_total_ns')
                label = f'{codec} {size}²' if codec != 'syn1' else f'{size}²'
                lines.append(f'| {label} | {arm} | {cell["operations"]} | {w/1e6:.3f} | {c/1e6:.3f} | {k/1e6:.3f} | {sw:.3f} | {aw/1e6:.3f} / {ac/1e6:.3f} |')
                compact['cells'].append([codec, size, arm, cell['operations'], *[round(v/1000,6) for v in [w,c,k]], round(sw,6), round(aw/1000,6), round(ac/1000,6), round(cell['min_loop_ns']/1e6,6)])
    lines += ['', '## Native resident-path contrasts', '',
              'Differences in microseconds/decode; A/A spread shown alongside each. Negative means the first named path used less time. An unresolved result is not a win or a tie.', '',
              '| Codec / size | Contrast | Wall delta / A/A | Wall resolved? | CPU delta / A/A | CPU resolved? |',
              '|---|---|---:|---|---:|---|']
    for codec in ['qoi','bmp','jpeg']:
        for size in [4096,8192]:
            for first, second in [('next_slice','native_probed'), ('borrowed_dyn','native'), ('native_probed','native')]:
                delta = [mean(codec,size,first,m)-mean(codec,size,second,m) for m in ['duration_time','cpu_total_ns']]
                spread = [aa(codec,size,m) for m in ['duration_time','cpu_total_ns']]
                status = ['yes' if abs(d)>v else 'unresolved' for d,v in zip(delta,spread)]
                lines.append(f'| {codec} {size}² | {first} − {second} | {delta[0]/1000:.3f} / {spread[0]/1000:.3f} | {status[0]} | {delta[1]/1000:.3f} / {spread[1]/1000:.3f} | {status[1]} |')
    findings = ['## What the measurements say', '',
                'At 8192² the boxed resident path’s difference from native-plus-probe is unresolved against A/A variation for all three codecs. This is not a claim of zero overhead. At 4096² BMP is an exception: the boxed path has a positive diagnostic contrast just beyond the observed spread.', '']
    for codec in ['qoi','bmp','jpeg']:
        size=8192
        pool=[mean(codec,size,'next_pool',m)/1e6 for m in ['duration_time','cpu_total_ns']]
        one=[mean(codec,size,'next_buffered_one_hop',m)/1e6 for m in ['duration_time','cpu_total_ns']]
        findings += [f'For an existing {codec.upper()} input owner at 8192², one-hop wall / CPU is {one[0]:.3f} / {one[1]:.3f} ms; generic pool is {pool[0]:.3f} / {pool[1]:.3f} ms. The generic pool source here is a borrowed InMemory slice and uses its owned input window. The one-hop source is an existing Arc owner, handed to the worker without copying its bytes.', '']
    for size in [4096,8192]:
        old=[mean('syn1',size,'older_tokio_pool',m)/1e6 for m in ['duration_time','cpu_total_ns']]
        new=[mean('syn1',size,'next_tokio_pool',m)/1e6 for m in ['duration_time','cpu_total_ns']]
        spread=[aa('syn1',size,m)/1e6 for m in ['duration_time','cpu_total_ns']]
        status=['beyond A/A spread' if abs(n-o)>v else 'unresolved' for o,n,v in zip(old,new,spread)]
        findings += [f'Synthetic {size}² pool wall / CPU: older {old[0]:.3f} / {old[1]:.3f} ms; newer {new[0]:.3f} / {new[1]:.3f} ms. A/A spreads are {spread[0]:.3f} / {spread[1]:.3f} ms: wall {status[0]}, CPU {status[1]}. The earlier pool regression cannot be declared removed by the window repair.', '']
    findings += ['Keep the resident slice and input-owner routes as direct conveniences. Use the input-owner route for resident offload; this evidence does not justify routing it through the generic source window. InPlace fairness and actual tile browsing still need their own evidence; this bounded phase does not implement or qualify them.', '']
    at=lines.index('## Method and limits');lines[at:at]=findings
    minimum = min(x['loop_duration_ns'] for x in r['rows']) / 1e6
    lines += ['', '## Identity and qualification', '',
              f'Source: `{r["source_revision"]}`. All 670 pairs retained; source hashes unchanged. Minimum measured adjusted loop: {minimum:.3f} ms. Calibration targeted at least 500 ms before planning; later block durations were not used to select or discard cells.', '',
              'Native sources: zenbitmaps `9d71b356f290f6438b810cc53d99f29301f6d247`, zenjpeg `6d6539dfaf3b9e09d74ad9ef7a56b4e4e7518dd0`. The two benchmark-only bitmap accessors are source-verified; native kernels are unchanged. Tracked dependency locks preserve both graphs. Full source hashes, exact process commands, fixture hashes, PIDs and raw counters are retained with the run.', '',
              'Before timing, the original 8K BMP prefix probe failed: it unwrapped a native amplification refusal on a 64 KiB prefix. That attempt contains zero calibration samples and zero timing pairs. It is preserved in the adjacent `large-study/` directory. The corrected valid-file binding asks for more bytes and retains a 128-byte regression. This is not general malformed-file classification qualification.', '',
              'The inherited async planner also continued source reads after a short read signalled cancellation. The failing-first control observed 22 reads; the repaired decode/transcode planning loops check optional Pulse between reads, and the control requires exactly one. Native errors retain priority over a stop raised by their failing call.', '',
              'Local standalone-workspace tests on r7900x, strict Clippy, strict Rustdoc, formatting and genuine no_std WASM/freestanding builds pass for the optional-Pulse source. Native/synthetic probe suites and strict Clippy pass with the corrected binding. The parent-repository CI and Fuzz runs pass (they do not test this separate workspace); the four native documentation qualification jobs still fail on the pre-existing archmage version conflict. They remain unqualified.', '',
              f'Native probe SHA256: `{r["binaries"]["native"]["sha256"]}`.', '',
              f'Synthetic probe SHA256: `{r["binaries"]["synthetic"]["sha256"]}`.', '',
              f'Raw artifact directory: `{a.run}`. The compact JSON receipt beside this report uses microseconds. This report’s main tables use milliseconds.', '',
              'Reproduction: `PROBE_TARGET=... TMPDIR=$HOME/tmp run-heavy --mem 4G --jobs 2 -- just -f reimagined/justfile probe-check`; build with `probe-build`, then set `ZENBENCH_EXECUTOR` to the retained verified zenbench performance executor and run `large-profile FRESH_OUTPUT` under the same wrapper. `probe-setup` prepares fresh exports and locks. No native CPU flags.', '']
    lines += ['Toolchain: Rust 1.99.1 / LLVM 23.1.1; r7900x (Ryzen 9 7900X), verified uniform performance-class cpuset 0–23. Two runtime workers and explicit build/pool limits; no native CPU flags.', '']
    for name in ['optional-gates.log','probe-build-first.log','probe-build-second.log','probe-build-third.log','probe-build-fourth.log','large-study.log','large-study-repaired.log']:
        path = a.run.parent / name
        if path.exists():
            matches = [x for x in path.read_text().splitlines() if x.startswith('run-heavy: done')]
            if matches:
                lines.append(f'`{name}`: `{matches[-1]}`')
                lines.append('')
    text = '\n'.join(lines)
    receipt = json.dumps(compact,separators=(',',':'))+'\n'
    assert len(receipt.encode()) < 30000
    assert len(text.encode()) < 30000
    a.report.parent.mkdir(parents=True,exist_ok=True)
    a.report.write_text(text)
    a.receipt.write_text(receipt)
    print(f'{len(r["rows"])} pairs, report {len(text.encode())} bytes, receipt {len(receipt.encode())} bytes')

if __name__ == '__main__':
    main()
