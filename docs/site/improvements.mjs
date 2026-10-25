// Each entry describes an executing change, its reason, and its remaining scope.
// Signatures come from Rust through apiPanels; examples remain executable files.
export const improvements = [
  {
    id:'video', title:'Video policy follows each actual presentation',
    state:'Partly implemented', date:'2026-10-25',
    summary:'Native AV1 retains planar samples, color, timing and packet provenance through the shared resolver. Allocation admission and coupled conversion remain open.',
    why:'A later frame can change color interpretation. A callback inspecting the latest configuration could authorize the wrong frame, and refusal must release unpublished reference owners.',
    consumer:'Wrap the existing VideoDecoder in PolicyDecoder; use the same fallible PolicyResolver as still-image operations. Returned frames keep their original interpretation.',
    adapter:'Hold the actual picture and its accepted packet context until the scoped decision succeeds. Failure drops the native decoder and queued output; already returned frames remain valid.',
    implementor:'Keep send/receive, drain and reset semantics. Capture context when a packet is accepted and carry it through reordered and repeated presentations.',
    open:'This callback runs after native reconstruction. rav1d already checks a pixel-count limit before reconstruction; a byte budget across native pictures, work buffers and queued output is still unqualified. Planar conversion and reconfiguration need a complete prepared route.',
    signatures:['framePolicy','frameResolution','video'],
    flow:[['implementor','Reconstruct picture','accepted packet context'],['adapter','Retain exact owner','immutable timing + color'],['consumer','Resolve publication','choose, decline or error']],
    evidence:[['Native AV1 scope and independent FFmpeg checks','perf/av1-policy-validation/README.md'],['Executable video examples','docs/examples/checked_video.rs']],
  },
  {
    id:'resolution', title:'Required density survives, or encoding refuses',
    state:'Implemented · qualified routes', date:'2026-10-25',
    summary:'Generic, erased and registered jobs carry required density. PNG output preserves exactly representable values; JPEG input retains non-square JFIF aspect.',
    why:'Pixel equality does not preserve print size or non-square pixel aspect. A successful transcode must not silently lose a reported field that it promised to retain.',
    consumer:'Use EncodeRequest::with_resolution for existing pixels. Default checked transcodes retain reported density where qualified; no capability-query sequence is required.',
    adapter:'Apply the requirement to the retained job before joint selection. Compare physical units exactly after encoding, and refuse unqualified EXIF/XMP reconciliation.',
    implementor:'Implement try_set_resolution only where the carrier can represent the value. A refused setter leaves the job unchanged; an unsupported default returns an error.',
    open:'PNG cannot represent exact 300 DPI as integer pixels/metre, so that request refuses. No implicit quantization is offered. Required JPEG output, TIFF rational precision, conflicting metadata and axis-changing transforms remain open.',
    signatures:['requiredResolution','resolutionValues'],
    example:'checked_resolution',
    evidence:[['Exact unit contract and rational oracle','docs/resolution-contract.md'],['Native transcode tests','https://github.com/imazen/zenpipe/blob/ffecbf56ec8dc0ebcc6dbb16eb1d5925dccd37a5/zencodecs/tests/checked_transcode/resolution.rs'],['Remaining resolution contract','https://github.com/imazen/zencodec/issues/169']],
  },
  {
    id:'resolver', title:'One resolver selects a complete executable choice',
    state:'Implemented · qualified routes', date:'2026-10-25',
    summary:'Typed questions cover encoding, interpretation boundaries, native frame publication and coupled pixel transcodes. Configured presets and application code use the same interface.',
    why:'Separate color, metadata and encoder choices can each appear supported but fail in combination. The adapter must prepare a coherent combination before asking the application.',
    consumer:'Start with a preset. Add if/then logic only for a situational rule; return a predefined decision or a traced application error.',
    adapter:'Retain the selected conversion plan, encoder job and admitted row buffers across the callback. Scope tokens to this question and preserve typed causes and bounded decision history.',
    implementor:'Suspend before affected effects when interpretation changes. Metadata notifications alone do not authorize conversion, encoder initialization or output publication.',
    open:'Encoded rewrite choices, whole-container privacy and qualified native workspace or encoded staging bounds remain separate qualification work. Unknown evidence refuses a requirement that needs it.',
    signatures:['operationResolver','operationPreset','operationControl'],
    evidence:[['Current contract and implementation record','docs/ideal-api.md'],['Policy history performance and limitations','benchmarks/frame_policy_2026-10-25.md']],
  },
  {
    id:'io', title:'Disk and network share positional demand',
    state:'Implemented · qualified routes', date:'2026-10-25',
    summary:'ReadAtReader bridges an existing synchronous Read + Seek parser; async sources keep byte demand and CPU scheduling separate.',
    why:'Disk benefits from positional reads, while a network source needs explicit ranges, bounded holding and suspension. A second core I/O hierarchy would not supply those missing contracts.',
    consumer:'Keep the checked request when switching among resident bytes, files and async sources. Choose limits for the operation, not a manual codec capability handshake.',
    adapter:'Own the cursor, source errors, input allowance and scheduling quota. A synchronous seek bridge does not itself provide network suspension or buffering.',
    implementor:'Request the bytes needed for progress and honor admitted sink row capacity. Native batch size is separate from the size of the consumer’s strip.',
    open:'Shared item/level/region selection still needs bounded discovery and source-version binding. Slow-writer backpressure needs a proven retained-state bound; output queue size alone is insufficient.',
    signatures:['seekReader','readAt','asyncSource'],
    evidence:[['Native TIFF disk/range experiments','perf/spatial-validation/README.md'],['Reused IMI transport model','benchmarks/io_transport_model_2026-10-25.md'],['Encoder retention experiment','benchmarks/io_encoder_retention_2026-10-25.md']],
  },
];

