import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {declarations,example,maskRust} from './source.mjs';
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
