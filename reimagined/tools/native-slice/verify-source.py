#!/usr/bin/env python3
"""Prove that the native bitmap export changes only the two named accessors."""
import hashlib, json, subprocess
from pathlib import Path
root=Path(__file__).resolve().parent
repo=Path.home()/'work/zen/zenbitmaps'
rev='9d71b356f290f6438b810cc53d99f29301f6d247'
changes={
    'src/decode.rs': b"\n    pub fn experiment_into_pixels(self) -> Cow<'a, [u8]> { self.pixels }\n",
    'src/lib.rs': b'\n#[cfg(feature = "qoi")]\npub fn experiment_probe_qoi(data: &[u8]) -> Result<(u32,u32,bool)> { let h=qoi::decode::parse_header(data)?; Ok((h.width,h.height,h.has_alpha)) }\n',
}
tree=subprocess.check_output(['git','-C',str(repo),'ls-tree','-rz',rev])
paths=set()
for record in tree.split(b'\0'):
    if not record: continue
    meta,name=record.split(b'\t',1)
    mode,kind,digest=meta.split()
    if kind!=b'blob': continue
    name=name.decode();paths.add(name)
    data=(root/'vendor/zenbitmaps'/name).read_bytes()
    if name in changes:
        assert data.count(changes[name])==1,name
        data=data.replace(changes[name],b'',1)
    actual=hashlib.sha1(b'blob '+str(len(data)).encode()+b'\0'+data).hexdigest()
    assert actual==digest.decode(),name
extra={str(p.relative_to(root/'vendor/zenbitmaps')) for p in (root/'vendor/zenbitmaps').rglob('*') if p.is_file()}-paths
assert extra=={'EXPERIMENT_SOURCE'},extra
assert (root/'Cargo.lock').read_bytes()==(root/'dependencies.lock').read_bytes(),'lock drift'
print(json.dumps({'revision':rev,'verified_blobs':len(paths),'only_changes':list(changes)}))
