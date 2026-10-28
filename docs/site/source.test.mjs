import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {declarations,example,maskRust,snapshotMethods} from './source.mjs';
import {api,chapterApi,entries,supplementalExamples} from './api.mjs';
import {lanes} from './content.mjs';
const root=new URL('../../',import.meta.url);
const source=await readFile(new URL('docs/examples/journeys.rs',root),'utf8');
const policySource=await readFile(new URL('docs/examples/policy.rs',root),'utf8');
const jpegSource=await readFile(new URL('docs/examples/jpeg_policy.rs',root),'utf8');
const pixelSource=await readFile(new URL('docs/examples/pixel_conversion.rs',root),'utf8');

test('source signatures preserve GATs, lifetimes, bounds and futures without default bodies',()=>{
  const rust=`// 🦀 pub trait False { fn hidden(); }
/* nested /* comment { */ end */
pub trait Source: Send {
    type View<'a>: AsRef<[u8]> where Self: 'a;
    fn read<'a>(&'a mut self, data: &'a [u8]) -> impl Future<Output = Self::View<'a>> + Send
    where Self: Sync {
        fn nested() {}
        let text = r##"} fn imaginary(); /*"##;
        let close = '}';
        todo!()
    }
    fn len(&self) -> Option<u64>;
}
impl Example {
    pub fn read(&self) {}
}`;
  const masked=maskRust(rust);
  assert.equal(masked.length,rust.length);
  assert.equal(masked.split('\n').length,rust.split('\n').length);
  const {code,line}=declarations(rust,{scope:'pub trait Source:',methods:['read','len']});
  assert.equal(line,5);
  assert.match(code,/type View<'a>: AsRef<\[u8\]> where Self: 'a;/);
  assert.match(code,/fn read<'a>\(&'a mut self, data: &'a \[u8\]\)/);
  assert.match(code,/impl Future<Output = Self::View<'a>> \+ Send/);
  assert.match(code,/where Self: Sync;/);
  assert.match(code,/fn len\(&self\) -> Option<u64>;/);
  for(const excluded of ['nested','imaginary','todo!','Example','False'])assert.ok(!code.includes(excluded));
  assert.throws(()=>declarations(rust,{scope:'pub trait Source:',methods:['nested']}),/exactly one/);
  assert.throws(()=>declarations(rust+rust,{scope:'pub trait Source:',methods:['read']}),/match once/);
  assert.throws(()=>declarations(rust,{methods:['absent']}),/exactly one/);
});

test('the flat API catalog cannot silently overwrite a source contract',async()=>{
  const catalog=await readFile(new URL('docs/site/api.mjs',root),'utf8');
  const apiSource=catalog.split('export const api = {')[1].split('\n};')[0];
  const declared=[...apiSource.matchAll(/^  (\w+): \{/gm)].map(m=>m[1]);
  assert.equal(declared.length,new Set(declared).size,'duplicate API catalog key');
  assert.deepEqual(declared,Object.keys(api),'every declared contract must survive evaluation');
  assert.match(api.collectionOpen.path,/TiffCollection/);
  assert.match(api.collectionOpenRequest.path,/OpenRequest/);
});

test('complete error declarations retain named fields and non-exhaustive context',()=>{
  const rust=`// pub enum Refusal { Wrong }
#[non_exhaustive]
pub enum Refusal {
    Limits { unsupported: Limits },
    Policy { unsupported: Policy },
}
fn hidden() {}`;
  const spec={scope:'#[non_exhaustive]\npub enum Refusal',methods:[],item:true};
  const found=declarations(rust,spec);
  assert.equal(found.line,2);
  assert.equal(found.code,rust.slice(rust.indexOf('#['),rust.indexOf('\nfn hidden')));
  assert.throws(()=>declarations(rust,{...spec,methods:['hidden']}),/no method selection/);
  assert.throws(()=>declarations(rust+rust,spec),/match once/);
});

test('every displayed contract is extracted from a unique current source declaration',async()=>{
  for(const spec of Object.values(api)) {
    const declaration=declarations(await readFile(new URL(spec.file,root),'utf8'),spec);
    assert.ok(declaration.line>0);
    for(const name of spec.methods)assert.match(declaration.code,new RegExp(`\\bfn ${name}\\b`));
    assert.ok(!declaration.code.includes(',;'));
  }
  for(const lane of lanes)for(let i=0;i<lane.steps.length;i++) {
    const keys=chapterApi[`${lane.id}-${i+1}`];
    assert.ok(keys?.length);
    for(const key of keys)assert.ok(api[key]);
  }
});

test('above, focus and below reconstruct complete independently compiled example scopes',()=>{
  const shown=new Set([
    ...entries.map(e=>e.snippet),
    ...lanes.flatMap(l=>l.steps.map(s=>s.snippet).filter(Boolean)),
  ]);
  const cases=[...shown].map(name=>[source,name]);
  const byFile={'docs/examples/journeys.rs':source,'docs/examples/pixel_conversion.rs':pixelSource};
  for(const extra of Object.values(supplementalExamples).flat()) {
    const text=byFile[extra.file||'docs/examples/journeys.rs'];
    assert.ok(text,`uncatalogued supplemental source: ${extra.file}`);
    cases.push([text,extra.snippet]);
  }
  for(const name of ['policy_default','policy_custom']) cases.push([policySource,name]);
  for(const name of ['jpeg_rewrite','jpeg_icc_refusal','jpeg_icc_admission']) cases.push([jpegSource,name]);
  for(const name of ['convert_display','convert_framebuffer']) cases.push([pixelSource,name]);
  for(const [text,name] of cases) {
    const parts=example(text,name);
    assert.ok(parts.above.startsWith('use '),name);
    assert.ok(parts.focus.trim(),name);
    assert.ok(parts.below.endsWith('}'),name);
    assert.equal(parts.full,parts.above+parts.focus+parts.below);
    assert.ok(!parts.full.includes('// ANCHOR:'));
    assert.ok(!parts.full.includes('// END:'));
    const scope=text.split(`// EXAMPLE: ${name}\n`)[1].split(`// END EXAMPLE: ${name}`)[0];
    const original=scope.split('\n').filter(l=>!/^\s*\/\/ (?:ANCHOR|END):/.test(l)).join('\n');
    assert.equal(parts.full.trim(),original.split('\n').map(l=>l.startsWith('    ')?l.slice(4):l).join('\n').trim());
  }
  assert.match(example(source,'codecset').above,/pub fn gallery_codecs\(\) -> CodecSet/);
  assert.match(example(source,'codecset').above,/with_decoder\(BmpDecoderConfig::new\(\)\)/);
  assert.throws(()=>example(source,'missing'),/unique/);
  assert.throws(()=>example(source+source,'preview'),/unique/);
});

test('integration method discovery retains generic callbacks and range sources',()=>{
  const snapshot=`pub fn Request<'a>::new() -> Self
pub fn Request<'a>::transcode_with<R: PolicyResolver>(&self, &mut R)
pub fn Request<'a>::transcode_from_read_at<S: ReadAt>(&self, &mut S)
pub fn Request<'a>::transcode_from_read_at_with<S, R>(&self, &mut S, &mut R)
pub const fn Request<'a>::configured() -> bool
pub fn Other::transcode_with<R>(&self, R)
pub Request<'a>::field: bool`;
  assert.deepEqual(snapshotMethods(snapshot,"Request<'a>"),[
    'new','transcode_with','transcode_from_read_at','transcode_from_read_at_with','configured',
  ]);
});

test('pinned integration exposes every synchronous callback and source entry point',async()=>{
  const {surface}=JSON.parse(await readFile(new URL('docs/integration-api.json',root),'utf8'));
  for(const method of ['transcode_with','transcode_from_read_at','transcode_from_read_at_with'])
    assert.match(surface,new RegExp(`pub fn ${method}<`),method);
});

test('native integration signatures retain the operation policy used by copied examples',async()=>{
  const snapshot=JSON.parse(await readFile(new URL('docs/integration-api.json',root),'utf8'));
  const manifest=await readFile(new URL('perf/jpeg-policy-validation/Cargo.toml',root),'utf8');
  const dependency=manifest.split('\n').find(line=>line.startsWith('zencodecs = '));
  assert.ok(dependency?.includes(`rev = "${snapshot.revision}"`),'captured signatures and native examples must use the same revision');
  const sync=declarations(snapshot.surface,{scope:"impl<'a> TranscodeRequest<'a> {",methods:['transcode_with','transcode_from_read_at_with'],associated:false}).code;
  const async=declarations(snapshot.surface,{scope:"impl TranscodeRequest<'_> {",methods:['transcode_async_with'],associated:false}).code;
  for(const surface of [sync,async]) {
    assert.doesNotMatch(surface,/\bPolicyResolver\b/,'a callback-only bound cannot accept BoundPolicy');
    assert.match(surface,/\bOperationPolicy\b/);
  }
});

// A selected trait view must not look like a complete implementation checklist.
test('signature selections identify omitted and required methods independently',()=>{
  const rust=`pub trait Config: Clone {
    fn format() -> u8;
    fn job(self);
    fn encode(self) { todo!() }
  }`;
  const view=declarations(rust,{scope:'pub trait Config',methods:['encode']});
  assert.deepEqual(view.omittedMethods,['format','job']);
  assert.deepEqual(view.requiredMethods,['format','job']);
  assert.ok(!view.code.includes('fn format'));
});

test('implementor selections include required method signatures but omit unrelated defaults',()=>{
  const rust=`pub trait Config {
    type Job;
    fn format() -> u8;
    fn job(self) -> Self::Job;
    fn encode(self) { todo!() }
    fn estimate(&self) -> usize { 0 }
  }`;
  const view=declarations(rust,{scope:'pub trait Config',methods:['encode'],includeRequired:true});
  assert.match(view.code,/fn format\(\) -> u8;/);
  assert.match(view.code,/fn job\(self\) -> Self::Job;/);
  assert.match(view.code,/fn encode\(self\);/);
  assert.deepEqual(view.omittedMethods,['estimate']);
  assert.deepEqual(view.requiredMethods,['format','job']);
});

test('array and const-generic signatures keep internal semicolons and braces',()=>{
  const rust=`impl Converter {
    pub fn new<const N: usize>(planes: [Plane; 3], table: &[u8; N]) -> Result<Self, Error> { todo!() }
    pub fn map(&self) -> Buffer<{ 2 * 3 }> { todo!() }
}`;
  const view=declarations(rust,{scope:'impl Converter',methods:['new','map']});
  assert.match(view.code,/planes: \[Plane; 3\], table: &\[u8; N\]\) -> Result<Self, Error>;/);
  assert.match(view.code,/fn map\(&self\) -> Buffer<\{ 2 \* 3 \}>;/);
});
