// Each entry describes an executing change, its reason, and its remaining scope.
// Signatures come from Rust through apiPanels; examples remain executable files.
export const improvements = [
  {
    id:'images', title:'Image identity, containment and composition stay distinct',
    state:'Private candidate · native validation', date:'2026-10-26',
    summary:'Native TIFF tests select nested and shared SubIFDs. Native JXL tests select 147 presentations, including 35 with visible dependency replay. Required raw layers currently refuse.',
    why:'A SubIFD is not always a reduced image, and a shared child makes a graph rather than a tree. A JXL file can expose one composited presentation or five layers, so a page index cannot describe every selection.',
    consumer:'Choose the view before discovery, retain the returned image identity, then request full pixels or a stored region for that image. TIFF returns the selected canvas and region origin. The operation owns the source; the identity is not a portable index or timestamp.',
    adapter:'Keep IDs scoped to the opened operation and bound discovery. TIFF/JXL return one mapped ImageOutput; native AV1 uses the same output without copying planes. Packed color uses zenpixels. JXL retains all seven named auxiliary roles, source integer/float precision, spot/CFA parameters and selected timecodes. Output storage precision and presentation timestamps remain separate facts. Retained spot planes disclose whether they already contributed to output color.',
    implementor:'Expose actual directory relations and presentation dependencies separately. Do not label arbitrary SubIFDs as reduced levels or silently substitute a flattened canvas for a layer.',
    open:'These are private executable candidates, not a public CodecSet collection API. Interpretation is frozen with storage; source facts stay separate from output color and retention permission. Rendering policy, source revision binding and native JXL layers remain open. Source shifts 1/2/3 now qualify on odd canvases; unknown/reserved roles still need fixtures. libjxl independently checks discovery and full generated decoding; general JXL pixel parity compares native sequential and selected decoding.',
    signatures:['collectionSelection','imageOutput','imageMapping','tiffSelection','tiffPixels','jxlSelection'],
    example:'selected_first_image', exampleSource:'perf/spatial-validation/src/collection.rs',
    flow:[['consumer','Choose the view','presentation or layers'],['adapter','Bind the operation','source + scoped identities'],['implementor','Discover actual items','containment + dependencies'],['adapter','Decode the selection','preserve samples or refuse']],
    evidence:[['WIC/OpenImageIO review and resulting contracts','docs/platform-review-2026-10-25.md'],['Native matrix and independent discovery oracle','perf/spatial-validation/README.md'],['JXL source details and selected timecodes','https://github.com/imazen/zenjxl-decoder/pull/68'],['Signed/wide samples and native allocation adoption','https://github.com/imazen/zenpixels/pull/93'],['Shared selection discussion','https://github.com/imazen/zencodec/issues/1'],['Native JXL layer support','https://github.com/imazen/zenjxl-decoder/issues/57']],
  },
  {
    id:'debugging', title:'Media failures retain the original cause',
    state:'Implemented · review PR', date:'2026-10-25',
    summary:'Pump and mapping errors preserve concrete backend causes and location traces through adapters and type erasure.',
    why:'A production error often crosses a codec, a pump, conversion and a destination. Losing the cause at any wrapper makes the failure hard to classify or reproduce.',
    consumer:'Use Error::source or zencodec::find_cause to inspect the original failure. The pump still identifies which side failed; no message parsing is required for retained typed causes.',
    adapter:'Forward the owned error through PumpError, AdaptError and erasure. Keep traced callback and destination failures intact. An error report adds context without replacing its cause.',
    implementor:'Pass the original error into MapError::Backend as a boxed Error + Send + Sync. Use Unavailable for storage without a CPU mapping.',
    open:'This changes the unpublished MapError payload from String to an owned error. Native AV1 plane-limit refusal still lacks a distinct typed category and stable refusal facts; retaining its cause does not supply those facts.',
    signatures:['findCause','mediaPumpCause','mediaMapCause'],
    evidence:[['Media error contract','docs/media-contract.md#error-inspection-across-pumps-and-mappings'],['Mapping and pump regression controls','zencodec-media-testkit/tests/error_chains.rs'],['Encoder pump and terminal behavior','zencodec-media-testkit/tests/encoder_fusing.rs'],['Native refusal category gap','https://github.com/imazen/rav1d-safe/issues/538']],
  },
  {
    id:'video', title:'Video policy follows each actual presentation',
    state:'Partly implemented', date:'2026-10-25',
    summary:'Native AV1 retains samples, color, timing and packet provenance through the shared resolver. Optional native plane admission precedes allocation; full workspace bounds and coupled conversion remain open.',
    why:'A later frame can change color interpretation. Publication policy must inspect that frame. A storage requirement must act earlier, before its native planes allocate; a callback after reconstruction cannot enforce it.',
    consumer:'Set max_picture_bytes when the session needs a native plane allowance, then wrap VideoDecoder in PolicyDecoder. The same fallible resolver checks each presentation. Returned frames keep their original interpretation and storage charge.',
    adapter:'Hold the actual picture and accepted packet context until the scoped decision succeeds. Native admission failure keeps its traced cause and drops unpublished state. Already returned frames remain valid and charged.',
    implementor:'Admit a complete picture before its first plane allocation. Carry the charge with native references and retained frames through reset/drop. Preserve send/receive, drain and accepted-packet provenance.',
    open:'The native plane byte budget covers requested bytes with padding and alignment. It excludes work buffers, input, metadata, allocator overhead and encoded queues; it is not a total RAM bound. C-FFI refuses this setting. Publication policy still runs after native reconstruction. Structured native refusal facts, coupled planar conversion and reconfiguration remain open.',
    signatures:['framePolicy','frameResolution','video'],
    example:'checked_video_storage', exampleSource:'docs/examples/checked_video.rs',
    flow:[['implementor','Admit native planes','before first allocation'],['implementor','Reconstruct picture','accepted packet context'],['adapter','Retain exact owner','immutable timing + color'],['consumer','Resolve publication','choose, decline or error']],
    evidence:[['Native AV1 scope and independent FFmpeg checks','perf/av1-policy-validation/README.md'],['Executable video examples','docs/examples/checked_video.rs'],['Native allocation contract','https://github.com/imazen/rav1d-safe/blob/b19aec6561204ff310250bf2bc1b2c6faa42c3b5/docs/PICTURE_STORAGE_LIMIT.md'],['Typed native refusal gap','https://github.com/imazen/rav1d-safe/issues/538']],
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
${entry.example?codePanel(`change-${entry.id}`,entry.example,entry.exampleSource??'docs/examples/checked_operations.rs'):''}
<details class="ideal-signatures"><summary>Exact traits and signatures</summary>${apiPanels(`change-${entry.id}`,entry.signatures)}</details>
<p class="change-open"><b>Still open.</b> ${escape(entry.open)}</p>
<p class="change-evidence">${entry.evidence.map(([label,target])=>`<a href="${target.startsWith('https://')?target:gh(target)}">${escape(label)} ↗</a>`).join(' · ')}</p>
</div></details>`).join('')}</div>
<p class="ideal-note">The footer identifies this site’s source revision and the native dependency pins. Main’s guide deploys through GitHub Actions; the active implementation is reviewed in the linked PRs.</p>
</section>`;
}
