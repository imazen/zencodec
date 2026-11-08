# PR201 frozen native graph repair

Parent: zencodec `92918f60d9b00651d1fc66292887dd910a695f1f`.

The independent AV1 and spatial manifests still selected SIMD 0.9.30,
while their declared pixel/linear dependencies require beta.2 or stable 0.9.31.
Resolution exposed older private pixel helpers and the removed media plane API.

## Exact compatibility sources

All codec numerical kernels stay at the original pinned sources. These separate
review bookmarks do not change codec mains or the independent PNG carrier cohort.

| Package | Frozen base | Review source | Change |
|---|---|---|---|
| rav1d-safe | b19aec65 | 68e499ec455dc6bc29ee4b69abd64022a6099411 | SIMD ceilings only |
| zenavif / zenmp4 | 94116d26 | b0901f3c87ce414012d6a8c933765c183c94e588 | SIMD ceilings/decoder edge; plane and public pixel facades |
| jxl-encoder | 554ad696 | 754699f734bea9d9642834e2d21ccf792ed8ea74 | SIMD ceilings only |
| zenjxl-decoder | fe819360 | 1514ee496526d6fadd518ff109a658acf135aa07 | SIMD ceilings only |
| zenjxl | a76ac76d | 8b86f9a2bcfee893ece89f7976b9132bfd88d315 | Public fallible orientation facade; original error causes/traces |
| ultrahdr-core | 7e9d296c | 5bdb194b64c39052f33d84f6d772158b5e660802 | Existing aadf1021 clone compatibility implementation |
| garb | 605276d7 | 8f9aa10d09f84bb0a1e3fdbd1773f610eec1b784 | SIMD ceilings only |

Both manifests retain pixels fde59b96 and linear 52896f4b. Their stale locks
previously selected pixels 5bf81d64 and registry linear-srgb. The repaired locks
match the declared sources. MoxCMS remains 0.9.1. The only package version changes
are archmage, archmage-macros and magetypes 0.9.30 -> 0.9.31.
Locked metadata reports 13 AV1 and 6 spatial interface consumers, zero mismatches.

## Qualification and limits

Run from an isolated export outside workspace overrides, with run-heavy 3G/jobs2,
explicit timeouts, full logs and separate AV1/spatial target directories.

- AV1: 70 tests pass, no ignores. Strict all-target Clippy passes. Independent
  float and deliberate-clipping controls pass for 54 presentations. Gamma controls
  cover 10,714 extended samples; maximum error 1.0032435548978924e-7 and roundtrip
  error 4.76837158203125e-7, under the unchanged existing bound.
- AV1 runtime receipt: rc0, 45s, peak RSS 0.59 GiB. Clippy: rc0, 1s, 0.25 GiB.
  Independent float: rc0, 1s, 0.05 GiB; gamma: rc0, 0s, 0.02 GiB.
- Spatial: **146 pass, 1 fails**, zero ignores. Receipt rc101, 124s, peak RSS
  0.81 GiB. Strict all-target Clippy passes (rc0, 1s, 0.29 GiB).
- Added native JXL orientation control passes: FlipH and Rotate90 compare every
  byte against independently indexed original samples and retain color context.
  Receipt rc0, 20s, peak RSS 0.64 GiB. This is separate from the 147-test run.

The spatial failure is
`native_cmyk_finalization_does_not_convert_float_cmyk_with_moxcms`: the pinned
pixel converter now succeeds where this existing test requires `NeedsCms`.
The assertion is retained, not skipped or weakened. Replacing it requires owner
approval and independent color qualification; successful conversion alone is
not evidence of correct CMYK output. The spatial job is not green.

The original resolution/compilation failures are retained. The first AV1 runtime
attempt passed 65 and failed 5 because mapped native planes omitted encoding
facts. Restoring per-plane encoding evidence fixes all five without changing
assertions. Strides are expressed in samples in PlaneRef, while SampleEncoding
retains code depth and bit placement; encoder admission checks both storage type
and any recorded plane encoding.

Reproduce with `just av1-policy-setup`, `just av1-policy-check ARTIFACTS` (set
AV1_TIMING_FFPROBE to the pinned timing inspector), and the existing spatial
oracle setup plus `just spatial-ci-check`. No codec feature exclusion, numerical
threshold change, or PNG core pin change is included.

Published-source requalification: the final git-only AV1 graph passes the full
`av1-policy-check` recipe (70 tests, strict Clippy, gamma verifier and its control),
rc0, 48s, peak RSS 0.59 GiB. The final git-only spatial graph passes strict
all-target Clippy, rc0, 5s, peak RSS 0.56 GiB. Guide source checks pass 27/27.
No claim of a green full spatial run is made.
