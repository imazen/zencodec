#!/usr/bin/env python3
"""Export the pinned owned codec; add two benchmark-only ownership/probe accessors."""
import io, subprocess, tarfile
from pathlib import Path
root=Path(__file__).resolve().parent
repo=Path.home()/'work/zen/zenbitmaps'
rev='9d71b356f290f6438b810cc53d99f29301f6d247'
out=root/'vendor/zenbitmaps'
if out.exists():
    raise SystemExit('Vendor export already exists; preserve it, use a fresh output for a different source.')
out.mkdir(parents=True)
archive=subprocess.check_output(['git','-C',str(repo),'archive',rev])
with tarfile.open(fileobj=io.BytesIO(archive)) as t:
    t.extractall(out,filter='data')
p=out/'src/decode.rs'
s=p.read_text()
needle="impl<'a> DecodeOutput<'a> {"
assert s.count(needle)==1
s=s.replace(needle,needle+"\n    pub fn experiment_into_pixels(self) -> Cow<'a, [u8]> { self.pixels }\n")
p.write_text(s)
p=out/'src/lib.rs'
p.write_text(p.read_text()+'\n#[cfg(feature = "qoi")]\npub fn experiment_probe_qoi(data: &[u8]) -> Result<(u32,u32,bool)> { let h=qoi::decode::parse_header(data)?; Ok((h.width,h.height,h.has_alpha)) }\n')
(out/'EXPERIMENT_SOURCE').write_text(rev+'\nOnly two accessors added; native decoding kernels unchanged.\n')
lock=root/'Cargo.lock'
frozen=root/'dependencies.lock'
if lock.exists() and lock.read_bytes()!=frozen.read_bytes():
    raise SystemExit('Existing Cargo.lock differs from dependencies.lock; preserve and inspect it.')
if not lock.exists():
    lock.write_bytes(frozen.read_bytes())
