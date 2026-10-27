// Every code body is extracted from a compiled source file, never transcribed.
export const codeFlows = [
  {id:'adapter-declaration',role:'adapter',title:'Declare color → plan without startup → verify start',note:'Shared trait hooks are implemented and tested. Native JPEG/PNG adoption and the checked request-to-destination driver remain in progress. Carrier evidence can be unknown; planning does not establish a native memory bound.',steps:[
    {scope:'fn erased_preflight_retains_job_color_access_and_native_error()',file:'src/traits/dyn_encoding.rs',api:['encodeDeclaration','encodePlan','encodePreflight','declaredBegin'],state:'Shared contract test; scripted codec, not native qualification'},
  ]},
  {id:'codec-preflight',role:'implementor',title:'One effective plan → declared startup',steps:[
    {scope:'impl EncodeJob for Job',file:'src/traits/dyn_encoding.rs',api:['encodePreflight'],state:'Shared contract fixture: planning without encoder construction'},
    {scope:'impl Encoder for Declared',file:'src/traits/dyn_encoding.rs',api:['declaredBegin'],state:'Shared contract fixture: current color reaches startup'},
  ]},
  {id:'app-start',role:'consumer',title:'Register → decode → transcode',steps:[
    {example:'checked_registry',file:'docs/examples/checked_operations.rs',api:['operationRegistration']},
    {example:'checked_decode',file:'docs/examples/checked_operations.rs',api:['operationDecode']},
    {example:'checked_default',file:'docs/examples/checked_operations.rs',api:[],external:['integration-api.json']},
  ]},
  {id:'app-render',role:'consumer',title:'Select → crop canvas → render',steps:[
    {example:'checked_registered_first',file:'perf/spatial-validation/src/collection.rs',api:['collectionFactories','checkedCollection']},
    {scope:'fn http_snapshot_render_first_matches_slice_and_rejects_late_source_faults()',file:'perf/spatial-validation/src/http_tests.rs',api:['canvasRenderRequest','canvasRenderer'],state:'Native TIFF integration test'},
  ]},
  {id:'app-convert',role:'consumer',title:'Convert color → map a framebuffer',steps:[
    {example:'convert_display',file:'docs/examples/pixel_conversion.rs',api:['registryCalls']},
    {example:'convert_framebuffer',file:'docs/examples/pixel_conversion.rs',api:['imageOutput']},
  ]},
  {id:'app-transcode',role:'consumer',title:'Transcode → metadata → custom policy',note:'Mandatory PolicySet requirements filter routes before the callback runs. Decision::Choose selects only an admitted route; unknown required evidence refuses. This is route admission, not a whole-file privacy certificate: deferred obligations and unrelated question domains still need operation-wide handling (issue 173).',steps:[
    {example:'checked_web',file:'docs/examples/checked_operations.rs',api:['operationPreset'],external:['integration-api.json']},
    {example:'checked_callback',file:'docs/examples/checked_operations.rs',api:['operationResolver'],external:['integration-api.json']},
  ]},
  {id:'app-io',role:'consumer',title:'Memory → disk → async source',steps:[
    {example:'checked_file',file:'docs/examples/checked_operations.rs',api:['file','readAt'],external:['integration-api.json']},
    {example:'checked_async',file:'docs/examples/checked_operations.rs',api:['asyncSource'],external:['integration-api.json']},
  ]},
  {id:'app-rewrite',role:'consumer',title:'Rewrite metadata → refuse opaque ICC',steps:[
    {example:'jpeg_rewrite',file:'docs/examples/jpeg_policy.rs',api:['rewriteRequest']},
    {example:'jpeg_icc_refusal',file:'docs/examples/jpeg_policy.rs',api:['rewritePlan']},
  ]},
  {id:'app-video',role:'consumer',title:'Packets → owned frames → policy',steps:[
    {example:'checked_video',file:'docs/examples/checked_video.rs',api:['video','framePolicy']},
    {example:'checked_video_storage',file:'docs/examples/checked_video.rs',api:['operationStorage']},
    {example:'checked_video_rule',file:'docs/examples/checked_video.rs',api:['frameResolution']},
  ]},
  {id:'app-density',role:'consumer',title:'Require resolution; handle refusal',steps:[
    {example:'checked_resolution',file:'docs/examples/checked_operations.rs',api:['operationEncode','requiredResolution','resolutionValues']},
  ]},
  {id:'app-collection',role:'consumer',title:'Inspect images → select JXL layers',steps:[
    {example:'checked_first_description',file:'perf/spatial-validation/src/collection.rs',api:['checkedCollection','collectionDescription']},
    {example:'checked_jxl_layer_first',file:'perf/spatial-validation/src/collection.rs',api:['jxlLayerSelection','layerPlacement'],state:'Native JXL adapter qualification'},
  ]},
  {id:'app-limits',role:'consumer',title:'Bound input → cancel → inspect errors',steps:[
    {example:'limits',file:'docs/examples/journeys.rs',api:['decodeJob','findCause'],state:'Lower-level configured decode'},
  ]},
  {id:'app-animation',role:'consumer',title:'Animation ↔ video; preserve exact timing',steps:[
    {scope:'fn animation_round_trips_through_the_video_model_with_exact_timing()',file:'zencodec-media-testkit/tests/contracts.rs',api:['animation','video'],state:'Media conformance test'},
  ]},
  {id:'app-layout',role:'consumer',title:'Plan color + layout → execute on held pixels',steps:[
    {example:'native_required_layout',file:'perf/spatial-validation/src/jxl_planned_color_tests.rs',api:['imageOutput'],external:['output-plan-api.json'],state:'Native JXL + zenpixels-convert qualification'},
  ]},
  {id:'adapter-input',role:'adapter',title:'Slice → positional reads → batched ranges',steps:[
    {example:'slice',file:'docs/examples/journeys.rs',api:['sliceDriver']},
    {example:'positioned',file:'docs/examples/journeys.rs',api:['positionedDriver']},
    {example:'batch',file:'docs/examples/journeys.rs',api:['readAt']},
  ]},
  {id:'adapter-seek',role:'adapter',title:'ReadAt → existing Read + Seek parser',steps:[
    {example:'positional_parser',file:'docs/examples/checked_source.rs',api:['seekReader','seekCursor']},
  ]},
  {id:'adapter-control',role:'adapter',title:'Suspend → resolve → resume → exact rows',steps:[
    {scope:'fn registered_native_jpeg_stays_suspended_while_an_adapter_prepares_its_destination()',file:'perf/jpeg-policy-validation/src/control.rs',api:['operationControl','operationResolver'],state:'Native JPEG integration test'},
  ]},
  {id:'adapter-output',role:'adapter',title:'Rows → schedule → forward / seek-back output',steps:[
    {scope:'fn public_row_driver_native_schedules_preserve_pixels_owners_and_failure_context()',file:'perf/jpeg-policy-validation/src/row_driver.rs',api:['rowEncode','rowSource','rowEncodeOptions'],state:'Native JPEG integration test'},
  ]},
  {id:'adapter-planning',role:'adapter',title:'Plan before begin: native contract gap',issue:160,evidence:'docs/encode-output-preparation.md',note:'Measured 2026-10-27: four prepared progressive JPEG jobs peaked at 77,485 heap bytes; beginning them peaked at 58,070,699 bytes before pixel reads. The native template plan also misses wrapper overrides. A checked destination path needs effective-job planning before startup.',steps:[
    {scope:'fn native_template_plan_does_not_describe_adapter_streaming_overrides()',file:'perf/jpeg-policy-validation/src/preparation.rs',api:['operationPreparation','rasterOutput'],state:'Native planning counterexample; replacement hook not implemented'},
    {scope:'fn explicit_native_and_progressive_settings_override_streaming_preference()',file:'perf/jpeg-policy-validation/src/preparation.rs',api:['outputScheduleFacts'],state:'Native override and small-image contract test'},
  ]},
  {id:'adapter-ownership',role:'adapter',title:'Admit storage → hold → publish / refuse',steps:[
    {example:'checked_output_storage',file:'perf/spatial-validation/src/jxl_packaged_storage.rs',api:['preparedStorage','imageOutput'],state:'Native JXL integration test'},
  ]},
  {id:'adapter-preflight',role:'adapter',title:'Plan the effective job → validate → begin',issue:160,evidence:'docs/encode-output-preparation.md',note:'Private native JPEG implementation experiment: requires the committed native-preflight.patch and explicit test feature. This is not yet a zencodec method. Planning agrees with actual startup across 952 mode/geometry cases; the common checked destination contract still needs frozen jobs, input protocol and policy obligations.',steps:[
    {scope:'fn planning_is_repeatable_and_does_not_change_strided_encode_bytes()',file:'perf/jpeg-policy-validation/src/native_preflight.rs',api:['operationPreparation','rasterOutput'],state:'Patched native JPEG preflight; byte-preserving strided control'},
    {scope:'fn current_color_context_is_checked_before_startup()',file:'perf/jpeg-policy-validation/src/native_preflight.rs',api:['checkedEncodePolicy'],state:'Patched native JPEG preflight; current color context refusal'},
  ]},
  {id:'adapter-scope',role:'adapter',title:'Keep obligations → inspect late metadata → publish',steps:[
    {scope:'impl<\'a> DeferredRewrite<\'a>',file:'perf/jpeg-policy-validation/src/requirement_scope.rs',api:['pendingPolicyPlan'],state:'Private native JPEG contract experiment'},
  ]},
  {id:'adapter-children',role:'adapter',title:'Top-level images → child / SubIFD selection',steps:[
    {scope:'pub fn first_child<C: Collection>',file:'perf/spatial-validation/src/collection.rs',api:['collectionSelection'],state:'Shared collection selection helper'},
  ]},
  {id:'adapter-replay',role:'adapter',title:'Retain raster → replay exact rows',steps:[
    {scope:'fn driver_retained_raster_is_exact_without_digest_or_raw_staging()',file:'perf/multipass-validation/tests/proof_substitution.rs',api:['multipassInput','multipassEncode','multipassEncoder'],state:'Private multipass contract experiment'},
  ]},
  {id:'adapter-patches',role:'adapter',title:'Finish → validate tail + patches → apply',steps:[
    {scope:'fn finish_preflight_supports_tail_patches_and_ordered_overlaps_and_rejects_overflow()',file:'zencodec-media-testkit/tests/contracts.rs',api:['muxFinish','muxFinishApply'],state:'Media output contract test'},
  ]},
  {id:'adapter-metadata',role:'adapter',title:'Inspect EXIF / XMP → diff → report unknowns',steps:[
    {scope:'fn main()',file:'zenmeta/examples/audit_metadata.rs',api:['metadataAudit'],state:'Metadata audit CLI example'},
    {scope:'fn duplicates_spoofing_and_qualifiers_are_not_silently_selected()',file:'zenmeta/tests/metadata_inspection.rs',api:['xmpPacket'],state:'XMP ambiguity contract test; not whole-file PII certification'},
  ]},
  {id:'codec-config',role:'implementor',title:'EncoderConfig → EncodeJob → Encoder',steps:[
    {scope:'impl EncoderConfig for MinimalEncoderConfig',file:'zencodec-testkit/src/minimal.rs',api:['encoderConfig','codecErrorBuild'],state:'Internal testkit fixture; requires reference.rs helpers'},
    {scope:'impl EncodeJob for MinEncodeJob',file:'zencodec-testkit/src/minimal.rs',api:['encodeJob','checkedEncodePolicy'],state:'Testkit codec: minimal supported capabilities'},
    {scope:'impl Encoder for MinEnc',file:'zencodec-testkit/src/minimal.rs',api:['rasterOutput'],state:'Testkit codec: one-shot encode'},
  ]},
  {id:'codec-decode',role:'implementor',title:'DecoderConfig → DecodeJob → Decode',steps:[
    {scope:'impl DecoderConfig for MinimalDecoderConfig',file:'zencodec-testkit/src/minimal.rs',api:['decoder'],state:'Testkit codec: minimal supported capabilities'},
    {scope:"impl<'a> DecodeJob<'a> for MinDecodeJob",file:'zencodec-testkit/src/minimal.rs',api:['decodeJob','checkedDecodePolicy'],state:'Testkit codec: minimal supported capabilities'},
    {scope:"impl Decode for MinDec<'_>",file:'zencodec-testkit/src/minimal.rs',api:['decode'],state:'Testkit codec: one-shot decode'},
  ]},
  {id:'codec-sansio',role:'implementor',title:'Request bytes → step → write rows → Done',steps:[
    {scope:'impl IncrementalDecode for Burst',file:'zencodec-io/examples/io_contract_probes.rs',api:['incremental'],state:'Protocol test codec, not a production format'},
    {example:'bridge',file:'docs/examples/journeys.rs',api:['wholeFile'],state:'Whole-file QOI bridge; not byte-resuming reconstruction'},
  ]},
  {id:'codec-coroutine',role:'implementor',title:'Read at an offset → await → emit a tile',steps:[
    {scope:'fn minitile_decoder()',file:'zencodec-coroutine/tests/minitile.rs',api:['coroutine'],state:'Compiled MiniTile protocol test'},
  ]},
  {id:'codec-rows',role:'implementor',title:'begin → push_rows → pending_output → finish',steps:[
    {scope:'impl Encoder for Raw',file:'zencodec-io/examples/io_contract_probes.rs',api:['rasterOutput','rasterOutputTiming'],state:'Protocol test encoder'},
    {scope:'fn encoder_output_timing_depends_on_configuration_and_rows_remain_bit_exact()',file:'perf/jpeg-policy-validation/src/tests.rs',api:['rowEncode'],state:'Native JPEG parity test'},
  ]},
  {id:'codec-video',role:'implementor',title:'send ↔ receive → drain → reset',steps:[
    {scope:'impl VideoDecoder for ToyDecoder',file:'zencodec-media-testkit/src/toy.rs',api:['video'],state:'Media conformance codec'},
    {scope:'impl VideoEncoder for ToyEncoder',file:'zencodec-media-testkit/src/toy.rs',api:['videoEncoderFidelity'],state:'Media conformance codec'},
  ]},
  {id:'codec-testkit',role:'implementor',title:'Prove capabilities and exact cross-path pixels',steps:[
    {scope:'fn reference_codec_is_honest_in_the_true_direction()',file:'zencodec-testkit/tests/sansio_checks.rs',api:['conformance'],state:'Executing conformance tests'},
  ]},
  {id:'codec-collection',role:'implementor',title:'Expose TIFF inventory through Collection',steps:[
    {scope:'impl<R: Read + Seek> Collection for TiffCollection<R>',file:'perf/spatial-validation/src/tiff_items.rs',api:['collectionSelection','collectionDescription'],state:'Native TIFF collection adapter experiment'},
  ]},
  {id:'codec-container',role:'implementor',title:'Container bytes ↔ packets; seek and finish',steps:[
    {scope:'impl Demuxer for ToyDemuxer',file:'zencodec-media-testkit/src/toy.rs',api:['demuxer'],state:'Media conformance container'},
    {scope:'impl Muxer for ToyMuxer',file:'zencodec-media-testkit/src/toy.rs',api:['muxer'],state:'Media conformance container'},
  ]},
];

