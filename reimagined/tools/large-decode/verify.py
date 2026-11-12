#!/usr/bin/env python3
"""Independently reproduce the compact receipt from retained raw perf CSV."""
import argparse,csv,hashlib,json,statistics
from decimal import Decimal
from pathlib import Path
p=argparse.ArgumentParser(description=__doc__);p.add_argument('run',type=Path);p.add_argument('receipt',type=Path);a=p.parse_args()
r=json.loads((a.run/'run.json').read_text());receipt=json.loads(a.receipt.read_text());group={}
assert r['stage']=='finished' and r['source_unchanged'] and r['completed_pairs']==r['required_pairs']==670
for i,row in enumerate(r['rows']):
    data={}
    for n in row['counts']:
        path=a.run/f'{i:03}-{row["codec"]}-{row["size"]}-{row["arm"]}-{n}.perf';cells={}
        with path.open() as f:
            for fields in csv.reader(f,delimiter=';'):
                if len(fields)>4 and fields[2] in ['duration_time','user_time','system_time','context-switches']:
                    cells[fields[2]]=int(Decimal(fields[0])*(1000000 if fields[1]=='msec' else 1))
        data[n]=cells
    n=row['operations']
    vals=[(data[n][k]-data[0][k])/n for k in ['duration_time','user_time','system_time','context-switches']]
    actual=[vals[0]/1000,(vals[1]+vals[2])/1000,vals[2]/1000,vals[3]]
    group.setdefault((row['codec'],row['size'],row['arm']),[]).append(actual)
for codec,size,arm,n,w,c,k,sw,*rest in receipt['cells']:
    raw=group[codec,size,arm]
    for idx,want in enumerate([w,c,k,sw]):
        # Receipt values round to six decimals; this covers only that rounding.
        assert abs(statistics.mean(v[idx] for v in raw)-want)<0.000001,(codec,size,arm,idx,want)
for family,item in r['binaries'].items():assert hashlib.sha256(Path(item['path']).read_bytes()).hexdigest()==item['sha256'],family
assert len(group)==134
print(json.dumps({'verified_pairs':len(r['rows']),'verified_receipt_cells':len(group),'independent_raw_csv_to_receipt':True,'binary_hashes_match':True,'source_unchanged_at_completion':r['source_unchanged']}))
