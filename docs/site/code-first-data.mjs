// Every code body is extracted from a compiled source file, never transcribed.
export const codeFlows = [
  {id:'app-start',role:'consumer',manifest:'perf/jpeg-policy-validation/Cargo.toml',title:'Register → decode → transcode',steps:[
    {example:'checked_registry',file:'docs/examples/checked_operations.rs',api:['operationRegistration']},
    {example:'checked_decode',file:'docs/examples/checked_operations.rs',api:['operationDecode','operationOutcome','decodedOutput']},
    {example:'checked_default',file:'docs/examples/checked_operations.rs',api:[],external:['integration-api.json']},
    {example:'checked_png_file',file:'docs/examples/checked_operations.rs',api:['operationRegistration','operationOutcome','encodedOutput'],external:['integration-api.json'],state:'Native JPEG/PNG cancellation and publication controls'},
  ]},
  {id:'adapter-input',role:'adapter',title:'Slice → positional reads → batched ranges',steps:[
    {example:'slice',file:'docs/examples/journeys.rs',api:['sliceDriver']},
    {example:'positioned',file:'docs/examples/journeys.rs',api:['positionedDriver']},
    {example:'batch',file:'docs/examples/journeys.rs',api:['readAt']},
  ]},
  {id:'codec-config',role:'implementor',title:'EncoderConfig → EncodeJob → Encoder',project:{manifest:'zencodec-testkit/Cargo.toml',helpers:'zencodec-testkit/src/reference.rs',command:'cargo test -p zencodec-testkit --lib --test usage'},steps:[
    {scope:'impl EncoderConfig for MinimalEncoderConfig',file:'zencodec-testkit/src/minimal.rs',api:['encoderConfig','codecErrorBuild'],state:'Internal testkit fixture; requires reference.rs helpers'},
    {scope:'impl EncodeJob for MinEncodeJob',file:'zencodec-testkit/src/minimal.rs',api:['encodeJob','checkedEncodePolicy'],state:'Testkit codec: minimal supported capabilities'},
    {scope:'impl Encoder for MinEnc',file:'zencodec-testkit/src/minimal.rs',api:['rasterOutput','encodedOutput'],state:'Testkit codec: one-shot encode'},
  ]},
  {id:'app-bound-policy',role:'consumer',title:'Preset → situational callback → reusable callback object',note:'One policy argument carries mandatory requirements. The callback sees only admitted choices. These executable controls show refusal diagnostics, choice indices and a logging wrapper; native workspace evidence and operation-wide deferred obligations remain separate qualifications.',steps:[
    {scope:'fn direct_preset_names_unknown_wait_and_contradictory_schedule()',file:'zencodec-testkit/tests/policy_binding.rs',api:['operationPreset','operationPolicy'],state:'Direct preset: refuse before effects and name the unmet requirement'},
    {scope:'fn bound_direct_choice_sees_only_compliant_original_indices()',file:'zencodec-testkit/tests/policy_binding.rs',api:['boundPolicy','operationResolver'],state:'Callback: select only among compliant routes'},
    {scope:'fn bound_requirements_survive_a_callback_only_logging_wrapper()',file:'zencodec-testkit/tests/policy_binding.rs',api:['boundPolicy','operationDecisionRecord'],state:'Custom callback wrapper: requirements remain outside the wrapper'},
  ]},
  {id:'adapter-declaration',role:'adapter',title:'Declare color → plan without startup → verify start',note:'Shared trait hooks are implemented and tested. Native JPEG shared-hook adoption is qualified. PNG planning qualification and the full checked EncodeRequest-to-destination convenience remain separate gates. Carrier evidence can be unknown; planning does not establish a native memory bound.',steps:[
    {scope:'fn erased_preflight_retains_job_color_access_and_native_error()',file:'src/traits/dyn_encoding.rs',api:['encodeDeclaration','encodePlan','encodePreflight','declaredBegin'],state:'Shared contract test; scripted codec, not native qualification'},
  ]},
  {id:'adapter-workspace',role:'adapter',title:'Check before preparation → keep later requirements',evidence:'docs/spec.md#workspace-deadlines-before-native-preparation',note:'A later route refusal cannot undo native workspace already allocated. This adapter helper checks the existing workspace requirement before preparation. It does not select a route or discharge metadata, publication or other requirements. Native entry-order qualification is separate.',steps:[
    {scope:'pub fn check_workspace(',file:'src/operation/policy/requirements.rs',api:['operationPreset'],state:'Adapter hook: unknown is not zero, and failure retains named evidence'},
    {scope:'fn early_workspace_success_does_not_discharge_metadata_or_final_adoption()',file:'src/operation/policy/requirements.rs',api:['operationPreset','operationPolicy'],state:'Shared control: an early success cannot authorize later publication'},
  ]},
  {id:'adapter-precision',role:'adapter',title:'Declare sample depth → verify rows → preserve failure effects',evidence:'docs/precision-admission.md',note:'A depth claim describes current canonical samples; it does not certify native codec fidelity. Constructor defaults stay unclaimed. With a claim, the driver checks each complete strip before push. FinishFirst withholds final publication on a late contradiction; Live may already have published earlier bytes.',steps:[
    {scope:'fn description_defaults_builders_debug_and_eq_include_evidence()',file:'zencodec-io/tests/planned_row_encode/exact_depth.rs',api:['encodeDeclaration','plannedRowEncode'],state:'Start without a claim; add one explicitly'},
    {scope:'fn false_initial_and_later_claims_never_push_a_partial_bad_strip_or_retry()',file:'zencodec-io/tests/planned_row_encode/exact_depth.rs',api:['plannedRowEncode'],state:'Executed strip-verification and publication controls'},
  ]},
  {id:'codec-preflight',role:'implementor',title:'One effective plan → declared startup',steps:[
    {scope:'impl EncodeJob for Job',file:'src/traits/dyn_encoding.rs',api:['encodePreflight'],state:'Shared contract fixture: planning without encoder construction'},
    {scope:'impl Encoder for Declared',file:'src/traits/dyn_encoding.rs',api:['declaredBegin'],state:'Shared contract fixture: current color reaches startup'},
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
    {example:'checked_registry_pulse',file:'docs/examples/checked_operations.rs',api:['operationRegistration'],state:'Native JPEG/PNG cancellation and publication controls'},
  ]},
  {id:'app-animation',role:'consumer',title:'Animation ↔ video; preserve exact timing',steps:[
    {scope:'fn animation_round_trips_through_the_video_model_with_exact_timing()',file:'zencodec-media-testkit/tests/contracts.rs',api:['animation','video'],state:'Media conformance test'},
  ]},
  {id:'app-layout',role:'consumer',title:'Plan color + layout → execute on held pixels',steps:[
    {example:'native_required_layout',file:'perf/spatial-validation/src/jxl_planned_color_tests.rs',api:['imageOutput'],external:['output-plan-api.json'],state:'Native JXL + zenpixels-convert qualification'},
  ]},
  {id:'adapter-seek',role:'adapter',title:'ReadAt → existing Read + Seek parser',steps:[
    {example:'positional_parser',file:'docs/examples/checked_source.rs',api:['seekReader','seekCursor']},
  ]},
  {id:'adapter-control',role:'adapter',title:'Suspend → resolve → resume → exact rows',steps:[
    {scope:'fn registered_native_jpeg_stays_suspended_while_an_adapter_prepares_its_destination()',file:'perf/jpeg-policy-validation/src/control.rs',api:['operationControl','operationResolver'],state:'Native JPEG integration test'},
  ]},
  {id:'adapter-registry-output',role:'adapter',title:'Registry job → plan → writer or seek-back destination',note:'The shared planned driver accepts the erased registry job directly. Policy precedes native construction; current color reaches startup. These controls use a scripted codec. Native pipeline adoption remains a separate gate.',steps:[
    {scope:'fn registry_job_to_writer<W: io::Write>',file:'zencodec-io/tests/planned_row_encode.rs',api:['plannedRowEncode','rowRaster','rowEncodeOptions'],state:'Compiled registry-to-writer usage with an executable control'},
    {scope:'fn erased_destination_preserves_negotiated_patches_and_forward_refusal()',file:'zencodec-io/tests/planned_row_encode.rs',api:['plannedRowEncode'],state:'Shared driver regression: forward refusal and seek-back patches'},
  ]},
  {id:'adapter-output',role:'adapter',title:'Rows → schedule → forward / seek-back output',steps:[
    {scope:'fn public_row_driver_native_schedules_preserve_pixels_owners_and_failure_context()',file:'perf/jpeg-policy-validation/src/row_driver.rs',api:['rowEncode','rowSource','rowEncodeOptions'],state:'Native JPEG integration test'},
  ]},
  {id:'adapter-planning',role:'adapter',title:'Why effective-job planning precedes begin',issue:160,evidence:'docs/encode-output-preparation.md',note:'Measured 2026-10-27: four prepared progressive JPEG jobs peaked at 77,485 heap bytes; beginning them peaked at 58,070,699 bytes before pixel reads. The native template plan also misses wrapper overrides. A checked destination path needs effective-job planning before startup.',steps:[
    {scope:'fn native_template_plan_does_not_describe_adapter_streaming_overrides()',file:'perf/jpeg-policy-validation/src/preparation.rs',api:['operationPreparation','rasterOutput'],state:'Native template-planning counterexample; shared effective-job hooks available'},
    {scope:'fn explicit_native_and_progressive_settings_override_streaming_preference()',file:'perf/jpeg-policy-validation/src/preparation.rs',api:['outputScheduleFacts'],state:'Native override and small-image contract test'},
  ]},
  {id:'adapter-ownership',role:'adapter',title:'Admit storage → hold → publish / refuse',steps:[
    {example:'checked_output_storage',file:'perf/spatial-validation/src/jxl_packaged_storage.rs',api:['preparedStorage','imageOutput'],state:'Native JXL integration test'},
  ]},
  {id:'adapter-preflight',role:'adapter',title:'Plan the effective job → validate → begin',issue:160,evidence:'docs/encode-output-preparation.md',note:'Native JPEG shared-hook qualification: the pinned adapter implements EncodeJob::plan_encode and Encoder::begin_declared. The native-preflight feature selects the tests; no native-preflight.patch is applied. The checked mode/geometry matrix retains its 952-case assertion. Planning is not a native memory bound, and the full EncodeRequest-to-destination convenience remains a separate gate.',steps:[
    {scope:'fn planning_is_repeatable_and_does_not_change_strided_encode_bytes()',file:'perf/jpeg-policy-validation/src/native_preflight.rs',api:['operationPreparation','rasterOutput'],state:'Native JPEG shared planning hooks; byte-preserving strided control'},
    {scope:'fn current_color_context_is_checked_before_startup()',file:'perf/jpeg-policy-validation/src/native_preflight.rs',api:['checkedEncodePolicy'],state:'Native JPEG shared planning hooks; current color context refusal'},
  ]},
  {id:'adapter-scope',role:'adapter',title:'Keep obligations → inspect late metadata → publish',steps:[
    {scope:'impl<\'a> DeferredRewrite<\'a>',file:'perf/jpeg-policy-validation/src/requirement_scope.rs',api:['pendingPolicyPlan'],state:'Private native JPEG contract experiment'},
  ]},
  {id:'adapter-children',role:'adapter',title:'Top-level images → child / SubIFD selection',steps:[
    {scope:'pub fn first_child<C: Collection>',file:'perf/spatial-validation/src/collection.rs',api:['collectionSelection'],state:'Shared collection selection helper'},
  ]},
  {id:'adapter-replay',role:'adapter',title:'Configure once → replay rows → release before publication',evidence:'docs/row-replay-driver.md',note:'The public retained-job driver executes Rows and Replay. Real IMI controls pass for typed/configuration-erased/configured-job-erased calls, padded U8/U16 input, four acquisition routes and forward/seek-back output. IMI still retains compressed output until finish. Strict unknown metadata/workspace evidence refuses; this is not a whole-operation memory or privacy certificate.',steps:[
    {scope:'pub fn encode_job<J: EncodeJob, S: Source, W: Write, R: OperationPolicy>(',file:'zencodec-io/src/row_encode.rs',api:['plannedRowEncode','rowSource','rowEncodeOptions'],state:'Public driver: retain the configured job through admission'},
    {scope:'fn configured_job_erasure_keeps_prepared_owner_without_startup_or_clone()',file:'src/traits/dyn_encoding/job_erasure_tests.rs',api:['encodePreflight'],state:'Executed ownership control: move the configured job before startup'},
    {scope:'fn native_retained_jobs_match_in_place_bytes_across_public_routes_and_dispatches()',file:'perf/multipass-validation/native/tests/public_driver.rs',api:['plannedRowEncode','encodeRows','rowDemand'],state:'Executed native/public-driver parity matrix; same-native byte/readback controls'},
  ]},
  {id:'adapter-patches',role:'adapter',title:'Finish → validate tail + patches → apply',steps:[
    {scope:'fn finish_preflight_supports_tail_patches_and_ordered_overlaps_and_rejects_overflow()',file:'zencodec-media-testkit/tests/contracts.rs',api:['muxFinish','muxFinishApply'],state:'Media output contract test'},
  ]},
  {id:'adapter-metadata',role:'adapter',title:'Inspect EXIF / XMP → diff → report unknowns',steps:[
    {scope:'fn main()',file:'zenmeta/examples/audit_metadata.rs',api:['metadataAudit'],state:'Metadata audit CLI example'},
    {scope:'fn duplicates_spoofing_and_qualifiers_are_not_silently_selected()',file:'zenmeta/tests/metadata_inspection.rs',api:['xmpPacket'],state:'XMP ambiguity contract test; not whole-file PII certification'},
  ]},
  {id:'codec-decode',role:'implementor',title:'DecoderConfig → DecodeJob → Decode',project:{manifest:'zencodec-testkit/Cargo.toml',helpers:'zencodec-testkit/src/reference.rs',command:'cargo test -p zencodec-testkit --lib --test usage'},steps:[
    {scope:'impl DecoderConfig for MinimalDecoderConfig',file:'zencodec-testkit/src/minimal.rs',api:['decoder'],state:'Testkit codec: minimal supported capabilities'},
    {scope:"impl<'a> DecodeJob<'a> for MinDecodeJob",file:'zencodec-testkit/src/minimal.rs',api:['decodeJob','checkedDecodePolicy'],state:'Testkit codec: minimal supported capabilities'},
    {scope:"impl Decode for MinDec<'_>",file:'zencodec-testkit/src/minimal.rs',api:['decode','decodedOutput'],state:'Testkit codec: one-shot decode'},
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
  'app-bound-policy': [
    ['Start with a preset', 'The gallery has a fixed output rule: do not wait on a destination without a qualified retained-memory bound. Pass PolicySet directly. This executable control checks that unknown evidence refuses and names OutputWait before work starts.'],
    ['Choose among admitted routes', 'The gallery later needs a situational choice. Add BoundPolicy::new(requirements, callback); required limits still filter the offers first. The callback selects using the current facts without repeating those checks.'],
    ['Keep tracing outside the requirements', 'The application now wraps its callback to count or log decisions. Put that wrapper inside BoundPolicy so forwarding the callback cannot forget mandatory requirements. Reuse the binding through a mutable borrow; operation progress stays with each operation.'],
  ],
  'adapter-registry-output': [
    ['Send the configured registry job to a writer', 'The gallery already chose and configured an encoder. Pass that job directly to the planned driver; it retains the job through admission and starts with the current pixel color. The example uses a small test output budget, not a production recommendation.'],
    ['Allow patches only on an actual seek-back destination', 'A layout that overwrites earlier bytes must refuse a forward writer before startup. When the destination can seek, preserve the final patch plan and account for appended and overwritten bytes separately.'],
  ],
  'adapter-workspace': [
    ['Reject an impossible limit before native work', 'The gallery adds a workspace cap. The adapter checks both operation-entry requirement snapshots before constructing native jobs or decoders. Pass None when the complete covered workspace is unknown; even the largest cap must then refuse. This is an adapter responsibility, not another caller capability query.'],
    ['Keep the remaining decisions intact', 'A known workspace bound does not prove metadata treatment or make final pixels safe to publish. Retain the same requirements through the joint decision. This control shows metadata mismatch and bare final adoption still refusing after a successful workspace check.'],
  ],
  'adapter-precision': [
    ['Keep the ordinary path simple', 'The image service starts with Description::new and no precision claim. An upstream source that deliberately preserves a canonical grid may attach ExactDepth; resizing or mutable processing must not reuse stale evidence. The declaration is an assertion, so structural validation alone cannot prove the samples.'],
    ['Make a late contradiction observable', 'The service now reads strips. A false claim in any row prevents the entire strip from reaching the encoder. The executable control compares early and late failures through typed, erased and raw routes, checks prior Live bytes, and confirms FinishFirst publishes nothing.'],
  ],
  'adapter-declaration': [['Keep one job across admission', 'Declare actual color and destination access before native startup. Plan the retained job, admit the route, then compare every startup fact before reading pixels or publishing bytes. This fixture checks forwarding; the checked driver must enforce the sequence for callers.']],
  'codec-preflight': [
    ['Plan without native image state', 'Use the effective job settings and the complete declaration. Return native errors intact; a codec without preflight explicitly refuses.'],
    ['Start with the same interpretation', 'Share the effective planning implementation with startup. A matching lag alone cannot prove matching input acquisition or color carriers.'],
  ],
  'app-start': [
    ['Choose the formats once', 'The gallery accepts JPEG and PNG and writes PNG. Register those implementations once; pass the set to each operation. with_send_encoder registers jobs that may move between threads for the later async path; with_encoder also exists for thread-affine jobs. Send does not mean writing to a destination.'],
    ['Open one upload', 'The caller has encoded bytes and wants their native pixels. DecodeRequest detects the format and uses the set’s default control; the result retains pixel interpretation and the operation report.'],
    ['Save a derivative', 'Now the gallery needs a PNG file. TranscodeRequest belongs to zencodecs, which composes the codecs and conversion. The default policy selects a permitted route; missing optional facts alone do not require refusal. Required preservation still applies.'],
    ['Write the completed PNG file', 'Keep the PNG request and its mandatory policy from the preceding recipes. Encode successfully before opening the destination, then check the same operation Pulse and write the completed bytes synchronously. The successful operation report remains available. A stopped or refused encode must not create or truncate the destination; an I/O failure after writing starts can leave partial bytes.'],
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
    ['Add a tenant decision', 'Only this tenant requires native workspace evidence. The callback chooses among admitted routes or returns a located application error; unknown is not zero. A later fact can require another decision before its affected effects; an earlier choice cannot discharge an unsatisfied requirement.'],
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
  'app-limits': [
    ['Bound and cancel an operation', 'An upload is untrusted or the user closes the viewer. Add resource limits and attach a SharedPulse to the set or job when the application needs cancellation or reporting. Borrowed per-call arguments accept None for no extra control; retained job controls and required limits still apply.'],
    ['Keep one cancellation source across the codecs', 'Extend the registered gallery with the pixel limit and the application’s SharedPulse. CodecSet supplies that control to each checked operation. Keep the application handle that signals cancellation. When a borrowed control is needed, pass Some(pulse) to retain reporting, phase planning and sharing as well as cancellation. Use StopOnly at the boundary for an existing legacy Stop.'],
  ],
  'app-animation': [['Keep exact animation timing', 'An animation enters a video pipeline. The conformance test preserves rational timing through both adapters rather than rounding it at the boundary.']],
  'app-layout': [['Require both layout and color', 'A native image has auxiliary channels and a particular color interpretation. Prepare one output plan and execute it on the held source; layout conversion alone is insufficient.']],
  'adapter-input': [
    ['Drive bytes already in memory', 'The application has a complete slice. The codec requests input through the same sans-I/O dialogue used by other sources. The example passes None for its optional borrowed Pulse, preserving any control retained by the decoder’s job.'],
    ['Supply only requested ranges', 'The input now lives on disk or behind ranged reads. The driver fulfills InputRequest without teaching the codec about files or HTTP. Forward the original Option unchanged; Some(pulse) carries the complete control, including reporting and sharing.'],
    ['Coalesce nearby ranges', 'Remote requests have latency. Batch useful reads while retaining their offsets and EOF semantics.'],
  ],
  'adapter-seek': [['Reuse a seek-based parser', 'An existing parser expects Read + Seek. The adapter maps that interface onto positional acquisition while keeping source identity and errors visible.']],
  'adapter-control': [['Hold the decoder at its decision boundary', 'Interpretation becomes available before affected rows may escape. Retain the suspended native JPEG decoder while the destination is prepared. A callback can select a permitted adaptation as facts arrive. Continue discovery only while affected output remains withheld within the admitted storage plan; otherwise refuse.']],
  'adapter-output': [['Choose when the destination may wait', 'A slow writer may retain a large encoder. Offer finish-first or qualified live output, keep the actual source and encoder, and preserve partial-write facts.']],
  'adapter-planning': [
    ['Find the cost of planning by startup', 'Beginning every candidate can allocate uncompressed codec state before the caller chooses. The native counterexample measures that cost.'],
    ['Include effective wrapper overrides', 'A codec template is not the configured job. Progressive mode, native color mode and small geometry change the route that can actually execute.'],
  ],
  'adapter-ownership': [['Reserve storage before reconstruction', 'The caller holds several images at once. The allocation charge must follow the returned owner, including after the decoder is dropped.']],
  'adapter-preflight': [
    ['Plan without beginning native work', 'The native JPEG shared planning hooks validate the effective job before allocating pixel state. The test also checks repeated planning leaves strided encode bytes unchanged.'],
    ['Validate current color before headers', 'The declared samples and attached profile must agree before startup writes color headers. These tests cover the patch’s current refusal scope, not all color transforms.'],
  ],
  'adapter-scope': [['Keep a late metadata obligation alive', 'An early decision cannot certify packets not yet seen. This private experiment retains the obligation until later inspection and final-effect admission.']],
  'adapter-children': [['Select a child image', 'A TIFF thumbnail or pyramid level is a child, not another animation frame. Use explicit child selection and preserve the parent relationship.']],
  'adapter-replay': [
    ['Start with the configured job', 'The image is already a raster. Pass its view and the retained job to encode_job; the actual plan selects row pushing or native pull requests. The default completes encoding before destination publication.'],
    ['Erase after native configuration', 'The application now dispatches codecs dynamically. into_dyn_job moves native settings and any retained preparation into the existing adapter without cloning or starting an encoder. The control checks ownership identity, zero startup and eventual cleanup.'],
    ['Add source replay and destination constraints', 'The input becomes forward-only or positional, or the destination permits seek-back. The same public driver admits a held raster, a private snapshot, positional digest checks or a separate raw file. The real-codec matrix checks every route against native bytes and both native readers; hashes never certify exact replay.'],
  ],
  'adapter-patches': [['Apply final seek-back patches', 'The container learns offsets or sizes at finish. Validate the full tail and patch plan before applying ordered writes to the actual destination.']],
  'adapter-metadata': [
    ['Inspect metadata with explicit coverage', 'The application wants an audit before choosing a treatment. Report recognized fields alongside uninspected data.'],
    ['Refuse ambiguous XMP meaning', 'Duplicate fields or misleading namespaces prevent a unique interpretation. The test keeps those unknowns visible instead of selecting a convenient value.'],
  ],
  'codec-config': [
    ['Expose a reusable configuration', 'An encoder emits one format: format() is singular. Register that identity and accepted descriptors. This internal test codec shows the trait shape; its complete workspace and shared helpers are linked above.'],
    ['Bind one operation', 'Read the per-operation trait shape. This internal fixture declares cancellation and limits unsupported and ignores those inputs; it is not a production control implementation. A production job must retain supported controls and reject unsupported required behavior.'],
    ['Accept one image', 'The simplest executor consumes PixelSlice, including its stride and interpretation, and returns encoded output or the codec’s own error.'],
  ],
  'codec-decode': [
    ['Register a decoder', 'A decoder may recognize several formats: formats() returns the list. Expose their detection and actual supported behavior once on DecoderConfig.'],
    ['Prepare one decode', 'Bind source lifetime, limits and output negotiation on DecodeJob. Attach an owned SharedPulse when needed. None on a later borrowed per-call argument keeps that retained control active.'],
    ['Return interpreted pixels', 'Decode owns the reconstructed result. Keep its descriptor, metadata and pixel storage consistent.'],
  ],
  'codec-sansio': [
    ['Ask the driver for bytes', 'IncrementalDecode consumes supplied bytes and names the next InputRequest. Its borrowed Pulse is optional. Preserve None through forwarding, check the retained job control, and keep the full Pulse when Some is supplied. The protocol fixture makes consumption and row ownership explicit.'],
    ['Bridge a whole-file codec explicitly', 'A codec that needs all bytes can use a whole-file bridge. This preserves the dialogue but does not make reconstruction incremental.'],
  ],
  'codec-coroutine': [['Write straight-line incremental decoding', 'A codec author can await requested bytes without choosing a runtime. The coroutine adapter exposes the same suspended state to the driver.']],
  'codec-rows': [
    ['Declare and drain row output', 'A row-capable encoder validates the image, accepts strided batches and exposes pending bytes. Timing describes bytes, not retained raw memory.'],
    ['Prove configuration-dependent behavior', 'The native JPEG test compares pixels across row and one-shot paths while varying output timing. A protocol fixture alone cannot establish this.'],
  ],
  'codec-video': [
    ['Decode through send and receive', 'A decoder can retain packets or delay pictures. Optional borrowed controls do not change backpressure, drain or ownership. None adds no per-call control and never disables retained cancellation or policy admission.'],
    ['Encode through send and receive', 'An encoder can delay output independently of input frames. Expose actual fidelity and preserve state-machine errors. Forward the supplied Option unchanged through session, pump and erasure boundaries.'],
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
