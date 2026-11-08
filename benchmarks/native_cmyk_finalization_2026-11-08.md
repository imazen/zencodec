# Native CMYK finalization: independent CMS qualification

Parent: zencodec `8d0a7234510e62b3c1f1178933a5b9d8788cccca` (PR201).
The owner approved replacing blanket CMYK refusal with correctness controls.
No runtime source, dependency pin, default, or existing numerical bound changes.

## Result: numerical gate fails

The native `conformance_test_images/cmyk_layers.jxl` selection supplies inverted
F32 CMYK (one means no ink), its original embedded ICC, and strided storage.
The existing frozen MoxCMS 0.9.1 backend accepts finalization to named sRGB F32.

| Measurement | Result |
|---|---:|
| RGB samples compared | 786432 |
| Samples violating strict absolute error < 0.0013 | 1050 |
| Maximum absolute error | 0.006681680679321289 |
| Nonfinite actual/reference/control samples | 0 |
| Samples distinguishing deliberately wrong ink polarity | 786432 |
| Unclipped LCMS samples outside [0,1] | 12978 |

The 0.0013 bound is retained from frozen zenjxl-decoder fe819360's
`cmyk_to_srgb_matches_lcms2_relative_bpc` test; its historical sample set leaves
some channels unchecked. This new control checks every RGB channel in the native
selection. Existing RGB matrix/profile oracle bounds remain 0.0001, unchanged.

Worst sample: pixel (114,136), blue. Native inverted CMYK input is
`[0.960784375667572, 1.0, 0.9843137860298157, 1.0]`.
MoxCMS/finalizer returns `0.982710063457489`; independent LCMS returns
`0.9760283827781677`. Both values are in range, so clipping cannot explain it.

Direct MoxCMS calls and finalization match **bit-for-bit on every output row**.
Direct calls also preserve destination suffix guards. This isolates the observed
numerical discrepancy from finalizer routing, row stride and publication code;
it does not identify a particular interpolation or profile-processing kernel.

## Independent oracle and preserved contracts

The C oracle consumes the exact native ICC and active rows. It explicitly maps
JXL polarity to Little CMS percentages (`100 - 100 * sample`), uses intent 1
(relative colorimetric), and `BLACKPOINTCOMPENSATION | HIGHRESPRECALC`.
The target is LCMS's sRGB profile. Reference output remains unclipped; comparison
clips it to match the documented permissive `EncodeReady::finalize` route.
A second invocation deliberately uses the wrong ink polarity as a negative control.

System LCMS 2.17 and independently compiled C LCMS 2.20 at
`7717fce0d85447ef3b2df0cca2e99207c15eeecd` produce byte-identical reference files:
`d08e131421d960abca15d54b105b5b6c137b7310181108b54ae9972ca927aa3f`.
The embedded CMYK profile SHA256 is
`4855b8fabb96bdc6495d45d089bb8c8efb1ae18389e0dc9e75a5f701a9c0b662`.
The builder records all C source/header hashes, binary identity and linked libraries.

Both focused runs report 4 passing controls and the one numerical failure.
Passing controls retain no-CMS `NeedsCms`, identity samples/context, current-profile
requirements and output-storage accounting. The numerical control also checks the
entire source's bytes/context are unchanged, output geometry/context/metadata are
sRGB, retained native charges are unchanged, and dropping the native owner releases
all its reservations **before** asserting the numerical bound. These are pool
accounting checks, not whole-process allocation bounds.

No assertion is skipped; no failure becomes a successful unsupported result.
The integration is not numerically qualified for release on this frozen CMS graph.
Broader CMS/backend research and a qualified dependency correction remain separate.

## Reproduction

Build the existing oracle with `just spatial-color-oracle OUTPUT`, or use:

```sh
python3 perf/spatial-validation/build_color_oracle.py OUTPUT --lcms-source PINNED_C_SOURCE
just spatial-cmyk-check FRESH_ARTIFACTS OUTPUT/color-oracle
```

Use the existing run-heavy cap and a timeout around these commands. The focused
recipe deliberately stops on the numerical failure. Strict all-target Clippy was
run separately with `just spatial-validation-lint cms_oracle,old_jpeg` and passes.

Receipts: LCMS 2.17 focused run rc101, 18s, peak RSS 0.64 GiB; LCMS 2.20
focused run rc101, 18s, peak RSS 0.64 GiB. Strict Clippy rc0, 1s, peak RSS
0.29 GiB. C 2.20 oracle compilation rc0, 5s, peak RSS 0.06 GiB. The initial
new witness's mistaken 64x64 size guard failed before comparison; its receipt is
retained. The corrected witness has a 64 MiB file budget and an 8 MiB ICC cap.
No diagnostic metric maps are generated; only required oracle inputs/outputs,
scalar comparisons and the worst input/output vector are retained.

Full spatial matrix with the new C 2.20 helper: **148 pass, 1 numerical failure,
0 ignored**, rc101, 39s, peak RSS 0.48 GiB. Existing RGB color, native orientation,
collection, TIFF and storage controls remain enabled; no other failure occurred.
