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

## Streaming-result ownership is now qualified for JPEG

The JPEG controlled producer now retains its actual metadata scan owner,
including configured native extras and MPF secondary images, and supplies it
through terminal `take_details`. The collector can move its finished PixelBuffer
with `into_buffer`. Native and consumer controls preserve descriptive metadata,
source details, extensions and every row. See [owned completion](decode-result-ownership.md).
This closes the missing-owner and extra-raster-copy barriers for the qualified
JPEG route. It does not establish metadata privacy completeness or qualify
float, CMYK, transformed or gain-map reconstruction paths.

## Measured early refusal and collection costs

`just decode-admission-profile OUTPUT REVISION` builds the committed native
probe and profiles ten matched paths with heaptrack. Inputs are separately
encoded 1024×768 RGB8 JPEGs; fixture generation is outside the profiles. The
collector storage is 2,359,296 pixel bytes. Refusal asks for unsupported RGB16;
its controlled sink panics on any begin/row/finish call, and the executable
requires the same typed OutputDescriptor failure in both paths. Successful
one-shot, copying and moving paths must have identical pixel checksums within
each configuration. All ten cases and strict binary Clippy pass.

| JPEG | Path | Peak live heap bytes | Allocation calls |
|---|---|---:|---:|
| baseline | one-shot-refuse | 2,820,075 | 76 |
| baseline | controlled-refuse | 494,778 | 59 |
| baseline | one-shot-ok | 2,820,071 | 73 |
| baseline | controlled-copy | 5,002,448 | 99 |
| baseline | controlled-own | 3,082,217 | 98 |
| progressive | one-shot-refuse | 5,243,513 | 122 |
| progressive | controlled-refuse | 474,414 | 59 |
| progressive | one-shot-ok | 5,243,509 | 119 |
| progressive | controlled-copy | 5,401,770 | 161 |
| progressive | controlled-own | 5,401,769 | 160 |

These are full-process heap peaks from allocation-event replay, cross-checked
against heaptrack_print, not timestamp-sampled estimates or codec RAM bounds.
No throughput comparison was run. The build/profile/Clippy wrapper reported
rc0, 8s, peak RSS 0.59 GiB, minimum available 10,875 MiB. Exact source tree,
binary and input identities are in
[the measurement record](../benchmarks/decode_admission_2026-10-29.json).
Raw captures and logs are retained under
`~/tmp/zenpixels-release-reconcile-20261029/decode-admission-profiles`.

The early boundary avoids native raster/coefficient work on this refusal.
Ownership transfer eliminates one allocation, but does not reduce the dominant
progressive native peak in this fixture. Controlled successful decoding still
retains the encoded input and has a higher observed peak than one-shot decoding.
Therefore switching every successful decode to this route is not justified.
Keep the ordinary success path, expose the controlled route for pre-effect
requirements, and qualify remaining native modes before promising equivalent
fallback. Do not turn unknown workspace evidence into a numeric bound.

## Required descriptors with complete, late ICC profiles

The native `7c5a9501` matrix now covers baseline/progressive JPEG, a complete
Display P3 profile in the header, before EOI or split across scans, and five
layouts: RGBA8, BGRA8, RGBX8, BGRX8 and RGBAF32. It first obtains the native
profile-tagged result, then requires that exact descriptor through `DecodeRequest`.
All 30 combinations preserve exact row bytes and metadata. Each case also requires
the complete original ICC bytes and verifies that its actual descriptor differs
from the initial sRGB/linear layout preference; a dropped profile cannot make
this control pass accidentally.

The suspected constant-comparison mismatch was not reproduced: native
`select_decode_descriptor` first normalizes channel layout and sample type,
then final output derives color interpretation from the retained source metadata.
Do not add a second header-only predictor to reproduce that logic. A future
one-shot admission hook must retain the configured decoder and account for those
late color facts before claiming an exact output descriptor. The current required
output API remains correct but may refuse after raster allocation.

`just jpeg-decode-admission-check` passes all eight admission tests and strict
native-harness Clippy on test source `90186109`: wrapper `rc=0 9s`, peak RSS
0.59 GiB, minimum available 13621 MiB. Full logs:
`~/tmp/zencodec-profile-layout-20261029/qualified.log`. The initial, weaker matrix
also passed and is retained in `before.log`; the stronger assertions were added
before landing. No production changes, new public API, native allocation bound,
CMYK qualification or HDR reconstruction qualification are claimed here.

## Retained one-shot preparation candidate

The [review experiment](../perf/retained-decode-validation/README.md) adds an
optional preparation method on the actual configured decoder. Required-output
operations can refuse a proven mismatch before raster reconstruction, while
unknown facts preserve the existing decode-and-validate path. Actual output is
always checked. Native JPEG layout/late-ICC witnesses and matched heap profiles
are recorded there, including the successful-path allocation cost. This is not
a whole-operation memory bound or blanket native adoption.
