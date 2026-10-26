// Scope audit for the API review. Entries name existing interfaces; missing
// behavior is not presented as an implemented method or a released promise.
const gaps = [
  ['Display rendering', 'CodecSet::open_collection · OpenRequest · ImageRequest · Placement::visible',
    'Explicit native composited/raw selection, checked placement intersection and retained source geometry.',
    'No shared render convenience guaranteeing the presentation canvas, applied orientation, requested color and aligned auxiliaries. Composited and packed do not mean display-ready.',
    'Shared request/result contract; codec composition; consumer adapter.', 'https://github.com/imazen/zencodec/issues/170'],
  ['Planar video conversion', 'Frame · PlaneView::as_strided_samples · ImageOutput::from_frame · Question::Frame',
    'Native AV1 ownership, timing, epochs, exact component codes and policy publication. Private bridge validates borrowed plane kernels and odd crops.',
    'No public planar conversion plan or prepared missing-siting choice. No coupled converter/encoder reconfiguration or converted video ImageOutput adoption. Independent RGB accuracy, HDR/other matrices, mono/GBR, cancellation and workspace admission remain open.',
    'zenpixels-convert and video adapters; shared facts/decisions in zencodec.', 'perf/av1-policy-validation/README.md'],
  ['Opaque and nested privacy', 'PolicyResolver::resolve · Question::Transcode · MetadataTransfer',
    'One fallible callback, scoped offers and reported metadata/color treatment.',
    'No whole-container PII guarantee over unknown packets, unparsed XML, reconstruction data and nested items. Missing inspection evidence cannot authorize pass-through under a strict exclusion requirement. Need explicit inspection scope and prepared preserve/drop/refuse mechanisms.',
    'Shared evidence and offers; format inspectors and rewrite adapters.', 'docs/ideal-api.md'],
  ['Native interpretation suspension', 'DecodeRequest::start_controlled · Question::Boundary',
    'Checked suspension/resume and native whole-source JPEG/PNG routes, including relevant late metadata refusals.',
    'Native byte-resuming reconstruction is not qualified by those whole-source implementations. Other codec/configuration combinations and late interpretation changes need exact suspension points before affected effects.',
    'Codec state machines and their controlled adapters.', 'docs/range-decoding-contract.md'],
  ['Whole-operation memory', 'StorageBudget · OpenRequest::with_output_storage · CollectionOpenContext · ResourceLimits',
    'Managed payload admission, retained owner charges, native AV1 picture-plane admission and scoped driver input limits.',
    'No universal bound including native codec/CMS workspaces, metadata, profile generation, parser state, conversion scratch, source caches and encoded staging. max_output_bytes is not an allocation bound.',
    'Native allocation owners plus adapter accounting.', 'docs/storage-admission.md'],
  ['Safe slow-writer behavior', 'Encoder::begin · Encoder::pending_output · OutputTiming · WorkQuota',
    'CPU fairness is separate from writer pressure; qualified finite routes finish encoding before awaiting output.',
    'No general admitted spool or configuration-specific sequential offer proving retained raw/intermediate storage and pausable upstream input. Byte availability does not prove that blocking the writer is safe.',
    'Encoder configurations, storage-aware drivers and transcode offers.', 'https://github.com/imazen/zencodec/issues/160'],
  ['Seek-back and repeated input', 'Encoder · DynReadAt · ReadAtReader',
    'Positional input and standard Read/Seek parser bridge; sync/async drivers.',
    'No common output backpatch transaction or complete bounded multi-pass raster-source route. Those must distinguish seekable disk output from network publication and declare staging before work.',
    'Optional output/source mechanisms in adapters; no mandatory general I/O hierarchy.', 'https://github.com/imazen/zencodec/issues/159'],
  ['Packaged spatial adapters', 'CollectionFactory::open_with · Collection::children · Collection::describe · Collection::decode',
    'Shared collection and CodecSet registration; packaged JXL review adapter; real private TIFF/SubIFD/BigTIFF, tiled/strip and conditional HTTP controls.',
    'TIFF is not yet a packaged factory. No universal efficient pyramid/region route, common blend/reference graph or full required-configuration coverage. A valid region request does not establish proportional decode work or memory.',
    'Codec packages and explicit realizable selection offers.', 'docs/collection-contract.md'],
  ['Conversion effects and ownership', 'OutputPlan::new_with_descriptor · ImageOutput::replace_packed_color · PackedImageOwner::retain_auxiliaries',
    'Required packed descriptors, native float RGB/RGBA CMS cases, same-coordinate color replacement and separately retained auxiliary allocations.',
    'No geometry-changing replacement preserving all auxiliary coordinates; component codes refuse packed replacement. General ICC/LUT and float CMYK conversion remain unqualified. forbid_lossy still needs explicit clip_out_of_gamut(false) for the qualified unclipped route.',
    'zenpixels-convert effects contract and output adapters.', 'docs/ideal-api.md'],
  ['Required metadata and settings', 'DecodeRequest::decode · EncodeRequest::prepare · EncodeRequest::encode_with',
    'Required-versus-preferred requests, prepared native choices, typed refusal and qualified JPEG/PNG metadata/density cases.',
    'No codec-wide proof for every configuration, fidelity and metadata combination. Wider density reconciliation and whole-container retention remain open. Legacy best-effort setters are still public and are not strengthened by the new checked methods.',
    'Native conformance loop, then migration/deprecation.', 'https://github.com/imazen/zencodec/issues/169'],
  ['0.3 surface cleanup', 'CodecSet::with_send_encoder · EncodeRequest::prepare_send · frame::SampleEncoding',
    'Additive Send registrations/preparation and shared image/video ownership.',
    'Send is not the ordinary registry default. Duplicate sample-encoding types and parallel legacy decode/configuration paths remain. No coordinated removal or renamed 0.3 surface has landed.',
    'Cross-crate migration after native consumers qualify.', 'docs/spec.md'],
  ['Freestanding no_std', 'Default zencodec dependency graph',
    'WASM build coverage and alloc-based core interfaces.',
    'The freestanding default graph still has the tracked std/math dependency gap. Passing wasm32 does not establish a bare-metal build.',
    'Dependency graph and qualified math implementation.', 'https://github.com/imazen/zencodec/issues/165'],
];

