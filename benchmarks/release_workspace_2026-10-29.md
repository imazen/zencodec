# Default workspace release gates

Base main dec7f37547209a1c3cf66fe20ea6d1825ad5e470, test dependency repair
7eb233ee. Isolated export, prior lock retained before metadata resolution.
`just release-workspace-check` runs all workspace targets, workspace doctests
and strict all-target Clippy with default features.

Initial unmodified all-target gate failed at compile time: media-testkit's
policy_completion_review imports std-only output/staged APIs without enabling
std. Its dev dependency now enables that feature; production dependency features
and all assertions remain unchanged. No target was excluded or ignored.

Repaired run: 1300 passed, 0 failed, 38 existing ignored across 77 result batches;
strict Clippy passed. run-heavy --mem 3G --jobs 2 -- timeout 900: rc0/56s,
peak RSS 0.45GiB, minimum available 13493MiB. All-target execution also runs the
existing benchmark harnesses. Their timing output is validation only, not a
controlled comparative performance result.

Merged-base CI run 40866031711 passed Linux x64/ARM64, Windows x64/ARM64,
macOS ARM64, i686, WASM, MSRV, Clippy, format, semver and metadata feature gates.
Documentation run 40866031700 passed guide/browser/example and native TIFF/JXL,
AV1 and JPEG gates; fuzz run 40866031510 passed. These runs predate the test-only
feature repair and do not qualify every codec or application mode.
