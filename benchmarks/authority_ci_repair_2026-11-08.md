# Authority review CI repair

Base: PR201 `1f3c8c3f`. Runtime authority guard unchanged.

The documentation job rejected the stale compiler catalog. Regenerating all
8 package catalogs with nightly-2026-10-06 updated only the core detail output
and two package fingerprints. The site build and all 27 Node source checks pass.
Generator receipt: rc=0, 35s, peak-RSS 0.34 GiB. This is not a performance study.

The native JPEG graph rejected old beta SIMD caps. It now uses the qualified
JPEG PR260 source `6d6539df`, PNG `d32e82e5`, HDR `5a3b158c`, garb `8f9aa10d`,
linear-srgb `2969ee84`, zenflate `a2c7d540`, pixels `fde59b96`, and stable SIMD
0.9.31. All core-family packages come from this checkout. The existing moxcms
0.9.1 lock remains unchanged; this does not qualify the separate moxcms 0.9.2
or PNG db6/pixels3ceb color cohort. No unrelated package versions changed.
Three unused root patches retain the JPEG cohort's optional dependency choices.

Compilation then exposed seven removed/private pixel calls in frozen zenpipe
`f0322466`. Its minimal descendant `154e16b0` uses public facades. Raw-row
adaptation retains permissive conversion and unchecked alpha removal, matching
the former wrapper exactly; no processing kernels or policy strength changed.
The guide's opacity check and JPEG fixture synthesis use the same public facades.

The original RGB16 refusal fixture now succeeds because this JPEG revision
supports RGB16. CMYK16 now exercises the original exact OutputDescriptor
refusal assertion. RGB16 was added to the unchanged row/sample/metadata parity
loop. The original failing run remains retained; no refusal assertion was removed.

Final remote-pinned graph: 10 interface consumers, zero mismatches. Default
native suite: 68 passed, zero ignored. Native-preflight suite: 73 passed, zero
ignored. Strict all-target Clippy with native-preflight passes. Combined final
run: rc=0, 20s, peak-RSS 0.58 GiB. Original resolution, compilation, and fixture
failures remain retained separately. These are correctness controls, not timing
comparisons. Reproduce via `just jpeg-policy-setup`, `just jpeg-policy-check`,
`just jpeg-preflight-check`, and `just docs-site-check` in an isolated export.
