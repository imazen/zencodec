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

## Executed native controls

`just jpeg-decode-admission-check` qualifies the pinned JPEG `e0bb92b4`:
for baseline and progressive JPEG, native output prediction differs from the
requested RGBA descriptor, while the checked decode succeeds with that exact
descriptor. Rows, dimensions, orientation and descriptive metadata equal direct
native execution. A separate RGB16 requirement verifies that native fallback
still produces a typed `OutputDescriptor` refusal.

Both tests and strict harness Clippy passed on source `4f955592` (2026-10-29).
The run-heavy receipt is rc=0, 12 seconds, peak RSS 0.70 GiB. This measures the
validation process, not decoder allocation. No early-allocation or HDR gain-map
qualification is claimed. The controls guard valid negotiation and final
validation while the early-admission contract is developed.

## Advertised float-layout mismatch found during admission work

The native JPEG 0.9 adapter advertised RGBAF32 but returned RGBF32 for that
preference. Required-output validation correctly refused it; accepting the
capability table or a preference-selection helper as proof would not repair the
actual output. Native main e49c76f0 now emits opaque alpha with unchanged RGB
bits and metadata. The native regression covers baseline/progressive input and
preference ordering. The shared harness checks seven required layouts across
both scan modes, retaining exact samples, dimensions, orientation and metadata.
This repairs one concrete negotiation mismatch, not early allocation admission.

Controlled final interpretation cannot be adopted without a descriptor:
BoundaryAction::permits rejects missing evidence, and the fused sink also checks
it against output. The optional comparison in CheckedDecoder is not a missing-
evidence bypass; do not add a redundant caller query for that case.

The final published native pin passes all three admission tests and strict
all-target harness Clippy (rc0, 13s, peak RSS 0.70GiB). The seven-layout
matrix contains 14 native/checked comparisons. The standalone native
controlled-decode target passes 17 tests including exact RGBA float sample/alpha
checks; native library/test Clippy also passes (rc0, 8s, peak RSS 0.59GiB).
The harness lock changes only the JPEG source revision. Full logs are retained
in `~/tmp/zenpixels-release-reconcile-20261029/jpeg-rgba-final.log` and
`jpeg-rgba-native4.log`. No decoder allocation measurement is claimed.
