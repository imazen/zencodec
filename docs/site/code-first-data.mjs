// Every code body is extracted from a compiled source file, never transcribed.
export const codeFlows = [
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
  {id:'app-transcode',role:'consumer',title:'Transcode → metadata → custom policy',note:'Current contract gap: direct Decision::Choose can bypass PolicySet requirements. Do not treat this callback composition as a privacy guarantee. Mandatory admission is tracked in issue 173; the code here deliberately matches the current review revision.',steps:[
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
  {id:'adapter-ownership',role:'adapter',title:'Admit storage → hold → publish / refuse',steps:[
    {example:'checked_output_storage',file:'perf/spatial-validation/src/jxl_packaged_storage.rs',api:['preparedStorage','imageOutput'],state:'Native JXL integration test'},
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
    {scope:'impl EncoderConfig for MinimalEncoderConfig',file:'zencodec-testkit/src/minimal.rs',api:['encoderConfig'],state:'Testkit codec: minimal supported capabilities'},
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
