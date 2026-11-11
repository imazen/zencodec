#!/usr/bin/env python3
"""Reuse the repository's heaptrack capture/stack-count method for driver arms."""
import argparse,hashlib,json,shutil,subprocess
from pathlib import Path
p=argparse.ArgumentParser()
p.add_argument('binary',type=Path)
p.add_argument('output',type=Path)
a=p.parse_args()
a.output.mkdir(parents=True,exist_ok=False)
saved=a.output/'allocations'
shutil.copy2(a.binary,saved)
rows=[]
for arm in ['direct','older_slice_typed','older_slice_dyn_stack','older_slice_dyn','next_slice_boxed','older_read_at','next_read_at','older_coroutine','older_tokio_inline','next_tokio_inline','older_tokio_pool','next_tokio_pool','next_buffered_one_hop']:
    captures=[]
    for count in [0,1000]:
        name=f'{arm}-{count}'
        prefix=a.output/name
        with prefix.with_suffix('.log').open('x') as f:
            subprocess.run(['heaptrack','--record-only','-o',str(prefix),str(saved),arm,str(count)],stdout=f,stderr=subprocess.STDOUT,check=True)
        files=list(a.output.glob(name+'*.zst'))+list(a.output.glob(name+'*.gz'))
        assert len(files)==1,(name,files)
        stacks=prefix.with_suffix('.stacks')
        with prefix.with_suffix('.txt').open('x') as f:
            subprocess.run(['heaptrack_print','-f',str(files[0]),'-F',str(stacks),'--flamegraph-cost-type','allocations','-n','15'],stdout=f,stderr=subprocess.STDOUT,check=True)
        total=sum(int(line.rsplit(' ',1)[1]) for line in stacks.read_text().splitlines())
        captures.append({'operations':count,'calls':total,'capture':files[0].name,'capture_sha256':hashlib.sha256(files[0].read_bytes()).hexdigest()})
    row={'arm':arm,'captures':captures,'additional_calls_in_1000_operations':captures[1]['calls']-captures[0]['calls']}
    rows.append(row)
    print(json.dumps(row),flush=True)
summary={'binary_sha256':hashlib.sha256(saved.read_bytes()).hexdigest(),'shape':'256x256 RGBA8; 4-row strips; quota64; same warm operation in both processes','method':'heaptrack allocation-cost flamegraph leaves summed over all threads; 1000-operation process minus zero-operation process; includes runtime effects that scale with operations','rows':rows,'scope':'allocation calls only, not latency, live heap or native workspace; no native codec work'}
(a.output/'summary.json').write_text(json.dumps(summary,indent=2)+'\n')
