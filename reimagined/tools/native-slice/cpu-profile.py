#!/usr/bin/env python3
"""External perf accounting around fixed in-memory loops in retained probes.

This is a diagnostic, not zenbench confirmation. All blocks survive; no gate or
acceptance threshold is relaxed. Zero-operation controls include identical
fixture creation, runtime startup and one warm operation.
"""
import argparse, datetime, hashlib, json, os, random, shutil, statistics, subprocess
from decimal import Decimal
from pathlib import Path

EVENTS=['task-clock','duration_time','user_time','system_time','context-switches','cpu-migrations']
def counters(path):
    result={}
    for line in path.read_text().splitlines():
        if not line or line.startswith('#'): continue
        fields=line.split(';')
        if len(fields)<3 or fields[2] not in EVENTS: continue
        assert fields[2] not in result,fields
        scale=1000000 if fields[1]=='msec' else 1
        result[fields[2]]=int(Decimal(fields[0])*scale)
        assert fields[4]=='100.00',fields
    assert set(result)==set(EVENTS),result
    result['cpu_total_ns']=result['user_time']+result['system_time']
    return result

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('native',type=Path)
    p.add_argument('synthetic',type=Path)
    p.add_argument('output',type=Path)
    p.add_argument('--parse-only',type=Path)
    a=p.parse_args()
    if a.parse_only:
        print(json.dumps(counters(a.parse_only)));return
    a.output.mkdir(parents=True,exist_ok=False)
    root=Path(__file__).resolve().parents[3]
    bins={}
    for label,binary in [('native',a.native),('synthetic',a.synthetic)]:
        saved=a.output/(label+'-probe');shutil.copy2(binary,saved);bins[label]=saved.resolve()
    operations=10000
    cases=[{'family':'native','codec':kind,'arm':arm,'label':label}
           for kind in ['qoi','bmp','jpeg']
           for label,arm in [('native','native'),('native_AA','native'),('borrowed_dyn','borrowed_dyn'),('next_slice','next_slice'),('older_erased','older_erased')]]
    cases += [{'family':'synthetic','codec':'syn1','arm':arm,'label':arm} for arm in
              ['direct','direct_AA','older_slice_typed','next_slice_boxed','older_tokio_inline','next_tokio_inline','older_tokio_pool','next_tokio_pool','next_buffered_one_hop']]
    rng=random.Random(20261111);plan=[]
    for block in range(5):
        order=list(cases);rng.shuffle(order)
        for i,case in enumerate(order):
            plan.append(dict(case,block=block,counts=[0,operations] if (block+i)%2==0 else [operations,0]))
    record={'started':datetime.datetime.now(datetime.timezone.utc).isoformat(),'stage':'starting',
            'source_revision':subprocess.check_output(['jj','log','-r','@-','--no-graph','-T','commit_id'],cwd=root,text=True).strip(),
            'command_line':os.sys.argv,'pid':os.getpid(),'probe_sha256':{k:hashlib.sha256(v.read_bytes()).hexdigest() for k,v in bins.items()},
            'script_sha256':hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
            'perf_version':subprocess.check_output(['perf','--version'],text=True).strip(),
            'operations':operations,'blocks':5,'seed':20261111,'plan':plan,'samples':[],
            'cpu_affinity':sorted(os.sched_getaffinity(0)),
            'method':'perf stat inherited task-clock and duration/user/system tool counters. For each shuffled case/block, subtract an independently launched zero-operation control from 10000 direct in-memory decode calls; both contain identical setup and one warm decode. No CLI launch per decode.',
            'limits':'Profiler diagnostic; no cleanliness or latency acceptance claim. Background load/frequency/cache state can affect results. Existing invalid zenbench studies stay invalid. Borrowed native arm uses &NoPulse. Negative adjusted system time is preserved as control noise.',
            'env':{k:os.environ.get(k) for k in ['TMPDIR','RAYON_NUM_THREADS','OMP_NUM_THREADS','CARGO_BUILD_JOBS','RUSTFLAGS','ZENBENCH_CPU_POLICY']}}
    dest=a.output/'run.json'
    def save():dest.write_text(json.dumps(record,indent=2)+'\n')
    save()
    rows=[]
    for sequence,case in enumerate(plan):
        reads={}
        for count in case['counts']:
            name=f'{sequence:03}-{case["codec"]}-{case["label"]}-{count}'
            output=a.output/(name+'.perf')
            argv=[str(bins[case['family']])]
            argv += [case['codec'],case['arm'],'256',str(count)] if case['family']=='native' else [case['arm'],str(count)]
            command=['perf','stat','-x',';','-o',str(output),'-e',','.join(EVENTS),'--']+argv
            record.update(stage='running',current_case=case,current_count=count,completed_pairs=sequence,load_before=os.getloadavg())
            save()
            with (a.output/(name+'.log')).open('x') as log:
                job=subprocess.Popen(command,stdout=log,stderr=subprocess.STDOUT)
                record['active_perf_pid']=job.pid;save();code=job.wait()
            if code:
                record.update(stage='failed',returncode=code);save();raise RuntimeError(command)
            measured=counters(output);reads[count]=measured
            record['samples'].append({'case':case,'operations':count,'counters':measured,'perf_file':output.name,'load_after':os.getloadavg()})
            save()
        row={**case,'adjusted_ns_per_operation':{k:(reads[operations][k]-reads[0][k])/operations for k in
             ['duration_time','user_time','system_time','cpu_total_ns','task-clock']},
             'adjusted_switches_per_operation':(reads[operations]['context-switches']-reads[0]['context-switches'])/operations}
        rows.append(row)
        print(json.dumps(row),flush=True)
    grouped={}
    for row in rows:
        key='/'.join([row['family'],row['codec'],row['label']]);grouped.setdefault(key,[]).append(row)
    summary={key:{metric:{'mean':statistics.mean(r['adjusted_ns_per_operation'][metric] for r in group),
                        'min':min(r['adjusted_ns_per_operation'][metric] for r in group),
                        'max':max(r['adjusted_ns_per_operation'][metric] for r in group)}
                  for metric in ['duration_time','user_time','system_time','cpu_total_ns','task-clock']}
             for key,group in grouped.items()}
    (a.output/'summary.json').write_text(json.dumps({'method':record['method'],'limits':record['limits'],'units':'nanoseconds per decode, baseline-subtracted','probe_sha256':record['probe_sha256'],'rows':rows,'summary':summary},indent=2)+'\n')
    record.update(stage='finished',completed_pairs=len(rows),active_perf_pid=None,finished=datetime.datetime.now(datetime.timezone.utc).isoformat());save()

if __name__=='__main__':
    try:
        main()
    except BaseException as error:
        if len(os.sys.argv)>3:
            path=Path(os.sys.argv[3])/'run.json'
            if path.exists():
                failed=json.loads(path.read_text());failed.update(stage='failed',failure=repr(error));path.write_text(json.dumps(failed,indent=2)+'\n')
        raise
