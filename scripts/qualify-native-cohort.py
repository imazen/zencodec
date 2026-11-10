#!/usr/bin/env python3
"""Compile the full native CI consumers from a committed, isolated source export.

Records every failed stage and returns nonzero on any failure. Existing locks
seed each resolution; pixel/garb sources move together; MoxCMS
0.9.2 is required by that selected cohort. Codec and linear-sRGB pins are not advanced.
"""
import argparse
import io
import json
import os
from pathlib import Path
import subprocess
import tarfile

ROOTS = {
    'io': 'perf/io-validation',
    'jpeg': 'perf/jpeg-policy-validation',
    'av1': 'perf/av1-policy-validation',
    'spatial': 'perf/spatial-validation',
    'lcms': 'perf/spatial-lcms-validation',
}
IO_TESTS = ['site_examples','encoder_retention','policy_resolution','policy_engine','policy_execution','policy_native','policy_disclosure','policy_stream']
IO_BINS = ['policy_media','transport_contracts']


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    parser.add_argument('--revision', required=True)
    parser.add_argument('--target', type=Path, required=True)
    parser.add_argument('--roots', nargs='+', choices=ROOTS, default=list(ROOTS))
    args = parser.parse_args()
    repo = Path(__file__).resolve().parents[1]
    revision = subprocess.check_output(['git','rev-parse',args.revision],cwd=repo,text=True).strip()
    output = args.output.resolve(); output.mkdir(parents=True,exist_ok=False)
    source = output/'source'; source.mkdir()
    with tarfile.open(fileobj=io.BytesIO(subprocess.check_output(['git','archive',revision],cwd=repo))) as files:
        files.extractall(source,filter='data')
    env = dict(os.environ,CARGO_TARGET_DIR=str(args.target.resolve()))
    receipt = {'revision':revision,'status':'running','steps':[]}
    def save(): (output/'receipt.json').write_text(json.dumps(receipt,indent=2)+'\n')
    def run(label, command):
        step={'label':label,'command':command,'status':'running'};receipt['steps'].append(step);receipt['stage']=label;save()
        with (output/(label+'.log')).open('wb') as log:
            try: rc=subprocess.run(command,cwd=source,env=env,stdout=log,stderr=subprocess.STDOUT,timeout=900).returncode
            except subprocess.TimeoutExpired: rc=124
        step.update(status='passed' if rc==0 else 'failed',returncode=rc);save();print(label,rc,flush=True);return rc==0
    for name in args.roots:
        directory=ROOTS[name];package=source/directory;manifest=directory+'/Cargo.toml'
        seed=package/('locks/harness.lock' if name=='io' else 'dependencies.lock')
        (package/'Cargo.lock').write_bytes(seed.read_bytes())
        if name=='io' and not run('io-sources',['python3',directory+'/setup.py']):continue
        if name=='jpeg' and not run('jpeg-mox-update',['cargo','update','--manifest-path',manifest,'-p','moxcms','--precise','0.9.2']):continue
        if not run(name+'-metadata',['cargo','metadata','--manifest-path',manifest,'--format-version','1']):continue
        command=['cargo','test','--manifest-path',manifest,'--no-run','--locked']
        if name=='io':
            for test in IO_TESTS:command+=['--test',test]
            for binary in IO_BINS:command+=['--bin',binary]
        else:command+=['--lib','--tests']
        if name in ['spatial','lcms']:command+=['--features','cms_oracle,old_jpeg']
        run(name+'-compile',command)
    receipt['status']='failed' if any(s['status']=='failed' for s in receipt['steps']) else 'passed';save()
    raise SystemExit(int(receipt['status']=='failed'))

if __name__=='__main__':main()
