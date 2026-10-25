import { readFile, writeFile, mkdir, copyFile, stat } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { lanes, questions, evidence } from './site/content.mjs';
import { api, chapterApi, entries, supplementalExamples } from './site/api.mjs';
import { declarations, example } from './site/source.mjs';
const docs = path.dirname(fileURLToPath(import.meta.url));
const root = path.dirname(docs);
const dest = path.join(docs, '_site');
const source = await readFile(path.join(docs, 'examples/journeys.rs'), 'utf8');
const policySource = await readFile(path.join(docs, 'examples/policy.rs'), 'utf8');
const sha = process.env.GITHUB_SHA || execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim();
const escape = s => String(s).replaceAll('&','&amp;').replaceAll('<','&lt;').replaceAll('>','&gt;').replaceAll('"','&quot;');
const signatures = new Map();
for (const [key, spec] of Object.entries(api)) {
  signatures.set(key, declarations(await readFile(path.join(root, spec.file), 'utf8'), spec));
}
function highlight(code) {
  const pattern = /\/\/[^\n]*|"(?:[^"\\]|\\.)*"|\b(?:let|mut|fn|pub|use|async|await|impl|for|return|if|else|match|Some|None|Ok|Err)\b/g;
  let out='', end=0;
  for(const m of code.matchAll(pattern)) {
    out += escape(code.slice(end,m.index));
    const kind=m[0].startsWith('//')?'comment':m[0].startsWith('"')?'string':'keyword';
    out += `<span class="syntax-${kind}">${escape(m[0])}</span>`;end=m.index+m[0].length;
  }
  return out + escape(code.slice(end));
}
function codePanel(id, name, file = 'docs/examples/journeys.rs') {
  const proposed = file === 'docs/examples/policy.rs';
  const parts = example(proposed ? policySource : source, name);
  const context = (direction, label) => `<details class="code-context" data-direction="${direction}"><summary>${label}<span>${parts[direction].trimEnd().split('\n').length} lines</span></summary><pre><code data-example-part="${direction}">${highlight(parts[direction])}</code></pre></details>`;
  return `<div class="code-panel" id="${id}-example" data-example="${name}" data-example-source="${file}">
    <div class="code-toolbar"><span>Rust · ${proposed ? 'tested private prototype' : 'tested example'}</span><div class="code-actions"><button type="button" data-copy="${id}-code">Copy focus</button><button type="button" data-copy-full="${id}-example">Copy full example</button></div></div>
    ${context('above', '↑ Expand above · imports &amp; setup')}
    <pre class="code-focus"><code id="${id}-code" data-example-part="focus">${highlight(parts.focus)}</code></pre>
    ${context('below', '↓ Expand below · return &amp; continuation')}
    <div class="code-provenance"><a href="https://github.com/imazen/zencodec/blob/${sha}/${file}#L${parts.line}">Complete source ↗</a><a href="https://github.com/imazen/zencodec/blob/${sha}/perf/io-validation/tests/${proposed ? 'policy_native' : 'site_examples'}.rs">${proposed ? 'Native contract tests' : 'Native pixel-parity tests'} ↗</a><span class="copy-status" role="status" aria-live="polite"></span></div>
  </div>`;
}
function apiPanels(id, keys) {
  if (!keys?.length) throw Error(`No API selection for ${id}`);
  return `<section class="api-section" aria-labelledby="${id}-api-title"><h4 id="${id}-api-title">The API used here</h4><p>Selected declarations from this revision. Bodies are omitted; receivers, lifetimes, associated types and bounds are retained. Expand another interface for its signatures.</p>${keys.map((key, i) => {
    const spec = api[key], signature = signatures.get(key);
    const kind = !spec.scope ? 'Function' : spec.scope.startsWith('impl') ? 'Inherent methods' : 'Trait methods';
    const feature = spec.file === 'zencodec-io/src/tokio_io.rs' ? ' · zencodec-io feature: tokio' : spec.file === 'zencodec-io/src/file.rs' ? ' · zencodec-io feature: std' : '';
    return `<details class="api-panel" data-api="${key}"${i === 0 ? ' open' : ''}><summary><span>${kind}${feature}</span><code>${escape(spec.path)}</code></summary><pre><code>${highlight(signature.code)}</code></pre><a class="api-source" href="https://github.com/imazen/zencodec/blob/${sha}/${spec.file}#L${signature.line}">Read defaults, imports and full contract ↗</a></details>`;
  }).join('')}</section>`;
}
const entryHtml = `<div class="entry-controls"><label for="entry-select">What do you have?</label><select id="entry-select" aria-controls="entry-panels">${entries.map(entry => `<option value="${entry.id}">${escape(entry.label)}</option>`).join('')}</select></div><div id="entry-panels">${entries.map(entry => `<article class="entry-panel consumer" id="entry-${entry.id}" data-entry="${entry.id}"><h3>${escape(entry.label)}</h3><p class="entry-summary">${escape(entry.summary)}</p><p class="example-context"><b>In scope.</b> ${escape(entry.context)}</p>${codePanel(`entry-${entry.id}`, entry.snippet)}${apiPanels(`entry-${entry.id}`, [entry.api])}${entry.id === 'transcode' ? '<p class="entry-issue">Fixture limitation: <a href="https://github.com/imazen/zenbitmaps/issues/28">zenbitmaps #28 — decoded color description</a>.</p>' : ''}</article>`).join('')}</div>`;
function mobileFlow(nodes) {
  return `<div class="graph-mobile">${nodes.map(([role,label,sub],i)=>`${i?'<div class="mobile-arrow" aria-hidden="true">↓</div>':''}<div class="mobile-node ${role}"><span>${role}</span><b>${escape(label)}</b><small>${escape(sub)}</small></div>`).join('')}</div>`;
}
function flow(id,nodes) {
  return `<figure class="flow"><svg viewBox="0 0 1080 150" role="img" aria-labelledby="${id}-title"><title id="${id}-title">${escape(nodes.map(n=>n[1]+': '+n[2]).join(' → '))}</title><defs><marker id="${id}-arrow" viewBox="0 0 10 10" refX="8" refY="5" markerWidth="6" markerHeight="6" orient="auto-start-reverse"><path d="M0 0L10 5L0 10z" fill="currentColor"/></marker></defs>${nodes.map(([role,label,sub],i)=>`<g class="${role}"><rect x="${i*370+1}" y="16" width="338" height="116" rx="12"/><text class="svg-role" x="${i*370+22}" y="43">${role.toUpperCase()}</text><text class="svg-label" x="${i*370+22}" y="77">${escape(label)}</text><text class="svg-sub" x="${i*370+22}" y="105">${escape(sub)}</text></g>${i<2?`<path class="connector" d="M${i*370+340} 74h27" marker-end="url(#${id}-arrow)"/>`:''}`).join('')}</svg>${mobileFlow(nodes)}<figcaption>${escape(nodes.map(n=>n[1]).join(' → '))}</figcaption></figure>`;
}
const architecture = `<figure class="architecture-chart"><svg viewBox="0 0 1120 345" role="img" aria-labelledby="map-title map-desc"><title id="map-title">The consumer, adapter, and implementor boundaries</title><desc id="map-desc">The consumer passes intent to the adapter. The adapter offers bytes and destination rows to the codec implementor, who returns requests, progress and output. Pixels and frames go back to the consumer. Storage lifetime and cancellation cross these boundaries.</desc><defs><marker id="map-arrow" viewBox="0 0 10 10" refX="8" refY="5" markerWidth="6" markerHeight="6" orient="auto"><path d="M0 0L10 5L0 10z" fill="#45566a"/></marker></defs>
<g class="consumer"><rect x="12" y="55" width="300" height="205" rx="16"/><text class="svg-role" x="36" y="87">01 / CONSUMER</text><text class="map-label" x="36" y="124">The application</text><text class="svg-sub" x="36" y="159">Config → job → result</text><text class="svg-sub" x="36" y="190">Pixels · policy · presentation</text><text class="svg-sub" x="36" y="226">Owns the useful output</text></g>
<g class="adapter"><rect x="410" y="55" width="300" height="205" rx="16"/><text class="svg-role" x="434" y="87">02 / ADAPTER</text><text class="map-label" x="434" y="124">The connection</text><text class="svg-sub" x="434" y="159">Source → driver → sink</text><text class="svg-sub" x="434" y="190">Disk · HTTP · compute</text><text class="svg-sub" x="434" y="226">Owns admission &amp; delivery</text></g>
<g class="implementor"><rect x="808" y="55" width="300" height="205" rx="16"/><text class="svg-role" x="832" y="87">03 / IMPLEMENTOR</text><text class="map-label" x="832" y="124">The codec</text><text class="svg-sub" x="832" y="159">step · rows · send / receive</text><text class="svg-sub" x="832" y="190">State · kernels · capabilities</text><text class="svg-sub" x="832" y="226">Owns exact codec behavior</text></g>
<g class="map-arrows"><path d="M315 132H405"/><text x="360" y="120">intent</text><path d="M713 132H803"/><text x="758" y="120">bytes</text><path d="M803 207H715"/><text x="758" y="196">requests</text><path d="M405 207H316"/><text x="360" y="196">result</text></g><path d="M162 267v40h796v-40" fill="none" stroke="#9ea7ab" stroke-dasharray="4 5"/><text class="svg-sub" x="560" y="335" text-anchor="middle">Cancellation, identity, and storage lifetime remain visible across the whole path.</text></svg>${mobileFlow([['consumer','The application','Intent: pixels, policy, presentation'],['adapter','The connection','Bytes, requests, budgets, delivery'],['implementor','The codec','State, kernels, exact results']])}<figcaption>Architecture diagram · colors identify responsibility, not separate runtimes or mandatory allocations.</figcaption></figure>`;
const ownership = `<figure class="ownership-chart"><svg viewBox="0 0 1100 430" role="img" aria-labelledby="wait-title wait-desc"><title id="wait-title">Choose where output waits from storage ownership</title><desc id="wait-desc">Start by checking whether this configuration has bounded raw and intermediate state and whether upstream input can pause. If yes, use a bounded output queue and wait. If no, admit encoded staging, finish the encoder to release raw state, then wait on the writer. If staging cannot be admitted, queue or refuse the job before starting.</desc><defs><marker id="wait-arrow" viewBox="0 0 10 10" refX="8" refY="5" markerWidth="6" markerHeight="6" orient="auto"><path d="M0 0L10 5L0 10z" fill="#45566a"/></marker></defs><g class="implementor"><path class="decision" d="M550 12L790 105L550 198L310 105Z"/><text class="svg-label" x="550" y="88" text-anchor="middle">Bounded raw state?</text><text class="svg-sub" x="550" y="116" text-anchor="middle">Upstream can pause?</text><text class="svg-role" x="550" y="144" text-anchor="middle">VERIFY THIS CONFIGURATION</text></g><path class="decision-arrow" d="M310 105H172V225"/><text class="svg-label" x="216" y="87">Yes</text><path class="decision-arrow" d="M790 105H928V225"/><text class="svg-label" x="829" y="87">No / unknown</text><g class="adapter"><rect x="15" y="232" width="315" height="100" rx="12"/><text class="svg-label" x="36" y="268">Bound the output queue</text><text class="svg-sub" x="36" y="298">Wait without accumulating raw input</text><rect x="770" y="232" width="315" height="100" rx="12"/><text class="svg-label" x="791" y="268">Admit encoded staging</text><text class="svg-sub" x="791" y="298">Finish → release raw → write</text></g><text class="svg-sub" x="928" y="366" text-anchor="middle">No capacity? Queue or refuse before starting.</text><path d="M172 344v46h756v-12" stroke="#9ea7ab" fill="none" stroke-dasharray="4 5"/><text class="svg-sub" x="550" y="420" text-anchor="middle">Both paths still need CPU fairness, cancellation, and a bound on admitted jobs.</text></svg><div class="graph-mobile mobile-decision"><div class="mobile-node implementor"><span>Decision / this configuration</span><b>Bounded raw state?<br>Upstream can pause?</b></div><p>Choose one path:</p><div class="mobile-node adapter"><span>If yes / sequential path</span><b>Bound the output queue</b><small>Wait without accumulating raw input.</small></div><div class="mobile-node adapter"><span>If no or unknown / buffered path</span><b>Admit encoded staging</b><small>Finish → release raw → write.<br>No capacity? Queue or refuse before starting.</small></div><p>Both paths need CPU fairness, cancellation, and a bound on admitted jobs.</p></div><figcaption>Output policy flowchart · proposed scheduling distinction, not a new public capability flag.</figcaption></figure>`;
function chapter(lane,step,i) {
  const id=`${lane.id}-${i+1}`;
  return `<article class="chapter ${lane.id}" id="${id}" data-lane="${lane.id}" tabindex="-1" aria-labelledby="${id}-heading"><div class="chapter-top"><span class="role-label">${lane.name} / ${String(i+1).padStart(2,'0')}</span><span class="status">${step.snippet?'Executable example':'Current contract'}</span></div><h3 id="${id}-heading">${escape(step.title)}</h3><p class="trigger"><span>New requirement</span>${escape(step.trigger)}</p><p class="chapter-body">${escape(step.body)}</p>${step.snippet ? codePanel(id, step.snippet) : ''}${(supplementalExamples[id] || []).map((extra, n) => `<div class="supplemental-example"><h4>${escape(extra.title)}</h4><p class="example-context">${escape(extra.note)}</p>${codePanel(`${id}-extra-${n}`, extra.snippet)}</div>`).join('')}${apiPanels(id, chapterApi[id])}<div class="why"><span>Why add this?</span><p>${escape(step.why)}</p></div>${flow(id,step.nodes)}<aside class="honesty"><span class="note-label">Contract boundary</span><p>${escape(step.note)}</p><a href="https://github.com/imazen/zencodec/blob/main/${step.source}">Inspect the source / evidence ↗</a>${step.issue?`<a href="https://github.com/imazen/zencodec/issues/${step.issue}">Open question #${step.issue} ↗</a>`:''}</aside><div class="chapter-bottom"><span>${i+1} of ${lane.steps.length} in this journey</span><a href="#${i+1<lane.steps.length?`${lane.id}-${i+2}`:lanes[(lanes.indexOf(lane)+1)%lanes.length].id+'-1'}">${i+1<lane.steps.length?'Next: '+escape(lane.steps[i+1].title):'Explore another perspective'} <span aria-hidden="true">→</span></a></div></article>`;
}
const journeyHtml = `<nav class="lane-tabs" aria-label="Audience">${lanes.map(l=>`<a class="lane-tab ${l.id}" data-lane-link="${l.id}" href="#${l.id}-1"><span>${l.number} / ${l.name}</span><strong>${l.title}</strong><small>${l.scope}</small></a>`).join('')}</nav><div class="journey-layout"><aside class="journey-nav" aria-label="Chapter navigation">${lanes.map(l=>`<div class="steps-group" data-lane="${l.id}"><p class="journey-intro">${l.intro}</p><ol>${l.steps.map((s,i)=>`<li><a href="#${l.id}-${i+1}" class="step-link"><span>${String(i+1).padStart(2,'0')}</span>${escape(s.title)}</a></li>`).join('')}</ol><a class="small-link" href="#questions">See unresolved design questions ↘</a></div>`).join('')}</aside><div class="chapter-container">${lanes.flatMap(l=>l.steps.map((s,i)=>chapter(l,s,i))).join('')}</div></div>`;
for (const lane of lanes) for (const step of lane.steps) await stat(path.join(root,step.source));
for (const row of evidence) await stat(path.join(root,row[3]));
const policyHtml = `
${flow('policy-admission',[['consumer','Stored request','required effects + preferences'],['implementor','Operation facts','qualified alternatives'],['adapter','Check requirements','offer feasible choices']])}
<div class="callout"><b>No feasible choice?</b> Refuse with the failed requirements before calling application code. Estimates cannot certify a hard cap.</div>
<h3>1. The gallery needs reliable metadata handling.</h3>
<p>A profile binds requirements and preferences once. The consumer makes one call. The native QOI fixture refuses supplied rights retention because it has no EXIF carrier; missing metadata is not invented. Both concrete and CodecSet paths use the same stored request.</p>
${codePanel('policy-default','policy_default','docs/examples/policy.rs')}
<h3>2. Publication gains an account-specific rule.</h3>
<p>Add one callback when the application has a conditional decision. A missing account rule returns its own traced error; a disabled account deliberately declines; enabled accounts delegate to the stored preset. None of these answers can weaken required effects.</p>
${codePanel('policy-custom','policy_custom','docs/examples/policy.rs')}
${apiPanels('policy',['policyResolver','policyRequest'])}
${flow('policy-execution',[['consumer','Choose / delegate','or traced refusal / error'],['adapter','Reserve + execute','recheck source and capacity'],['consumer','Result + decisions','cause, trace, partial progress']])}
<p>Resolve at operation or configuration-epoch boundaries. Writer stalls and individual rows or packets do not re-enter the callback. Native codec and transport errors keep their own causes. A later failure retains earlier selections.</p>
<p><a href="https://github.com/imazen/zencodec/blob/main/docs/policy-resolution.md">Proposed contract and API removal candidates ↗</a> · <a href="https://github.com/imazen/zencodec/blob/main/docs/policy-scenarios.md">Scenario matrix and qualification gaps ↗</a> · <a href="https://github.com/imazen/zencodec/blob/main/benchmarks/policy_cost_2026-10-25.md">Measured dispatch and allocation costs ↗</a></p>`;
let html=await readFile(path.join(docs,'site/index.html'),'utf8');
html=html.replace('<!-- ENTRYPOINTS -->',entryHtml).replace('<!-- ARCHITECTURE -->',architecture).replace('<!-- JOURNEYS -->',journeyHtml).replace('<!-- OWNERSHIP -->',ownership).replace('<!-- POLICY -->',policyHtml)
.replace('<!-- EVIDENCE -->',`<div class="evidence-grid">${evidence.map(([tag,title,text,url])=>`<a class="evidence-item" href="https://github.com/imazen/zencodec/blob/main/${url}"><span class="evidence-tag">${tag}</span><h3>${title} <span aria-hidden="true">↗</span></h3><p>${text}</p></a>`).join('')}</div>`)
.replace('<!-- QUESTIONS -->',`<div class="question-list">${questions.map(q=>`<a href="https://github.com/imazen/zencodec/issues/${q.n}"><span class="issue-number">#${q.n}</span><div><h3>${q.title}</h3><p>${q.text}</p></div><span aria-hidden="true">↗</span></a>`).join('')}</div>`).replaceAll('BUILD_SHA',sha).replaceAll('BUILD_SHORT',sha.slice(0,8));
await mkdir(dest,{recursive:true});
await writeFile(path.join(dest,'index.html'),html);
for(const file of ['styles.css','app.mjs']) await copyFile(path.join(docs,'site',file),path.join(dest,file));
await copyFile(path.join(docs,'examples/journeys.rs'),path.join(dest,'journeys.rs'));
await copyFile(path.join(root,'benchmarks/io_transport_model_2026-10-25.csv'),path.join(dest,'transport.csv'));
await writeFile(path.join(dest,'.nojekyll'),'');
await writeFile(path.join(dest,'build.json'),JSON.stringify({revision:sha,chapters:lanes.reduce((n,l)=>n+l.steps.length,0),source:'docs/'})+'\n');
console.log(`Built docs/_site from ${sha.slice(0,8)}; 15 chapters with checked source links.`);
