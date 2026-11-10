# HLG native carrier policy

Base is the frozen stable core199 candidate `9c0df9ad`, retaining all 167
previously resolved dependency identities. The policy change is `74d81f07`.
No numerical kernel, dependency pin or public signature changes.

A CICP-only HLG source now keeps a permitted native carrier without requesting
a targetless ICC companion. RGB, gray and gray-alpha follow the same rule.
Source ICC retention still follows policy. ICC-only output, or a policy that
forbids CICP, retains the synthesis obligation so unsupported lowering reports
an error rather than dropping the source's color description. PQ/SDR companion
assertions remain unchanged. The existing HLG assertion is updated under the
owner's explicit choice to preserve native CICP and refuse HLG synthesis.

`just hlg-carrier-check` passes26 color policy and2 gray/lowering controls.
`just metadata-check` passes the workspace/all-feature tests and doctests,
five zenmeta no-default-feature configurations, formatting and strict
workspace/all-target/all-feature Clippy. Exact totals and the resource receipt
are in `benchmarks/hlg_native_carrier_2026-11-08.json`. The new HLG policy test
failed before the change. Original failures, including an invalid cached-binary
attempt caused by restored archive mtimes, are retained. The final run forces
fresh source timestamps and records compilation of the changed crates.

This is a local candidate for the core owner's integration. Existing source
review heads remain untouched. Native PNG/AVIF/JXL container qualification and
pin integration remain separate; pure resolver tests do not establish those.
