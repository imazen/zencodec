#!/usr/bin/env python3
"""A filterable HTML view of the bounded route receipt; no additional measurements."""
import argparse
from html import escape
from html.parser import HTMLParser
import json
from pathlib import Path

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('receipt',type=Path)
p.add_argument('output',type=Path)
a = p.parse_args()
r = json.loads(a.receipt.read_text())
rows = []
for codec,size,arm,n,w,c,k,sw,aw,ac,loop in r['cells']:
    group = 'older' if arm.startswith('older_') else 'new' if arm.startswith('next_') else 'base'
    vals = [codec, str(size)+'²', arm, str(n), f'{w/1000:.3f}',f'{c/1000:.3f}',f'{k/1000:.3f}',f'{sw:.3f}',f'{aw/1000:.3f} / {ac/1000:.3f}']
    rows.append(f'<tr data-codec="{codec}" data-size="{size}" class="{group}">' + ''.join('<td>'+escape(v)+'</td>' for v in vals)+'</tr>')
html = '''<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>zencodec · optional Pulse and large-image routes</title>
<style>
:root{font:16px/1.55 system-ui,sans-serif;color:#dee6ed;background:#10181f;color-scheme:dark}body{max-width:1160px;margin:auto;padding:32px 20px 80px}h1{line-height:1.15;font-size:2.4rem;max-width:900px}h2{margin-top:2.5rem}p{max-width:900px}a{color:#85c9ff}code,pre{font:14px/1.55 ui-monospace,monospace}pre{overflow:auto;padding:18px;background:#1a2833;border-radius:8px}ins{color:#a0e7a2;text-decoration:none;background:#193b28}.cols{display:grid;grid-template-columns:1fr 1fr;gap:16px}.note{padding:16px 20px;border-left:4px solid #e5b677;background:#2b281f}.controls{display:flex;gap:20px;align-items:center;flex-wrap:wrap;margin:24px 0}select{padding:8px}.scroll{overflow:auto}table{border-collapse:collapse;width:100%;font-size:14px}th,td{padding:9px 11px;text-align:right;white-space:nowrap;border-bottom:1px solid #314451}th{background:#20303c}td:nth-child(-n+3),th:nth-child(-n+3){text-align:left}tr.new td:first-child{border-left:3px solid #74d79c}tr.older td:first-child{border-left:3px solid #f1bf74}tr.base td:first-child{border-left:3px solid #8ca9c0}tr:hover{background:#1b2e3b}.muted{color:#b1c0cc;font-size:14px}footer{margin-top:40px;overflow-wrap:anywhere}@media(max-width:700px){.cols{grid-template-columns:1fr}h1{font-size:1.9rem}}
</style>
<body><p class="muted">ZENCODEC · DESIGN REVIEW · 2026-11-12</p><h1>Optional control, borrowed input,<br>and the cost of each decode route</h1>
<p>Claude’s preserved work is continued on <a href="https://github.com/imazen/zencodec/pull/211">PR 211</a>. The ordinary path borrows input and accepts no observer. Worker paths can take an existing input owner. Exact pixel checks passed before these measurements.</p>
<div class="cols"><div><h2>Older erased slice call</h2><pre>config.dyn_job()
    .into_decoder(Cow::Borrowed(bytes), &amp;[])?
    .decode()?</pre><p class="muted">Captured older API and native adapter, compiled into the comparison. The bitmap adapter includes an output copy.</p></div><div><h2>Reimagined convenience call</h2><pre>zencodec_io::decode(
    codec, bytes, &amp;DecodeSpec::new(), <ins>None</ins>
)?.into_still()</pre><p class="muted">The input stays borrowed. <code>Some(&amp;pulse)</code> adds reporting and cancellation. The result’s first packed frame is optional. The measured <code>next_slice</code> uses a minimal owned-frame sink; this convenience wrapper also collects frames and clones info, and its total overhead is not timed here.</p></div></div>
<pre>Decoder::step(input, sink, pulse: <ins>Option&lt;&amp;dyn Pulse&gt;</ins>)
Encoder::step(input, pulse: <ins>Option&lt;&amp;dyn Pulse&gt;</ins>)
worker drivers: pulse: <ins>Option&lt;SharedPulse&gt;</ins></pre>
<p>The non-shareable borrowed observer control reaches native checks and JPEG reporting. <code>None</code> creates no shared observer. Decoders still require <code>Send + 'static</code>: retaining a source borrow inside the decoder, and thread-bound codecs, remain outside this contract.</p>
<h2>4096² and 8192² measurements</h2>
<p>All 670 pairs from five shuffled blocks are retained. Values are arithmetic means of baseline-subtracted processes; each process loads the same fixture and warms one decode. <strong>Times are milliseconds per decode.</strong> A/A is the maximum same-block difference between identical native/direct controls, independently for wall and CPU. Differences below that spread are unresolved.</p>
<p>Green edge: reimagined route. Amber: older route. Gray: native/direct/control. These colors identify API families and do not rank performance.</p>
<div class="controls"><label>Codec <select id="codec"><option value="qoi">QOI</option><option value="bmp">BMP</option><option value="jpeg">JPEG</option><option value="syn1">Synthetic RGBA8</option><option value="">All</option></select></label><label>Size <select id="size"><option value="8192">8192²</option><option value="4096">4096²</option><option value="">Both</option></select></label><span id="shown" aria-live="polite"></span></div>
<div class="scroll"><table><thead><tr><th>Codec</th><th>Size</th><th>Route</th><th>N / process</th><th>Wall ms</th><th>CPU ms</th><th>Kernel ms</th><th>Switches</th><th>A/A wall / CPU ms</th></tr></thead><tbody>ROWS</tbody></table></div>
<h2>What this establishes</h2><p>QOI, BMP and baseline JPEG retain exact samples, dimensions and descriptors through all 15 native paths at both sizes. Native output extraction and PixelBuffer adoption preserve pointer identity. Synthetic RGBA8 retains exact active samples, all rows, completion and untouched padding through all 19 routes, with 64-row strips and a 64-step quota.</p>
<div class="note"><strong>Limits:</strong> external profiler diagnostics, not zenbench Confirm. Streamed readers and positioned Cursor readers are memory-backed. InPlace is called from Runtime::block_on outside a worker task, so worker replacement and concurrent fairness are not measured. No tile/ROI browsing, actual disk/network throughput, current-head allocation counts or complete native metadata/error conformance is established. JPEG still parses its header twice in this binding. The four native documentation jobs remain blocked by the recorded archmage dependency conflict.</div>
<p>The first qualification attempt failed before any timing: the BMP binding unwrapped a native input-amplification check on a 64 KiB prefix. Its corrected probe requests more bytes; the native kernel and safety check remain unchanged. The failed attempt is retained.</p>
<footer><p><a href="report.md">Full report and all route contrasts</a> · <a href="receipt.json">Compact receipt</a> · <a href="http://dev.lan:3300/zencodec/reimagined-wall-cpu-2026-11-11/index.html">Earlier API comparison and bounded studies</a></p><p class="muted">Measured source: SOURCE<br>Native probe: NATIVE<br>Synthetic probe: SYNTHETIC</p></footer>
<script>const codec=document.querySelector('#codec'),size=document.querySelector('#size'),rows=[...document.querySelectorAll('tbody tr')];function filter(){let n=0;for(const row of rows){row.hidden=!!((codec.value&&row.dataset.codec!==codec.value)||(size.value&&row.dataset.size!==size.value));if(!row.hidden)n++;}document.querySelector('#shown').textContent=n+' routes shown';}codec.addEventListener('change',filter);size.addEventListener('change',filter);filter();</script></body></html>'''
html = html.replace('ROWS',''.join(rows)).replace('SOURCE',escape(r['source'])).replace('NATIVE',escape(r['binaries']['native']['sha256'])).replace('SYNTHETIC',escape(r['binaries']['synthetic']['sha256']))
class Check(HTMLParser):
    count=0
    def handle_starttag(self,tag,attrs):
        if tag=='tr' and any(k=='data-codec' for k,v in attrs):self.count+=1
check=Check();check.feed(html);assert check.count==134
assert 'Option&lt;&amp;dyn Pulse&gt;' in html
assert len(rows)==134
# Fresh output only: preserve earlier generated pages.
with a.output.open('x') as f:f.write(html)
print(f'HTML: {len(rows)} routes; {len(html.encode())} bytes')
