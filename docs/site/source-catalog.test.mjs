import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { sourceCatalog } from './source-catalog.mjs';
import { declarations } from './source.mjs';
import { filterCatalog } from './catalog-search.mjs';

const read = file => readFile(new URL(file,import.meta.url),'utf8');
const source = await read('../../src/detect.rs');
const surface = await read('../code-api/zencodec.txt');
const details = JSON.parse(await read('../code-api/zencodec.json'));
const scopes = ['impl dyn SourceEncodingDetails {','impl dyn SourceEncodingDetails + Send {','impl dyn SourceEncodingDetails + Send + Sync {'];
const plain = html => html.replace(/<[^>]*>/g,'').replaceAll('&quot;','"').replaceAll('&gt;','>').replaceAll('&lt;','<').replaceAll('&amp;','&');

test('all three source-derived dyn signatures, docs and locations survive searchable rendering',()=>{
  const items=sourceCatalog(source,surface,details);
  assert.equal(items.length,3);
  assert.equal(new Set(items.map(item=>item.id)).size,3);
  for(const [i,item] of items.entries()) {
    const declaration=declarations(source,{scope:scopes[i],methods:['codec_details'],associated:false});
    assert.equal(item.signature,declaration.code);
    assert.equal(plain(item.html),declaration.code);
    assert.equal(item.detail.line,declaration.line);
    assert.equal(item.detail.file,'src/detect.rs');
    assert.ok(source.split('\n')[item.detail.line-1].includes('pub fn codec_details'));
    assert.equal(item.provenance,'Source-backed supplement');
    assert.match(item.detail.docs,/Downcast to a concrete codec probe type/);
    assert.ok(item.documentation.length);
    for(const part of item.documentation)if(part.html)assert.equal(plain(part.html),part.code);
    assert.deepEqual(filterCatalog([item],{search:'codec_details SourceEncodingDetails',kind:'function',legacy:false}),[item]);
    assert.deepEqual(filterCatalog([item],{search:'concrete codec probe'}),[item],'source docs are searchable');
  }
  assert.equal(filterCatalog(items,{search:'codec_details'}).length,3);
  assert.equal(filterCatalog(items,{search:'codec_details Send'}).length,2);
  assert.equal(filterCatalog(items,{search:'codec_details Sync'}).length,1);
  assert.equal(filterCatalog(items,{search:'codec_details',kind:'trait'}).length,0);
  assert.equal(filterCatalog(items,{search:'not_a_real_signature'}).length,0);
  assert.ok(items[0].documentation.some(part=>part.code?.includes('details.codec_details::<WebPProbe>()')));
});

test('explicit supplement fails on source or compiler coverage drift and follows signature/docs changes',()=>{
  assert.throws(()=>sourceCatalog(source.replace('impl dyn SourceEncodingDetails + Send {','impl dyn SourceEncodingDetails + Sync {'),surface,details),/dyn impls changed/);
  assert.throws(()=>sourceCatalog(source+'\nimpl dyn SourceEncodingDetails + Unpin {}',surface,details),/dyn impls changed/);
  assert.throws(()=>sourceCatalog(source.replace('pub fn codec_details','pub fn renamed'),surface,details),/Expected exactly one/);
  assert.throws(()=>sourceCatalog(source.replace('pub fn codec_details','fn codec_details'),surface,details),/no longer public/);
  assert.throws(()=>sourceCatalog(source.replace('pub fn codec_details','pub(crate) fn codec_details'),surface,details),/no longer public/);
  assert.throws(()=>sourceCatalog(source.replace('    pub fn codec_details','    pub fn extra(&self) {}\n    pub fn codec_details'),surface,details),/gained methods|Missing source documentation/);
  assert.throws(()=>sourceCatalog(source.replaceAll('    /// Downcast to a concrete codec probe type.',''),surface,details),/Missing source documentation/);
  assert.throws(()=>sourceCatalog(source,surface+'\npub fn zencodec::SourceEncodingDetails::codec_details<T>()',details),/Compiler catalog now includes/);
  assert.throws(()=>sourceCatalog(source,surface,[...details,{path:'zencodec::SourceEncodingDetails::codec_details'}]),/Compiler catalog now includes/);
  const revised=source.replaceAll('Option<&T>','Result<&T, ()>').replaceAll('Downcast to a concrete codec probe type.','Updated source contract.');
  for(const item of sourceCatalog(revised,surface,details)) {
    assert.ok(item.signature.includes('Result<&T, ()>'),'signature is extracted, never transcribed');
    assert.match(item.detail.docs,/Updated source contract/);
  }
  assert.deepEqual(sourceCatalog(source+'\n// impl dyn SourceEncodingDetails + Unpin {}',surface,details),sourceCatalog(source,surface,details));
});

test('published supplement is additive to unchanged compiler counts and raw surfaces',async()=>{
  const published=JSON.parse(await read('../_site/code-api/source-supplements.json'));
  assert.deepEqual(published,sourceCatalog(source,surface,details));
  const manifest=JSON.parse(await read('../_site/code-api/manifest.json'));
  assert.deepEqual(manifest.source_supplements,{
    file:'code-api/source-supplements.json',count:3,source:'src/detect.rs',
    source_sha256:createHash('sha256').update(source).digest('hex'),
  });
  assert.equal(await read('../_site/code-api/zencodec.txt'),surface);
  const compiler=JSON.parse(await read('../_site/code-api/zencodec.json'));
  assert.equal(manifest.packages.find(pkg=>pkg.name==='zencodec').count,compiler.length);
  const matches=filterCatalog([...compiler,...published],{search:'codec_details'});
  assert.deepEqual(matches.filter(item=>item.provenance),published);
  assert.deepEqual(matches.filter(item=>!item.provenance),filterCatalog(compiler,{search:'codec_details'}),'keep existing documentation matches');
  const html=await read('../_site/code.html');
  assert.ok(html.includes('>Every signature</a>'));
  assert.ok(html.includes('<h2>Every exported signature</h2>'));
  assert.ok(html.includes('three compiler-omitted inherent'));
  assert.ok(html.includes('code-api/source-supplements.json'));
});
