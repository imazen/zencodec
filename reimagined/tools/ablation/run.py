#!/usr/bin/env python3
"""Retain one frozen zenbench run and its exact source/binary identity."""
import argparse,datetime,hashlib,json,os,subprocess,sys
from pathlib import Path
parser=argparse.ArgumentParser()
parser.add_argument('binary',type=Path)
parser.add_argument('output',type=Path)
parser.add_argument('flags',nargs=argparse.REMAINDER)
args=parser.parse_args()
args.output.mkdir(parents=True,exist_ok=False)
root=Path(__file__).resolve().parents[3]
paths=[root/'src',root/'zencodec-io/src',root/'zencodec-coroutine/src',root/'reimagined/zencodec/src',root/'reimagined/zencodec-io/src',Path(__file__).resolve().parent]
files={}
for path in paths:
    for f in sorted(path.rglob('*')):
        if f.is_file() and 'target' not in f.parts and '__pycache__' not in f.parts:
            files[str(f.relative_to(root))]=hashlib.sha256(f.read_bytes()).hexdigest()
revision=subprocess.check_output(['jj','log','-r','@-','--no-graph','-T','commit_id'],cwd=root,text=True).strip()
flags=args.flags[1:] if args.flags[:1]==['--'] else args.flags
command=[str(args.binary.resolve())]+flags
manifest={'started':datetime.datetime.now(datetime.timezone.utc).isoformat(),'source_revision':revision,'binary':str(args.binary.resolve()),'binary_sha256':hashlib.sha256(args.binary.read_bytes()).hexdigest(),'source_sha256':files,'command':command,'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'cargo':subprocess.check_output(['cargo','--version'],text=True).strip(),'flags':{k:os.environ.get(k) for k in ['RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS','CARGO_BUILD_JOBS','CARGO_TARGET_DIR','TMPDIR','ZENBENCH_DATA_DIR']},'stage':'starting'}
path=args.output/'run.json'
def save():
    path.write_text(json.dumps(manifest,indent=2)+'\n')
save()
with (args.output/'result.json').open('x') as stdout,(args.output/'stderr.log').open('x') as stderr:
    job=subprocess.Popen(command,stdout=stdout,stderr=stderr,cwd=root)
    manifest.update(stage='running',pid=job.pid)
    save()
    print(f'benchmark PID {job.pid}; source {revision}; artifacts {args.output}',flush=True)
    code=job.wait()
manifest.update(stage='finished',returncode=code,finished=datetime.datetime.now(datetime.timezone.utc).isoformat(),source_unchanged=all((root/name).exists() and hashlib.sha256((root/name).read_bytes()).hexdigest()==digest for name,digest in files.items()))
save()
print(f'benchmark returned {code}; full stdout and stderr retained',flush=True)
sys.exit(code)