const escape=s=>s.replaceAll('&','&amp;').replaceAll('<','&lt;').replaceAll('>','&gt;').replaceAll('"','&quot;');
export function improvementsSection({apiPanels,codePanel,flow,gh}) {
  return `<section id="changes" class="ideal-role ideal-changes" aria-labelledby="changes-title">
<p class="section-kicker">Living implementation record / updated with the code</p>
<h2 id="changes-title">What changed, and why?</h2>
<p>Start with the current scope. Expand a change for the consumer, adapter and implementor consequences, exact source signatures and evidence. These entries describe review-PR code; they do not announce a crates.io release.</p>
<div class="change-list">${improvements.map(entry=>`<details class="api-change" id="change-${entry.id}">
<summary><span class="change-title">${escape(entry.title)}</span><span class="change-state">${escape(entry.state)} · <time datetime="${entry.date}">${entry.date}</time></span><span class="change-summary">${escape(entry.summary)}</span></summary>
<div class="change-body"><p><b>Why this belongs in the API.</b> ${escape(entry.why)}</p>
<dl class="change-roles">${['consumer','adapter','implementor'].map(role=>`<div class="${role}"><dt>${role[0].toUpperCase()+role.slice(1)}</dt><dd>${escape(entry[role])}</dd></div>`).join('')}</dl>
${entry.flow?flow(`change-${entry.id}-flow`,entry.flow):''}
${entry.example?codePanel(`change-${entry.id}`,entry.example,'docs/examples/checked_operations.rs'):''}
<details class="ideal-signatures"><summary>Exact traits and signatures</summary>${apiPanels(`change-${entry.id}`,entry.signatures)}</details>
<p class="change-open"><b>Still open.</b> ${escape(entry.open)}</p>
<p class="change-evidence">${entry.evidence.map(([label,target])=>`<a href="${target.startsWith('https://')?target:gh(target)}">${escape(label)} ↗</a>`).join(' · ')}</p>
</div></details>`).join('')}</div>
<p class="ideal-note">The footer identifies this site’s source revision and the native dependency pins. Main’s guide deploys through GitHub Actions; the active implementation is reviewed in the linked PRs.</p>
</section>`;
}
