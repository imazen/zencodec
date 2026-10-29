# Spatial contracts on the corrected JXL encoder

The spatial harness moves jxl-encoder and its two companion crates from
55c59fe7 to 554ad696. The latter includes the qualified F32 modular-gradient
overflow repair; native encoder evidence is recorded in that repository's
`benchmarks/gradient_float_2026-10-29.md`. The decoder and collection adapter
pins remain unchanged. No shared API or existing assertion changes.

An isolated export of zencodec source 1568152e, with its resolved spatial lock,
ran the existing `just spatial-validation-with-oracles` and
`just spatial-validation-lint`. The locked graph changes only the three JXL
source identities and the root's direct libm edge. Tests retain debug assertions
and overflow checks. This is contract qualification, not a performance study.

The complete 142-test native spatial suite passes, including TIFF orientation,
regions, float payloads, JXL selected layers/frames, auxiliary channels, float
alpha, independent sample/carrier checks, retained owners, cancellation and
policy refusal. Strict all-target Clippy passes. It does not qualify every
encoder mode, browser execution, old-JPEG feature paths or operation-wide memory.

The independent oracle remains unmodified libjxl 0.11.1 with source checksum
1492dfef8dd6c3036446ac3b340005d92ab92f7d48ee3271b5dac1d36945d3d9.
The current C helpers were rebuilt against that same library using
`just spatial-jxl-oracle-relink`; helper and library hashes accompany the outputs.

Three attempts are retained:

1. rc101, 97s, peak RSS 0.75 GiB: artifact parent was root-owned, so output
   creation failed. No assertions changed.
2. rc101, 34s, peak RSS 0.49 GiB: writable artifacts yielded 141 passes and one
   failure. The old oracle helper did not implement `samples-oriented` and
   returned status 2 before comparison.
3. rc0, 44s, peak RSS 0.57 GiB, minimum available 13703 MiB, peak load 0.98:
   same Rust source/lock/assertions, current helper against the same library,
   fresh artifact directory; full suite and lint pass.

Logs on the qualification host: `~/tmp/zencodec-jxl-main-20261029/`
(`full.log`, `rerun.log`, `final.log`). Native artifacts and oracle provenance:
`/mnt/v/output/zencodec/jxl-main-20261029-rerun/`. These paths were verified on
r7900x on 2026-10-29. No executables or generated media are committed.
