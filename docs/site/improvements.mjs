// Each entry describes an executing change, its reason, and its remaining scope.
// Signatures come from Rust through apiPanels; examples remain executable files.
export const improvements = [
  {
    id:'mapping', title:'Still images and video share the native mapping contract',
    state:'Implemented · review PR', date:'2026-10-26',
    summary:'Frame owners and borrowed component-plane mappings now live in zencodec::frame. Existing media imports name the same types.',
    why:'A selected still image can expose native component planes just like a video frame. Its output must retain the backend owner without depending on video sessions or copying pixels to cross an API boundary.',
    consumer:'Keep the returned frame owner and borrow its mapped rows. Retain significant code depth, bit placement, crop and timing; converting ten-bit component codes to display RGB is a separate checked operation.',
    adapter:'Use the shared Frame, Picture and MappedPlanes types. Native TIFF/JXL selected output and AV1 exercise the same implementation; media import paths remain compatible.',
    implementor:'Implement Picture::map with a guard that keeps native storage borrowed. Core mapping has no dependency on media sessions or I/O drivers. Accounted software allocations remain in media.',
    open:'The mapped ImageOutput is now implemented in core and checked by the same native consumers. The shared collection is now implemented on the API review PR. JXL has a concrete factory in its codec review PR; TIFF remains in the native harness. Publication policy, color conversion and workspace bounds remain separate gates.',
    signatures:['nativePicture','nativePlanes'],
    flow:[['implementor','Retain native owner','storage survives decoder'],['adapter','Map through core','borrow the backend guard'],['consumer','Read native rows','retain interpretation']],
    evidence:[['Shared ownership contract','docs/media-contract.md#frames'],['Native output qualification','perf/spatial-validation/README.md']],
  },
  {
    id:'images', title:'Image identity, containment and composition stay distinct',
    state:'Shared contract implemented · JXL codec package in review', date:'2026-10-26',
    summary:'Native TIFF tests select nested and shared SubIFDs. Native JXL tests select 147 presentations, including 35 with visible dependency replay. A separate raw-layer adapter now selects nine qualified local layers through the same checked consumer call.',
    why:'A SubIFD is not always a reduced image, and a shared child makes a graph rather than a tree. A JXL file can expose one composited presentation or five layers, so a page index cannot describe every selection.',
    consumer:'Configure the resolver once, choose the view before discovery, then request full pixels or a stored region for a retained image identity. CheckedCollection runs publication policy automatically on every decode. TIFF returns the selected canvas and region origin. The operation owns the source; the identity is not a portable index or timestamp.',
    adapter:'Keep IDs scoped to the opened operation and bound discovery. TIFF/JXL return one mapped ImageOutput; native AV1 uses the same output without copying planes. Packed color uses zenpixels. JXL retains all seven named auxiliary roles, source integer/float precision, spot/CFA parameters and selected timecodes. Output storage precision and presentation timestamps remain separate facts. Retained spot planes disclose whether they already contributed to output color. JXL exposes JxlCollectionConfig::new(max_images, max_frame_headers): retained identities and hidden header work have separate allowances.',
    implementor:'Expose actual directory relations and presentation dependencies separately. Do not label arbitrary SubIFDs as reduced levels or silently substitute a flattened canvas for a layer.',
    open:'Collection, Discovery, CheckedCollection and CodecSet registration now live in core. JXL factory packaging is implemented in zenjxl PR31; TIFF packaging remains open. Publication follows reconstruction, so it proves neither preallocation admission nor whole-container privacy. Native JXL header counts and shared discovery-call counts do not bound work inside each parse or total workspace. Broader source snapshots, hard cancellation latency, remaining layer configurations and unknown auxiliary roles remain unqualified.',
    signatures:['checkedCollection','collectionSelection','collectionDiscovery','sourceSnapshot','collectionOpen','imageOutput','imageMapping','imageResolution','tiffSelection','tiffPixels','jxlSelection'],
    example:'checked_jxl_first', exampleSource:'perf/spatial-validation/src/collection.rs',
    flow:[['consumer','Choose the view','presentation or layers'],['adapter','Bind the operation','source + scoped identities'],['implementor','Discover actual items','containment + dependencies'],['adapter','Check the held output','policy accepts or refuses']],
    evidence:[['Shared collection contract and boundaries','docs/collection-contract.md'],['WIC/OpenImageIO review and resulting contracts','docs/platform-review-2026-10-25.md'],['Native matrix and independent discovery oracle','perf/spatial-validation/README.md'],['JXL source details and selected timecodes','https://github.com/imazen/zenjxl-decoder/pull/68'],['Signed/wide samples and native allocation adoption','https://github.com/imazen/zenpixels/pull/93'],['Shared selection discussion','https://github.com/imazen/zencodec/issues/1'],['Native JXL layer support','https://github.com/imazen/zenjxl-decoder/issues/57']],
  },
  {
    id:'inspection', title:'Inspect a selected image before reconstructing pixels',
    state:'Shared contract implemented · native TIFF and JXL', date:'2026-10-26',
    summary:'The viewer can obtain stored dimensions, orientation and placement for an issued identity before asking for pixels.',
    why:'An explorer needs page and layer geometry to plan layout and region requests. Decoding a full raster merely to learn its dimensions creates an allocation the caller has not yet chosen.',
    consumer:'Call images.describe(&id). Keep the opened operation for later decode requests. Unknown orientation remains unknown; a successful description does not promise that decoding or conversion will succeed.',
    adapter:'Implement Collection::describe using established source facts without reconstructing pixels or advancing cursors. TIFF reads directory tags; JXL retains basic and layer headers. The checked consumer validates geometry, preserves original causes and checks the final cancellation seam.',
    implementor:'Keep local stored dimensions distinct from a presentation canvas, and preserve the source revision through later reconstruction. Do not infer complete metadata, a rendered pixel format or a workspace estimate from these header facts.',
    open:'Native discovery can still traverse prior compressed frames, and transport buffers may read payload bytes. TIFF factory packaging, additional header facts, complete privacy inspection and native workspace admission remain open. Pixel publication still uses the existing resolver after reconstruction.',
    signatures:['collectionDescription','collectionSelection','checkedCollection','collectionDiscovery'],
    example:'checked_first_description', exampleSource:'perf/spatial-validation/src/collection.rs',
    flow:[['consumer','Inspect the selected identity','dimensions + orientation'],['adapter','Validate source geometry','same source revision'],['consumer','Choose a region','decode and resolve publication']],
    evidence:[['Selected description contract','docs/collection-contract.md#inspect-geometry-before-allocating-samples'],['Native qualification and limits','perf/spatial-validation/README.md#inspect-a-selected-image-before-reconstructing-it'],['Shared API qualification','benchmarks/public_description_2026-10-26.json'],['Initial experiment receipt','benchmarks/selected_description_2026-10-26.json']],
  },
  {
    id:'factories', title:'Keep format-specific opening out of the viewer loop',
    state:'Implemented on review PR · native adapter packaging open', date:'2026-10-26',
    summary:'Register native configurations once, then open the same checked consumer over borrowed bytes or a positional source.',
    why:'The explorer now handles both TIFF pages and JXL layers. Native configurations belong in CodecSet; its selection loop should not repeat format-specific construction and error conversion.',
    consumer:'Choose the format and view, supply one source, resolver and stop token, then use the existing first-image, description and selected-decode methods. Keeping an image does not require keeping the registry or source alive.',
    adapter:'Erase the collection once at open and owned cursor state once per cursor. ReadAtShim now accepts a local source through the same DynReadAt protocol; require + Send only when the erased object crosses threads. Preserve source lifetimes and errors. A failed first matching factory does not fall back to a different implementation. Inherited limits and decode policy must hold or refuse. Typed refusals retain the specific unhandled settings; all inherited and per-call cancellation sources remain active.',
    implementor:'TIFF accepts positional reads, including the thread-local HTTP simulator. JXL currently requires an already-resident slice and refuses before reading when one is unavailable. Do not add implicit whole-file buffering to make an opening signature succeed.',
    open:'CodecSet registration is implemented. JXL uses the real zenjxl::collection::JxlCollectionConfig; TIFF remains a private adapter. TIFF enforces stored width, height and pixel caps before compressed-pixel reads, while allowing inspection and smaller children. JXL enforces the actual resident input-byte cap before parsing, regardless of length hints. Both refuse unqualified legacy limit/policy configurations before I/O. Native tests show that a selected layer can fit a pixel cap while its canvas does not, and independently budgeted decoders can retain more output than one decoder budget. Native work and retained ownership need separate enforcement. Transferable opened jobs, automatic detection, native progress, fallible cursor allocation, wider codec coverage and measured erasure costs remain open.',
    signatures:['collectionFactories','collectionOpenRequest','collectionRefusal','operationRegistration','collectionFactory','dynReadAt','checkedCollection','collectionStop'],
    example:'checked_registered_first', exampleSource:'perf/spatial-validation/src/collection.rs',
    flow:[['consumer','Open one checked operation','source + view + policy'],['adapter','Select the registered factory','native limits + source lifetime'],['implementor','Honor the source mode','positional or resident-only']],
    evidence:[['Measured native resource scopes','perf/spatial-validation/README.md#native-jxl-limit-scope'],['Packaged JXL API and qualified limits','https://github.com/imazen/zenjxl/blob/review/checked-collections/docs/COLLECTIONS.md'],['Native opening experiment and limits','perf/spatial-validation/README.md#open-native-collections-through-registered-factories'],['Executing native controls','perf/spatial-validation/src/factory_tests.rs'],['Conditional HTTP controls','perf/spatial-validation/src/http_tests.rs']],
  },
  {
    id:'placement', title:'Layer placement does not weaken pixel bounds',
    state:'Shared placement implemented · native raw layers', date:'2026-10-26',
    summary:'A layer can extend beyond its presentation canvas. The public native F32 path now returns nine qualified complete local layers. The shared collection binds their placement and composition facts to the sample owner before policy approval.',
    why:'The real JXL fixture has a 1024×576 layer at (1860,950) on a 2048×1152 canvas. Its visible rectangle is 188×202. Clipping the stored layer would lose samples; accepting those coordinates as a bounded crop would weaken mapping checks.',
    consumer:'Choose the layer view at open, then use the same first_image or decode call as for presentations. Read ImageInterpretation::placement from the returned owner and ask for its intersection when drawing. The owner rejects invalid placement before publication. Off-canvas samples remain available; a hidden layer is not end of discovery.',
    adapter:'Freeze signed placement, native blend instructions and reference slots with the actual local samples. The adapter replays from the start for selection. Native CMYK uses CmykF32Inverted; separate black also declares inverted polarity, so attaching an ICC profile cannot silently change sample meaning.',
    implementor:'Save local F32 rows before blending and canvas extension while continuing to reconstruct references. Stored orientation, skipped previews, unrendered spots and retained black are required for this native path. Independent libjxl agrees on geometry, blend facts and CMYK/alpha bits; lossy RGB uses the unchanged conformance bound.',
    open:'The shared collection is implemented in core; placement is frozen in ImageInterpretation and native raw output is implemented on its review PR. All blend configurations, native negative-origin fixtures, reference-only export, preview/progressive output, raw random seeking, CMYK conversion and workspace bounds remain unqualified. Publication happens after reconstruction. Source blend facts are not a lossless re-encoding contract.',
    signatures:['jxlLayerSelection','checkedCollection','layerPlacement','imageOutput'],
    example:'checked_jxl_layer_first', exampleSource:'perf/spatial-validation/src/collection.rs',
    flow:[['implementor','Retain local samples','1024 × 576 layer'],['adapter','Place on the canvas','origin (1860,950)'],['consumer','Draw the intersection','188 × 202 visible']],
    evidence:[['Executing placement contract and limits','perf/spatial-validation/README.md#layer-placement-without-weakening-pixel-bounds'],['Placement qualification','benchmarks/spatial_placement_2026-10-26.json'],['Selected raw-output qualification','benchmarks/raw_collection_2026-10-26.json'],['Native F32 output and limits','https://github.com/imazen/zenjxl-decoder/blob/0f402c6ad1df35f8326e0bb9ec7c1b2073e449d7/docs/source-interpretation.md#qualified-public-raw-layer-output'],['Native layer contract','https://github.com/imazen/zenjxl-decoder/issues/57']],
  },
  {
    id:'cms', title:'Callbacks remain fallible inside the codec',
    state:'Implemented · native review PR', date:'2026-10-26',
    summary:'Native JXL color-management failures now return the original traced error through both renderers. They cannot become a panic or an apparently successful fallback image.',
    why:'A fallible application resolver is useful only if downstream services also honor refusal. The real-codec loop found RGB callbacks that panicked and CMYK callbacks whose errors produced substitute pixels.',
    consumer:'Use the normal Result path to inspect the original CMS error and its whereat trace. Native CMYK samples retain their declared inverted ink domain; conversion requires an explicit qualified CMS route.',
    adapter:'Keep ink-polarity adaptation inside the selected CMS backend. Pass the requested destination profile and preserve the original callback error through the common color stage.',
    implementor:'Validate the returned transformer count and channel layout before use. Propagate failures from initialization and every row chunk, including parallel rendering. One shared fallible stage now handles RGB, gray and CMYK.',
    open:'Qualified native callback cases cover first and later chunks, both renderers and threaded execution. Arbitrary ICC transforms, callback panics, callback workspace/time bounds and recovery through the mutable low-level decoder remain unqualified. This is not a complete CMYK conversion contract.',
    signatures:['findCause'],
    flow:[['implementor','Call the chosen CMS','requested profile + native samples'],['adapter','Preserve its result','same traced error on refusal'],['consumer','Handle the failure','typed cause + original trace']],
    evidence:[['Native callback contract and qualification','https://github.com/imazen/zenjxl-decoder/blob/fe819360581acb3796ea2925f2a81f44f0a430e6/docs/source-interpretation.md#fallible-cms-callbacks'],['Native reproducer and implementation','https://github.com/imazen/zenjxl-decoder/issues/72'],['Downstream TIFF/JXL/HTTP regression receipt','benchmarks/cms_collection_2026-10-26.json']],
  },
  {
    id:'debugging', title:'Keep the original failure and the observed stop',
    state:'Implemented · review PR', date:'2026-10-26',
    summary:'Collection errors expose the observed stop without discarding a coincident failure. Pump and mapping errors preserve concrete backend causes and location traces.',
    why:'A production error often crosses a codec, a pump, conversion and a destination. Losing the cause at any wrapper makes the failure hard to classify or reproduce.',
    consumer:'On a collection error, use stop_reason() for the first stop observed by its shared token. Use Error::source or zencodec::find_cause for the original failure. The pump still identifies which side failed; retained typed causes require no message parsing.',
    adapter:'Forward the owned error through PumpError, AdaptError and erasure. Keep traced callback and destination failures intact. An error report adds context without replacing its cause.',
    implementor:'Pass the original error into MapError::Backend as a boxed Error + Send + Sync. Use Unavailable for storage without a CPU mapping.',
    open:'This changes the unpublished MapError payload from String to an owned error. Native AV1 plane-limit refusal still lacks a distinct typed category and stable refusal facts; retaining its cause does not supply those facts.',
    signatures:['findCause','collectionStop','mediaPumpCause','mediaMapCause'],
    evidence:[['Collection failure boundaries','docs/collection-contract.md#failure-and-publication-boundaries'],['Media error contract','docs/media-contract.md#error-inspection-across-pumps-and-mappings'],['Mapping and pump regression controls','zencodec-media-testkit/tests/error_chains.rs'],['Encoder pump and terminal behavior','zencodec-media-testkit/tests/encoder_fusing.rs'],['Native refusal category gap','https://github.com/imazen/rav1d-safe/issues/538']],
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
    summary:'Typed questions cover encoding, interpretation boundaries, selected-image/native-frame publication and coupled pixel transcodes. Configured presets and application code use the same interface.',
    why:'Separate color, metadata and encoder choices can each appear supported but fail in combination. The adapter must prepare a coherent combination before asking the application.',
    consumer:'Start with a preset. Add if/then logic only for a situational rule; return a predefined decision or a traced application error.',
    adapter:'Retain the selected conversion plan, encoder job and admitted row buffers across the callback. Scope tokens to this question and preserve typed causes and bounded decision history. DecisionContext retains input facts on success and refusal; Selection records the chosen effect without repeating frame or boundary facts.',
    implementor:'Suspend before affected effects when interpretation changes. Metadata notifications alone do not authorize conversion, encoder initialization or output publication.',
    open:'Encoded rewrite choices, whole-container privacy and qualified native workspace or encoded staging bounds remain separate qualification work. Unknown evidence refuses a requirement that needs it.',
    signatures:['operationResolver','operationPreset','operationControl'],
    evidence:[['Current contract and implementation record','docs/ideal-api.md'],['Policy history performance and limitations','benchmarks/frame_policy_2026-10-25.md'],['Selected-image policy cost: confirmation refused','benchmarks/image_policy_cost_2026-10-26.md']],
  },
  {
    id:'io', title:'Disk and network share positional demand',
    state:'Implemented · qualified routes', date:'2026-10-25',
    summary:'ReadAtReader bridges an existing synchronous Read + Seek parser; async sources keep byte demand and CPU scheduling separate.',
    why:'Disk benefits from positional reads, while a network source needs explicit ranges, bounded holding and suspension. A second core I/O hierarchy would not supply those missing contracts.',
    consumer:'Keep the checked request when switching among resident bytes, files and async sources. Choose limits for the operation, not a manual codec capability handshake.',
    adapter:'Own the cursor, source errors, input allowance and scheduling quota. Bind coherent source bytes before native selection: conditional HTTP requests can refuse replacement; a private Linux adapter uses an atomic file clone. Both feed the same ReadAt contract. A seek bridge does not itself provide network suspension or buffering.',
    implementor:'Request the bytes needed for progress and honor admitted sink row capacity. Native batch size is separate from the size of the consumer’s strip.',
    open:'HTTP qualification covers one conditional object; file-clone qualification covers the tested Linux/XFS path. Multi-object sources, browser transports and other filesystems/platforms remain open. Neither path proves native workspace bounds. Slow-writer backpressure still needs a proven retained-state bound; output queue size alone is insufficient.',
    signatures:['seekReader','readAt','asyncSource','fileSnapshot'],
    evidence:[['Native TIFF disk/range experiments','perf/spatial-validation/README.md'],['Atomic file-source qualification','benchmarks/file_snapshot_2026-10-26.json'],['Reused IMI transport model','benchmarks/io_transport_model_2026-10-25.md'],['Encoder retention experiment','benchmarks/io_encoder_retention_2026-10-25.md']],
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
