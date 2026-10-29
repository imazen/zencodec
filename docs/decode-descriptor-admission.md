# Required decode output: remaining admission gap

`DecodeRequest::with_output` passes the required descriptor as the sole preferred
format, then validates actual output after `DynDecoder::decode`. A mismatch
never escapes as success, but an unwanted full raster may already be allocated.
Controlled decoding checks at final interpretation before policy/sink effects;
that does not prove the one-shot path has the same allocation boundary.

Two tempting shortcuts are not equivalent contracts:

- `DecodeJob::output_info(data)` receives no format preferences. At
  [zenjpeg26cbe9b8](https://github.com/imazen/zenjpeg/blob/26cbe9b8c73c975762363c314ebbf7e888caa2db/zenjpeg/src/codec/decode.rs#L448)
  it calls `decode_descriptor(&[], ...)`, while the actual decoder stores the
  caller's preferences and selects output during execution. Comparing that
  prediction to a required negotiated format could reject a valid decode.
- `push_decode` can reject a sink descriptor before its allocation on native
  streaming paths, but returns OutputInfo rather than the full DecodeOutput.
  Switching execution paths must also preserve metadata, gain-map components,
  source precision evidence and other owned extras. Whole-file push fallback
  still decodes first, so choosing push alone cannot promise early refusal.

The next implementation must retain the same configured native owner, distinguish
unknown/late descriptor facts from a committed selected descriptor, and keep the
final output check. It must not use a capability list as proof of the selected
output or silently narrow the result to pixels plus basic image information.
Qualify negotiated RGB/RGBA and U8/F32, late color/profile changes, HDR gain-map
reconstruction, native errors, metadata/extras parity, and allocation before
refusal. Default decode remains successful without mandatory caller queries.

This is a source-verified open contract, not a completed preflight implementation.
No new public planning API was introduced from this review alone.
