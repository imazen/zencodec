import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { implementation, catalogItems, sourceFingerprint } from './code-first.mjs';
import { codeFlows } from './code-first-data.mjs';
import { example } from './source.mjs';
const root=fileURLToPath(new URL('../../',import.meta.url));
const manifest=JSON.parse(await readFile(new URL('../code-api/manifest.json',import.meta.url),'utf8'));
test('every catalog declaration survives rendering and metadata matches exact source inputs',async()=>{
  assert.equal(manifest.packages.length,8);
  for(const pkg of manifest.packages) {
    assert.equal(await sourceFingerprint(root,pkg.directory),pkg.fingerprint,pkg.name);
    const surface=await readFile(new URL(`../code-api/${pkg.name}.txt`,import.meta.url),'utf8');
    const metadata=await readFile(new URL(`../code-api/${pkg.name}.json`,import.meta.url),'utf8');
    assert.equal(createHash('sha256').update(metadata).digest('hex'),pkg.details_sha256);
    const items=catalogItems(surface,JSON.parse(metadata));
    assert.deepEqual(new Set(items.map(i=>i.signature)),new Set(surface.trim().split('\n')),pkg.name);
    assert.equal(new Set(items.map(i=>i.id)).size,items.length,`${pkg.name}: stable distinct IDs`);
    const published=JSON.parse(await readFile(new URL(`../_site/code-api/${pkg.name}.json`,import.meta.url),'utf8'));
    assert.deepEqual(published,items);
  }
});
test('every flow copies a complete source scope and preserves context on both sides',async()=>{
  assert.equal(new Set(codeFlows.map(f=>f.id)).size,codeFlows.length);
  for(const flow of codeFlows)for(const step of flow.steps) {
    const source=await readFile(root+step.file,'utf8');
    const parts=step.example?example(source,step.example):implementation(source,step.scope);
    assert.equal(parts.above+parts.focus+parts.below,parts.full);
    if(!step.example)assert.equal(parts.full,source);
    assert.ok(parts.focus.trim());
    assert.ok(step.api?.length||step.external?.length,flow.id);
  }
  const html=await readFile(new URL('../_site/code.html',import.meta.url),'utf8');
  const ids=[...html.matchAll(/\bid="([^"]+)"/g)].map(m=>m[1]);
  assert.equal(new Set(ids).size,ids.length);
  for(const [,hash] of html.matchAll(/href="#([^"]+)"/g))assert.ok(ids.includes(hash),hash);
  for(const flow of codeFlows)assert.ok(ids.includes(flow.id));
});
test('implementation extraction ignores Rust comments and strings but rejects ambiguous scopes',()=>{
  const source='use x::Y;\nimpl Foo { fn x() { let s = "}"; /* } */ } }\nfn after() {}';
  const parts=implementation(source,'impl Foo');
  assert.equal(parts.focus,'impl Foo { fn x() { let s = "}"; /* } */ } }');
  assert.throws(()=>implementation(source+source,'impl Foo'),/once/);
});