export function gapsPage({sha, apiPanels, flow, compile}) {
  const gh = file => `https://github.com/imazen/zencodec/blob/${sha}/${file}`;
  const e = s => String(s).replaceAll('&','&amp;').replaceAll('<','&lt;').replaceAll('>','&gt;').replaceAll('"','&quot;');
  const rows = gaps.map(([name, api, present, missing, owner, evidence]) =>
    `<tr><th scope="row">${e(name)}<small>${e(owner)}</small></th><td><code>${e(api)}</code><details><summary>What is implemented</summary><p>${e(present)}</p></details></td><td>${e(missing)} <a href="${e(evidence.startsWith('https:') ? evidence : gh(evidence))}">Evidence / tracking ↗</a></td></tr>`).join('');
  return `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Remaining API contracts — zencodec</title><link rel="icon" href="data:,"><link rel="stylesheet" href="styles.css"><link rel="stylesheet" href="ideal.css"><style>.ideal-role .gap-table{min-width:0;table-layout:fixed;width:100%}.gap-table th{width:20%;white-space:normal;overflow-wrap:anywhere}.gap-table td{width:40%;vertical-align:top;white-space:normal;overflow-wrap:anywhere}.gap-table code{white-space:normal;overflow-wrap:anywhere}.gap-table small{display:block;font-weight:normal;margin-top:.6em}.gap-table details{margin-top:.75em}@media(max-width:650px){.gap-table,.gap-table tbody,.gap-table tr,.gap-table th,.gap-table td{display:block;width:100%}.gap-table thead{display:none}.gap-table tr{margin:1rem 0;border:1px solid var(--line)}.gap-table th,.gap-table td{box-sizing:border-box}}</style></head>
<body><a class="skip" href="#gaps-main">Skip to remaining contracts</a><header class="ideal-header"><a class="brand" href="index.html">zen<span>codec</span></a><nav aria-label="Scope audit"><a href="ideal.html">Current API</a><a href="#canvas">Canvas default</a><a href="#listing">API gaps</a><a href="#compile-time">Compile time</a></nav></header>
<main id="gaps-main" class="wrap ideal-main"><section class="ideal-intro"><p class="section-kicker">Implementation audit / review PR</p><h1>What is still missing?</h1><p class="ideal-lede">Existing signatures and tested mechanisms do not establish the complete 0.3 target. This page separates missing shared contracts, missing native adapters and behavior that still needs qualification.</p><p>Read alongside the <a href="ideal.html">executing API guide</a>. This is review-branch scope, not a claim that these changes are released or all on main.</p></section>
<section id="canvas" class="ideal-role"><h2>Render the canvas by default.</h2><p><b>Proposed consumer contract:</b> an ordinary render returns the presentation canvas with requested orientation and color established. An explicit raw-layer request preserves the full local sample bounds and signed placement. The render convenience is not implemented yet.</p>
${flow('gaps-canvas',[['implementor','Decode required state','keep reference / off-canvas data'],['adapter','Compose visible contribution','respect blend and disposal'],['adapter','Apply requested rendering','canvas, orientation, color'],['consumer','Receive rendered canvas','raw layers require explicit access']])}
<p>The current <code>OpenRequest::new(view, discovery_attempts)</code> requires a view. <code>ImageRequest::new()</code> asks for the whole selected image; in a raw view that may exceed the presentation canvas. <code>Placement::visible()</code> returns matching sample and canvas rectangles but does not change pixels. Native tests include a 1024×576 JXL layer with only a 188×202 visible intersection.</p><p>Clipping must happen at the appropriate composition stage, without destroying data required by later references. A smaller layer still renders onto the complete canvas; a fully off-canvas layer is valid, not EOF. Orientation can change the final width and height. Video crop and display resize are separate operations. An output crop must not evade admission of larger coded dependencies.</p>
<p>The naming concern is concrete: <code>ImageGeometry::canvas_width</code> describes the selected image’s local full width, while <code>Placement::canvas</code> is its presentation canvas. Also, <code>with_pixels</code> means packed access; it does not establish display readiness. <a href="https://github.com/imazen/zencodec/issues/170">Track default rendering and its adversarial tests in issue 170</a>.</p>
${apiPanels('gaps-placement',['layerPlacement'])}</section>
<section id="listing" class="ideal-role"><h2>Remaining contracts by API</h2><p>Names below exist today. The final column describes missing behavior, not proposed methods disguised as an implemented API.</p><table class="gap-table"><thead><tr><th>Concern / owner</th><th>Existing API</th><th>Missing or unqualified</th></tr></thead><tbody>${rows}</tbody></table></section>
<section id="compile-time" class="ideal-role"><h2>Compile time</h2>${compile}</section>
</main><footer class="wrap"><p>Audit source in docs/site/gaps.mjs · revision <a href="https://github.com/imazen/zencodec/commit/${sha}">${sha.slice(0,8)}</a> · <a href="${gh('docs/spec.md')}">Specification</a> · <a href="ideal.html">Current API</a></p></footer></body></html>`;
}
