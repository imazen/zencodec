import { readFile, writeFile, readdir, mkdir, stat } from 'node:fs/promises';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { example, maskRust } from './source.mjs';
import { highlight, documentation as renderDocumentation } from './highlight.mjs';
import { sourceCatalog } from './source-catalog.mjs';
import { codeFlows, roleStories } from './code-first-data.mjs';
import { api } from './api.mjs';
const esc = s => String(s).replaceAll('&','&amp;').replaceAll('<','&lt;').replaceAll('>','&gt;').replaceAll('"','&quot;');
const digest = text => createHash('sha256').update(text).digest('hex');
const roles = ['consumer','adapter','implementor'];
const labels = {consumer:'Consumer',adapter:'Adapter',implementor:'Codec implementor'};

export function implementation(source, scope) {
  const mask = maskRust(source), start = mask.indexOf(scope);
  if (start < 0 || mask.indexOf(scope,start+scope.length) >= 0) throw Error(`Scope must match once: ${scope}`);
  const open = mask.indexOf('{',start);
  let depth=1,end=open+1;
  while(depth && end<mask.length) {if(mask[end]==='{')depth++;if(mask[end]==='}')depth--;end++;}
  if(depth)throw Error(`Unclosed implementation: ${scope}`);
  return {above:source.slice(0,start),focus:source.slice(start,end),below:source.slice(end),full:source,line:source.slice(0,start).split('\n').length};
}
async function rustFiles(dir) {
  const entries=await readdir(dir,{withFileTypes:true});
  const result=[];
  for(const entry of entries) {
    const file=path.join(dir,entry.name);
    if(entry.isDirectory())result.push(...await rustFiles(file));
    else if(file.endsWith('.rs'))result.push(file);
  }
  return result.sort();
}
// Keep symbol origins visible without opening the whole fixture's setup.
export function exampleImports(above) {
  const masked=maskRust(above);
  return [...masked.matchAll(/^[ \t]*(?:pub\s+)?use\s+[^;]+;/gm)]
    .map(match=>above.slice(match.index,match.index+match[0].length).trim())
    .join('\n');
}
export async function sourceFingerprint(root,directory) {
  const base=path.join(root,directory),hash=createHash('sha256');
  const files=[path.join(base,'Cargo.toml'),...await rustFiles(path.join(base,'src'))];
  const readme=path.join(base,'README.md');
  try {if((await stat(readme)).isFile())files.push(readme);}
  catch(error) {if(error.code!=='ENOENT')throw error;}
  for(const file of files) {
    hash.update(path.relative(root,file)+'\0');hash.update(await readFile(file));hash.update('\0');
  }
  return hash.digest('hex');
}
export function catalogItems(surface,details) {
  const byPath=new Map(details.map(d=>[d.path,d]));
  const aliases=new Map();
  for(const detail of details) {
    const name=detail.path.split('::').slice(-2).join('::');
    if(!aliases.has(name))aliases.set(name,detail);
    else if(aliases.get(name)?.file!==detail.file || aliases.get(name)?.line!==detail.line)aliases.set(name,null);
  }
  let context='';const seen=new Set(),items=[];
  for(const signature of surface.trim().split('\n')) {
    if(/^(impl[ <]|pub (struct|enum|trait|mod) )/.test(signature))context=signature;
    const key=context+'\n'+signature;
    if(seen.has(key))continue;seen.add(key);
    let plain='',depth=0;
    for(const c of signature){if(c==='<')depth++;else if(c==='>'&&depth)depth--;else if(!depth)plain+=c;}
    const qualified=plain.match(/(?:zencodec(?:_[a-z_]+)?|zenmeta)::[\w:]+/)?.[0]?.replace(/::$/,'');
    const detail=byPath.get(qualified)||aliases.get(qualified?.split('::').slice(-2).join('::'))||null;
    const kind=/\bfn /.test(signature)?'function':/^pub trait /.test(signature)?'trait':/^impl[ <]/.test(signature)?'implementation':/^pub (struct|enum|type|mod) /.test(signature)?'type':'value';
    const flows=detail?codeFlows.filter(flow=>flow.steps.some(step=>step.api?.some(name=>api[name]?.file===detail.file))).map(flow=>({id:flow.id,title:flow.title})):[];
    const documentation=renderDocumentation(detail?.docs||'');
    items.push({id:digest(key).slice(0,16),signature,context,kind,detail,flows,documentation,html:highlight(signature)});
  }
  return items;
}
export async function codeFirstPage({sha,apiPanels,docs,dest}) {
  const root=path.dirname(docs),manifest=JSON.parse(await readFile(path.join(docs,'code-api/manifest.json'),'utf8'));
  await mkdir(path.join(dest,'code-api'),{recursive:true});
  let supplements;
  for(const pkg of manifest.packages) {
    if(await sourceFingerprint(root,pkg.directory)!==pkg.fingerprint)throw Error(`Stale code catalog: ${pkg.name}; regenerate with just code-catalog EXPORT`);
    const surface=await readFile(path.join(docs,`code-api/${pkg.name}.txt`),'utf8');
    if(digest(surface)!==pkg.surface_sha256)throw Error(`Modified code catalog: ${pkg.name}`);
    const detailsText=await readFile(path.join(docs,`code-api/${pkg.name}.json`),'utf8');
    if(digest(detailsText)!==pkg.details_sha256)throw Error(`Modified source documentation: ${pkg.name}`);
    const details=JSON.parse(detailsText);
    const items=catalogItems(surface,details);pkg.count=items.length;
    if(pkg.name==='zencodec') {
      const source=await readFile(path.join(root,'src/detect.rs'),'utf8');
      supplements=sourceCatalog(source,surface,details);
      manifest.source_supplements={file:'code-api/source-supplements.json',count:supplements.length,source:'src/detect.rs',source_sha256:digest(source)};
      await writeFile(path.join(dest,manifest.source_supplements.file),JSON.stringify(supplements));
    }
    await writeFile(path.join(dest,`code-api/${pkg.name}.json`),JSON.stringify(items));
    await writeFile(path.join(dest,`code-api/${pkg.name}.txt`),surface);
  }
  if(!supplements)throw Error('Missing zencodec package for source supplement');
  await writeFile(path.join(dest,'code-api/manifest.json'),JSON.stringify({...manifest,sha}));
  const sources=new Map();
  async function externalPanels(files=[]) {
    const panels=[];
    for(const file of files) {
      const snapshot=JSON.parse(await readFile(path.join(docs,file),'utf8'));
      if(digest(snapshot.surface)!==snapshot.sha256)throw Error(`Modified dependency snapshot: ${file}`);
      panels.push(`<details class="api-panel"><summary>Pinned dependency · ${esc(snapshot.repository)}</summary><pre><code>${highlight(snapshot.surface)}</code></pre><a class="api-source" href="https://github.com/${snapshot.repository}/blob/${snapshot.revision}/${snapshot.path}">Source at ${snapshot.revision.slice(0,8)} ↗</a></details>`);
    }
    return panels.join('');
  }
  async function panel(step,id) {
    if(!sources.has(step.file))sources.set(step.file,await readFile(path.join(root,step.file),'utf8'));
    const source=sources.get(step.file),parts=step.example?example(source,step.example):implementation(source,step.scope);
    const imports=exampleImports(parts.above);
    const origins=imports?`<div class="code-imports"><small>Imports used by this source</small><pre><code>${highlight(imports)}</code></pre></div>`:'';
    const context=(key,label)=>`<details class="code-context" data-direction="${key}"><summary>${label}<span>${parts[key].split('\n').length} lines</span></summary><pre><code data-example-part="${key}">${highlight(parts[key])}</code></pre></details>`;
    return `<div class="code-panel" id="${id}" data-example-source="${esc(step.file)}" data-example="${esc(step.example||'')}" data-scope="${esc(step.scope||'')}"><div class="code-toolbar"><span>${esc(step.state||'Implemented · review API')}</span><div class="code-actions"><a href="https://github.com/imazen/zencodec/blob/${sha}/${step.file}#L${parts.line}">Source ↗</a><button data-copy-full="${id}">Copy ${step.example?'full example':'source file'}</button></div></div>${origins}${context('above',step.example?'Show setup above ↑':'Show preceding definitions ↑')}<pre class="code-focus"><code data-example-part="focus">${highlight(parts.focus)}</code></pre>${context('below',step.example?'Show continuation below ↓':'Show following definitions ↓')}<span class="copy-status sr-only" role="status"></span></div><details class="cf-contract"><summary>Signatures & related contracts</summary>${step.api?.length?apiPanels(id,step.api):''}${await externalPanels(step.external)}</details>`;
  }
  const rendered=new Map();
  for(const flow of codeFlows) {
    let project='';
    if(flow.manifest) {
      await readFile(path.join(root,flow.manifest));
      project=`<aside class="cf-project"><a href="https://github.com/imazen/zencodec/blob/${sha}/${flow.manifest}">Native example manifest, dependency pins &amp; features ↗</a><p>This is the qualification harness configuration. Use the source revision shown on this page and retain its dependency patches.</p></aside>`;
    }
    if(flow.project) {
      const setup=flow.project;
      await readFile(path.join(root,setup.manifest));
      await readFile(path.join(root,setup.helpers));
      project=`<aside class="cf-project"><a href="https://github.com/imazen/zencodec/blob/${sha}/${setup.manifest}">Project manifest & dependencies ↗</a> · <a href="https://github.com/imazen/zencodec/blob/${sha}/${setup.helpers}">Shared fixture helpers ↗</a><details><summary>Build and run in this revision's repository checkout</summary><pre><code>${esc(setup.command)}</code></pre><p>The implementation is an internal test fixture. Run its workspace tests with the helpers present; copying the implementation block alone is not a standalone codec project.</p></details></aside>`;
    }
    const steps=[];for(const [i,step] of flow.steps.entries())steps.push(`<details class="cf-step"${i===0?' open':''}><summary><span class="cf-step-number">${i+1}</span><span>${esc(step.title)}</span><span class="cf-step-hint">${i===0?'Start here':'Add this need'}</span></summary><p class="cf-why">${esc(step.why)}</p>${await panel(step,`${flow.id}-${i}`)}</details>`);
    const siblings=codeFlows.filter(f=>f.role===flow.role),next=siblings[siblings.indexOf(flow)+1];
    rendered.set(flow.id,`<article class="cf-flow ${flow.role}" id="${flow.id}" data-flow="${flow.id}"><h2>${esc(flow.title)}</h2>${project}${flow.note?`<aside class="cf-note">${esc(flow.note)} <a href="https://github.com/imazen/zencodec/issues/${flow.issue||173}">Tracked contract gap ↗</a>${flow.evidence?` · <a href="https://github.com/imazen/zencodec/blob/${sha}/${flow.evidence}">Measurements & contract ↗</a>`:''}</aside>`:''}${steps.join('')}<nav class="cf-next" aria-label="Continue this story">${next?`<a href="#${next.id}">Next: ${esc(next.title)} →</a>`:`<a href="#catalog">Look up a signature →</a>`}</nav></article>`);
  }
  return `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Code first — zencodec</title><link rel="icon" href="data:,"><link rel="stylesheet" href="styles.css"><link rel="stylesheet" href="code-first.css"><script type="module" src="code-first-ui.mjs"></script></head><body class="code-first"><a class="skip" href="#cf-main">Skip to code</a><header class="cf-header"><a class="brand" href="index.html">zen<span>codec</span></a><nav><a href="ideal.html">API decisions</a><a href="gaps.html">Remaining gaps</a><a href="#catalog">Every signature</a></nav></header><main id="cf-main"><div class="cf-intro"><h1>zencodec, in code.</h1><p>Start with a working call. Add one real requirement at a time. Expand imports, setup and continuation inline.</p><small>Review branch · <a href="https://github.com/imazen/zencodec/pull/168">PR 168</a> · ${sha.slice(0,8)} · examples are extracted from source, not pseudocode.</small><p class="cf-note">Pulse migration: core interfaces and workspace checks are updated. Native integrations and pinned dependency examples are awaiting migration and requalification. <a href="https://github.com/imazen/zencodec/blob/${sha}/docs/pulse-migration.md">Migration contract ↗</a></p></div><div class="cf-tabs" role="tablist" aria-label="Your side of the API">${roles.map((role,i)=>`<button class="${role}" role="tab" id="tab-${role}" aria-controls="role-${role}" aria-selected="${i===0}" tabindex="${i===0?0:-1}" data-role-tab="${role}">${labels[role]}</button>`).join('')}</div>${roles.map((role,i)=>`<section role="tabpanel" id="role-${role}" aria-labelledby="tab-${role}"${i?' hidden':''}><p class="cf-story">${esc(roleStories[role])}</p><div class="cf-layout"><nav class="cf-menu" aria-label="${labels[role]} flows">${codeFlows.filter(f=>f.role===role).map(f=>`<a href="#${f.id}">${esc(f.title)}</a>`).join('')}<a href="#catalog">Search all signatures ↘</a></nav><div class="cf-content">${codeFlows.filter(f=>f.role===role).map(f=>rendered.get(f.id)).join('')}</div></div></section>`).join('')}<section id="catalog" class="cf-catalog"><h2>Every exported signature</h2><p>All features on the generation host (Linux), across eight crates. Canonical compiler paths; blanket, auto and derived implementations omitted. Private experiments and dependency-only APIs are outside this index. A signature is not a claim of native codec support.</p><p>The search also includes three compiler-omitted inherent <code>dyn SourceEncodingDetails</code> methods, extracted from source with their documentation and links. Crate counts and raw API files remain compiler-derived. <a href="code-api/source-supplements.json">Source-backed supplements</a>.</p><div class="cf-filters"><label>Crate<select id="cf-package"><option value="all">All eight crates</option>${manifest.packages.map(p=>`<option value="${p.name}">${p.name} (${p.count})</option>`).join('')}</select></label><label class="cf-search-label">Name, signature or contract<input id="cf-search" type="search" placeholder="e.g. EncodeRequest encode"></label><label>Kind<select id="cf-kind"><option value="all">Everything</option>${['function','trait','type','value','implementation'].map(k=>`<option>${k}</option>`).join('')}</select></label><label class="cf-checkbox"><input type="checkbox" id="cf-legacy" checked> Include deprecated</label></div><p id="cf-count" role="status">Loading signatures…</p><div id="cf-results"></div><button id="cf-more" hidden>Show 40 more</button><details class="cf-scope"><summary>Coverage, source and regeneration</summary><p>The flows above cover concrete uses; the index covers the compiler-derived exported surface. Open each contract for its source documentation. Items without local docs link to their crate source. Third-party APIs used by examples, including zenpixels-convert, retain their original imports and source links.</p><code>just code-catalog /path/to/prepared/source-export</code><p>The build rejects a catalog whose Rust sources or manifest changed. Raw API files: ${manifest.packages.map(p=>`<a href="code-api/${p.name}.txt">${p.name}</a>`).join(' · ')}.</p></details></section></main><footer>Source in docs/ · <a href="https://github.com/imazen/zencodec/commit/${sha}">${sha.slice(0,8)}</a> · <a href="ideal.html">Design and validation evidence</a></footer></body></html>`;
}
