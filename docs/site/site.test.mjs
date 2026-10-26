import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile, stat } from 'node:fs/promises';
import { lanes } from './content.mjs';
import { migrations } from './migration-data.mjs';
import { improvements } from './improvements.mjs';
const root=new URL('../_site/',import.meta.url);
const html=await readFile(new URL('index.html',root),'utf8');
test('all chapter anchors, diagrams, and Rust examples survive the build',()=>{
  const ids=[...html.matchAll(/\bid="([^"]+)"/g)].map(m=>m[1]);
  assert.equal(new Set(ids).size,ids.length,'duplicate IDs break navigation and accessible SVG titles');
  assert.equal(lanes.length,3);
  for(const lane of lanes) for(let i=0;i<lane.steps.length;i++) {
    const id=`${lane.id}-${i+1}`;
    assert.ok(ids.includes(id));assert.ok(ids.includes(`${id}-title`));
    if(lane.steps[i].snippet) assert.ok(ids.includes(`${id}-code`));
  }
  for(const [,anchor] of html.matchAll(/href="#([^"]+)"/g)) assert.ok(ids.includes(anchor),anchor);
  assert.equal((html.match(/class="chapter /g)||[]).length,15);
  assert.ok(!html.includes('BUILD_SHA'));assert.ok(!html.includes('<!-- JOURNEYS -->'));
  assert.ok(!html.includes('<!-- POLICY -->'));
  assert.ok(html.includes('data-api="policyResolver"'));
  // Grammar highlighting can split the type into several colored spans.
  assert.ok(html.replace(/<\/?span\b[^>]*>/g, '').includes('At&lt;Self::Error&gt;'));
  assert.ok(html.includes('data-example="policy_default"'));
  assert.ok(html.includes('data-example="policy_custom"'));
  for(const name of ['jpeg_rewrite','jpeg_icc_refusal','jpeg_icc_admission']) assert.ok(html.includes(`data-example="${name}"`));
  assert.ok(html.includes('data-api="policyGate"'));
  assert.ok(html.includes('policy-discovery-title'));
});
test('deployment only needs local allowlisted assets',async()=>{
  for(const [,url] of html.matchAll(/(?:href|src)="([^"]+)"/g)) {
    if(url.startsWith('#')||url.startsWith('https://')||url==='data:,') continue;
    assert.ok(!url.includes('..'),url);
    assert.ok((await stat(new URL(url,root))).isFile(),url);
  }
  const script=await readFile(new URL('app.mjs',root),'utf8');
  assert.ok(!script.includes('eval('));assert.ok(!script.includes('innerHTML'));
});
test('lab uses the exact committed matrix, with no invented timing interpolation',async()=>{
  const csv=await readFile(new URL('transport.csv',root),'utf8');
  const committed=await readFile(new URL('../../benchmarks/io_transport_model_2026-10-25.csv',import.meta.url),'utf8');
  assert.equal(csv,committed);
  const rows=csv.trim().split('\n').slice(1).map(l=>l.split(','));
  assert.equal(rows.length,32);
  assert.equal(new Set(rows.map(r=>r.slice(0,4).join(','))).size,32);
  assert.ok(rows.some(r=>r.join(',')==='budgeted,160,0,8,650.48576,131072,0,2'));
});

test('unlisted migration map separates release states and deploys with the guide',async()=>{
  const page=await readFile(new URL('migration.html',root),'utf8');
  assert.ok(!/href="[^"\n]*migration\.html/.test(html),'migration page must stay out of guide navigation');
  assert.match(page,/<meta name="robots" content="noindex, nofollow">/);
  const ids=[...page.matchAll(/\bid="([^"]+)"/g)].map(m=>m[1]);
  assert.equal(new Set(ids).size,ids.length);
  for(const migration of migrations) {
    assert.ok(ids.includes(migration.id));
    await stat(new URL(`../../${migration.source}`,import.meta.url));
  }
  for(const stage of ['released','current','proposed']) {
    assert.equal((page.match(new RegExp(`data-stage="${stage}"`,'g'))||[]).length,migrations.length);
  }
  for(const [,url] of page.matchAll(/(?:href|src)="([^"]+)"/g)) {
    if(url.startsWith('#')){assert.ok(ids.includes(url.slice(1)),url);continue;}
    if(url.startsWith('https://')||url==='data:,')continue;
    assert.ok(!url.includes('..'),url);
    assert.ok((await stat(new URL(url,root))).isFile(),url);
  }
  const baseline=JSON.parse(await readFile(new URL('migration-baseline.json',root),'utf8'));
  assert.equal(baseline.packages[0].version,'0.1.26');
  assert.ok(page.includes('prototype::PolicyResolver (private)'));
  assert.ok(page.includes('data-example="convert_display"'));
  assert.ok(page.includes('data-example="convert_framebuffer"'));
});

test('checked API page links executable examples and actual signatures for each role',async()=>{
  const page=await readFile(new URL('ideal.html',root),'utf8');
  assert.ok(html.includes('href="ideal.html"'));
  const ids=[...page.matchAll(/\bid="([^"]+)"/g)].map(m=>m[1]);
  assert.equal(new Set(ids).size,ids.length);
  for(const role of ['consumer','adapter','implementor','target'])assert.ok(ids.includes(role));
  for(const [,url] of page.matchAll(/(?:href|src)="([^"]+)"/g)) {
    if(url.startsWith('#')){assert.ok(ids.includes(url.slice(1)),url);continue;}
    if(url.startsWith('https://')||url==='data:,')continue;
    assert.ok(!url.includes('..'),url);
    assert.ok((await stat(new URL(url,root))).isFile(),url);
  }
  assert.ok(page.includes('docs/examples/checked_operations.rs'));
  assert.ok(page.includes('perf/jpeg-policy-validation/src/checked_examples.rs'));
  assert.ok(page.includes('data-api="operationResolver"'));
  assert.ok(page.includes('data-api="operationControl"'));
  assert.ok(page.includes('qualification gaps'));
  assert.ok(!page.includes('prototype::PolicyResolver'));
  assert.ok(page.includes('data-example="checked_resolution"'));
  assert.ok(page.includes('data-example="checked_video_storage"'));
  assert.ok(html.includes('ideal.html#changes'));
  for(const entry of improvements) {
    assert.ok(ids.includes(`change-${entry.id}`));
    for(const field of ['why','consumer','adapter','implementor','open']) assert.ok(entry[field]?.length,`${entry.id}: ${field}`);
    assert.ok(entry.signatures.length>0);
    assert.ok(entry.evidence.length>0);
    for(const [,target] of entry.evidence) if(!target.startsWith('https://')) await stat(new URL(`../../${target}`,import.meta.url));
  }
});