// Editorial context is separate from the extracted, compilable source.
// Each addition answers a concrete need; a test fixture is never presented as
// the application entry point for a production codec.
export const roleStories = {
  consumer: 'Build a photo gallery: open an upload, produce a derivative, then add display, metadata and transport requirements only as the product needs them.',
  adapter: 'Connect that gallery to storage and a destination: retain the real source and codec across decisions, and account for effects when an operation fails.',
  implementor: 'Expose an existing codec: start with a still image, then add byte acquisition, row output, collections or video where the codec can honor the contract.',
};
const lessons = {
  'adapter-declaration': [['Keep one job across admission', 'Declare actual color and destination access before native startup. Plan the retained job, admit the route, then compare every startup fact before reading pixels or publishing bytes. This fixture checks forwarding; the checked driver must enforce the sequence for callers.']],
  'codec-preflight': [
    ['Plan without native image state', 'Use the effective job settings and the complete declaration. Return native errors intact; a codec without preflight explicitly refuses.'],
    ['Start with the same interpretation', 'Share the effective planning implementation with startup. A matching lag alone cannot prove matching input acquisition or color carriers.'],
  ],
  'app-start': [
    ['Choose the formats once', 'The gallery accepts JPEG and PNG and writes PNG. Register those implementations once; pass the set to each operation. with_send_encoder registers jobs that may move between threads for the later async path; with_encoder also exists for thread-affine jobs. Send does not mean writing to a destination.'],
    ['Open one upload', 'The caller has encoded bytes and wants their native pixels. DecodeRequest detects the format; the result retains pixel interpretation and the operation report.'],
    ['Save a derivative', 'Now the gallery needs a PNG file. TranscodeRequest belongs to zencodecs, which composes the codecs and conversion; unsupported preservation can refuse.'],
  ],
  'app-render': [
    ['Open the first display image', 'A collection can contain pages, children or layers. Select through the common collection interface before assuming that the first stored raster is a complete canvas.'],
    ['Render from a remote snapshot', 'The viewer needs a display result from positional reads. This native test checks the full read → select → render path and rejects changed or faulty sources.'],
  ],
  'app-convert': [
    ['Choose the display color', 'Native pixels need not match the display. Keep their current color context and let zenpixels-convert prepare the requested output interpretation.'],
    ['Map the framebuffer', 'The display needs rows in a specific layout. Borrow the returned owner and honor its stride; do not infer packed storage from width.'],
  ],
  'app-transcode': [
    ['Publish a web derivative', 'The output now needs named sRGB and descriptive-metadata removal. These are explicit requirements; alpha or depth loss must not be silently substituted.'],
    ['Add a tenant decision', 'Only this tenant requires native workspace evidence. The callback chooses among admitted routes or returns a located application error; unknown is not zero.'],
  ],
  'app-io': [
    ['Read a file positionally', 'The source is on disk instead of in a byte slice. Keep the same transcode request and supply FileReadAt.'],
    ['Await a network source', 'The source can suspend for I/O. AsyncSource drives acquisition; this example still returns completed encoded bytes before the application sends them.'],
  ],
  'app-rewrite': [
    ['Change metadata without pixel reconstruction', 'The product needs removal, not a new image encode. RewriteRequest retains the codestream and applies the selected metadata treatment.'],
    ['Handle an uninspectable profile', 'Opaque ICC bytes may contain fields the privacy rule cannot inspect. Refuse rather than treating an unparsed payload as proven clean.'],
  ],
  'app-video': [
    ['Gate decoded frame publication', 'The source is video. Wrap the native decoder once so every returned frame passes through the shared policy.'],
    ['Bound native picture allocations', 'A publication decision happens after reconstruction. This native setting adds picture-plane admission before allocation; it is not a total-RAM bound.'],
    ['Check each frame interpretation', 'A later frame can carry different color facts. This callback refuses an unsupported transfer before publication; it does not perform tone mapping.'],
  ],
  'app-density': [['Require a physical resolution', 'A print consumer relies on exact density. Ask for it on EncodeRequest and handle refusal when the format cannot represent it exactly.']],
  'app-collection': [
    ['Inspect before decoding', 'A large collection may contain images the caller never wants. Read the selected description before allocating its pixels.'],
    ['Select a JXL layer deliberately', 'The application now edits layers. Preserve layer placement and distinguish a layer raster from a composited presentation.'],
  ],
  'app-limits': [['Bound and cancel an operation', 'An upload is untrusted or the user closes the viewer. Configure resource limits and a Pulse, and retain the typed cause when work refuses or stops.']],
  'app-animation': [['Keep exact animation timing', 'An animation enters a video pipeline. The conformance test preserves rational timing through both adapters rather than rounding it at the boundary.']],
  'app-layout': [['Require both layout and color', 'A native image has auxiliary channels and a particular color interpretation. Prepare one output plan and execute it on the held source; layout conversion alone is insufficient.']],
  'adapter-input': [
    ['Drive bytes already in memory', 'The application has a complete slice. The codec requests input through the same sans-I/O dialogue used by other sources.'],
    ['Supply only requested ranges', 'The input now lives on disk or behind ranged reads. The driver fulfills InputRequest without teaching the codec about files or HTTP.'],
    ['Coalesce nearby ranges', 'Remote requests have latency. Batch useful reads while retaining their offsets and EOF semantics.'],
  ],
  'adapter-seek': [['Reuse a seek-based parser', 'An existing parser expects Read + Seek. The adapter maps that interface onto positional acquisition while keeping source identity and errors visible.']],
  'adapter-control': [['Hold the decoder at its decision boundary', 'Interpretation becomes available before affected rows may escape. Retain the suspended native JPEG decoder while the destination is prepared.']],
  'adapter-output': [['Choose when the destination may wait', 'A slow writer may retain a large encoder. Offer finish-first or qualified live output, keep the actual source and encoder, and preserve partial-write facts.']],
  'adapter-planning': [
    ['Find the cost of planning by startup', 'Beginning every candidate can allocate uncompressed codec state before the caller chooses. The native counterexample measures that cost.'],
    ['Include effective wrapper overrides', 'A codec template is not the configured job. Progressive mode, native color mode and small geometry change the route that can actually execute.'],
  ],
  'adapter-ownership': [['Reserve storage before reconstruction', 'The caller holds several images at once. The allocation charge must follow the returned owner, including after the decoder is dropped.']],
  'adapter-preflight': [
    ['Plan without beginning native work', 'This private JPEG patch validates the effective job before allocating pixel state. The test also checks repeated planning leaves strided encode bytes unchanged.'],
    ['Validate current color before headers', 'The declared samples and attached profile must agree before startup writes color headers. These tests cover the patch’s current refusal scope, not all color transforms.'],
  ],
  'adapter-scope': [['Keep a late metadata obligation alive', 'An early decision cannot certify packets not yet seen. This private experiment retains the obligation until later inspection and final-effect admission.']],
  'adapter-children': [['Select a child image', 'A TIFF thumbnail or pyramid level is a child, not another animation frame. Use explicit child selection and preserve the parent relationship.']],
  'adapter-replay': [['Retain the exact raster for another pass', 'The codec needs analysis before coding. Driver-held immutable pixels provide replay identity; a caller-supplied digest alone does not.']],
  'adapter-patches': [['Apply final seek-back patches', 'The container learns offsets or sizes at finish. Validate the full tail and patch plan before applying ordered writes to the actual destination.']],
  'adapter-metadata': [
    ['Inspect metadata with explicit coverage', 'The application wants an audit before choosing a treatment. Report recognized fields alongside uninspected data.'],
    ['Refuse ambiguous XMP meaning', 'Duplicate fields or misleading namespaces prevent a unique interpretation. The test keeps those unknowns visible instead of selecting a convenient value.'],
  ],
  'codec-config': [
    ['Expose a reusable configuration', 'Register format identity and accepted descriptors. This minimal test codec supplies a small implementation to read before a production adapter.'],
    ['Bind one operation', 'Keep Pulse, limits and requested metadata on the job. Reject unsupported requirements rather than silently accepting setters.'],
    ['Accept one image', 'The simplest executor consumes PixelSlice, including its stride and interpretation, and returns encoded output or the codec’s own error.'],
  ],
  'codec-decode': [
    ['Register a decoder', 'Expose format detection and actual supported behavior once on DecoderConfig.'],
    ['Prepare one decode', 'Bind source lifetime, Pulse, limits and output negotiation on DecodeJob.'],
    ['Return interpreted pixels', 'Decode owns the reconstructed result. Keep its descriptor, metadata and pixel storage consistent.'],
  ],
  'codec-sansio': [
    ['Ask the driver for bytes', 'IncrementalDecode consumes supplied bytes and names the next InputRequest. The protocol fixture makes consumption and row ownership explicit.'],
    ['Bridge a whole-file codec explicitly', 'A codec that needs all bytes can use a whole-file bridge. This preserves the dialogue but does not make reconstruction incremental.'],
  ],
  'codec-coroutine': [['Write straight-line incremental decoding', 'A codec author can await requested bytes without choosing a runtime. The coroutine adapter exposes the same suspended state to the driver.']],
  'codec-rows': [
    ['Declare and drain row output', 'A row-capable encoder validates the image, accepts strided batches and exposes pending bytes. Timing describes bytes, not retained raw memory.'],
    ['Prove configuration-dependent behavior', 'The native JPEG test compares pixels across row and one-shot paths while varying output timing. A protocol fixture alone cannot establish this.'],
  ],
  'codec-video': [
    ['Decode through send and receive', 'A decoder can retain packets or delay pictures. The conformance codec makes backpressure, drain and ownership visible.'],
    ['Encode through send and receive', 'An encoder can delay output independently of input frames. Expose actual fidelity and preserve state-machine errors.'],
  ],
  'codec-testkit': [['Check promises against behavior', 'Run both supported and unsupported directions, then compare exact pixels across feeding paths. A capability flag alone is not evidence.']],
  'codec-collection': [['Expose native TIFF structure', 'Map pages and child images through Collection while retaining native range access. This remains an explicitly labeled adapter experiment.']],
  'codec-container': [
    ['Demux encoded packets', 'Container parsing requests bytes and emits packets with timing and provenance; pixel decoding is a separate stage.'],
    ['Mux and finalize', 'Packet acceptance, delayed output and final patches belong to the container state machine. Keep destination effects visible on error.'],
  ],
};
for (const flow of codeFlows) {
  const steps = lessons[flow.id];
  if (!steps || steps.length !== flow.steps.length) throw Error(`Missing graduated explanation: ${flow.id}`);
  flow.steps.forEach((step, index) => { [step.title, step.why] = steps[index]; });
}
