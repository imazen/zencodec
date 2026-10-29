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

## Ranked precision selection

A mixed preference regression found another native mismatch: JPEG e49c76f0
returned RGBF32 for `[RGB8_SRGB, RGBF32_LINEAR]`, because its one-shot adapter
enabled float output when any alternative was F32. Native b2495f7d uses the
first-supported layout selection after reading component count to select
precision, preserving preference order. No public signature changed.

The new control compares RGB/RGBA and U8-first/F32-first lists against
singleton-preference execution for baseline/progressive JPEG, including exact
row bytes and metadata. All four admission tests and strict harness Clippy pass
on the published pin (rc0, 12s, peak RSS 0.69 GiB); the native controlled-decode
target passes 18 tests and strict Clippy (rc0, 20s, peak RSS 0.68 GiB).
The original failing control remains in the run log `jpeg-ranked-before.log`.
Early allocation admission remains open. These mismatches demonstrate why a
second prediction implementation cannot substitute for the actual configured
decoder's output-selection boundary.

## Streaming-result audit

The current JPEG controlled route returns `OutputInfo` after row delivery.
Its context retains updated `ImageInfo`, but it does not return the one-shot
`DecodeOutput` extension map or owned source-encoding details. `CollectSink`
returns pixel storage only, and `to_buffer` copies that storage. Reconstructing
a complete one-shot result from these APIs would therefore silently discard
owned extras and add a second full-raster allocation. These are separate from
the descriptor adoption barrier, which already precedes native strip execution.

A shared completion owner must move final metadata, source-encoding details and
extensions alongside the collected pixels, distinguish unsupported/incomplete
completion from an empty but complete result, and preserve terminal/error/drop
ownership. Until those native paths are qualified, retain the one-shot route
and its final descriptor validation. Do not invent an empty extras map as proof
of completeness. Collector capacity refusal is now fallible, but this does not
supply that missing completion contract or native workspace evidence.
