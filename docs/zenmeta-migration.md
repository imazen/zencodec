# zenmeta source and test migration

This is the implementation map for the [design](zenmeta-design.md), based on
source inspection and integration on 2026-10-12. The status below distinguishes
verified migrations from remaining work. Read the [release audit](zenmeta-published-api-2026-10-12.md) before changing public paths.

## Integration status (2026-10-12)

Changes remain on per-repository `review/zenmeta-integration` bookmarks pending
final API review and PR splitting. These are not crate releases.

| Owner | Verified work | Remaining work |
|---|---|---|
| zencodec | EXIF metadata subset of PR 130, baseline contract, typed unsupported inventory; registry comparison against 0.1.26 passes | Final public-surface snapshots and dependency guards |
| zenmeta | XMP/audit/display/Apple/IFD extraction, plist bounds, strict/legacy Apple distinction, shared gain-map XMP source (`ddb6f905`) | JPEG consumers, generic scrub analysis, final review |
| ultrahdr | Apple/plist delegation preserving 0.6.0 nominal types (`8db388f9`); registry comparison passes | PR 35 encoder metadata-policy integration, remote CI |
| TIFF | Default-off shared-IFD inventory, dispatch/error tests, six-platform CI configuration; native/i686 suites pass (`e61b3460`) | Finish remote CI and final API review |
| RAW | Default-off shared-IFD inventory for both backends, saved crashes, native/i686 suites, independent ExifTool checks (`47aeffac`) | Finish remote CI and final API review |
| JPEG | Extended-inspection source and tests imported; shared gain-map XMP source extracted | Reconcile PRs 210/225/221, wrappers, inventory gating and all corpus/oracle tests |
| Other codecs and zenpipe | Source/PR map below | Consumer migration and codec-owned rewrite planners |

The semver audit found existing unreleased breaks independently of this
extraction: TIFF's six associated error types differ from 0.1.2; RAW's error
variants differ from 0.2.0. Their codec changelogs retain those queued breaks.
Do not infer complete compatibility from the additive inventory contract.

## Ownership

| Existing source | Destination | Keep with its current owner |
|---|---|---|
| zencodec EXIF, orientation, ICC/CICP, ISO gain-map helpers | zencodec | Published types, policy behavior, baseline parsing and serialization |
| zencodec proposed XMP/audit/display-metadata modules | zenmeta feature modules | Only shared interchange types demonstrated necessary by codec callers |
| ultrahdr Apple MakerNote and binary-plist parser | zenmeta Apple module | Published ultrahdr types and compatibility wrappers |
| zenjpeg generic XMP interpretation | zenmeta XMP module | JPEG packet framing and extended-packet validation/reassembly |
| proposed zenifd walker/tag data | zenmeta inventory module | Backend-specific Rules implementations in TIFF/RAW codecs |
| per-codec inventory walkers | Respective codecs, gated by `inventory` | Decoder-consumption rules, selected-copy rules, container structure |
| zenpipe metadata scrub planners | Respective codecs | Container offsets, checksums, reference updates and output emission |
| generic metadata analysis from scrub services | zenmeta | Pipeline dispatch/CLI presentation stays in zenpipe |

Do not move every EXIF implementation into zenmeta by name alone. The published
zencodec tree is a useful metadata API already. A richer physical IFD walker
handles malformed entries and overlapping ranges that a salvaged EXIF tree does
not retain. Initially move the existing walker intact. Extract a common bounded
reader only once both consumers and their acceptance tests are explicit.

## Pinned sources to reuse

Each row pins the inspected head. Resolve the PR again before editing: another
change may have landed after this audit. These are source provenance, not claims
that all PR test suites were run during this audit.

| Work | Source | Inspected head |
|---|---|---|
| EXIF color category | [zencodec 123](https://github.com/imazen/zencodec/pull/123) | `19091618c1f17d3734485a2add53c2888438c61d` |
| EXIF device/time fields | [zencodec 124](https://github.com/imazen/zencodec/pull/124) | `21a4c3d77e69c683110aa4c980b87ed4c8016075` |
| EXIF entries | [zencodec 125](https://github.com/imazen/zencodec/pull/125) | `cfed5cbb21ea63a5d610d1a58a6fe1f8ae8f1975` |
| Display metadata and audit | [zencodec 128](https://github.com/imazen/zencodec/pull/128) | `2c72a170aada1792d1d10980d38c0fc639dcb970` |
| Media integration | [zencodec 130](https://github.com/imazen/zencodec/pull/130) | `27cfae5d4de24bfb20d2005d77daf4ac9ff05f39` |
| JPEG display/privacy | [zenjpeg 210](https://github.com/imazen/zenjpeg/pull/210) | `74e727f649a23063e3de1d964af2ced99b8bcd26` |
| JPEG extended metadata | [zenjpeg 225](https://github.com/imazen/zenjpeg/pull/225) | `897af48647265aa1bf34be9639d4690f20e55351` |
| Apple metadata | [ultrahdr 35](https://github.com/imazen/ultrahdr/pull/35) | `1dee8471bb32d3189add54e63d8f45bfeebfb440` |
| HEIC gain-map/privacy | [heic 51](https://github.com/imazen/heic/pull/51) | `e8008485391a990fecf686dc2a7d349156078456` |
| Metadata scrub services | [zenpipe 87](https://github.com/imazen/zenpipe/pull/87) | `aa21cba063da50962d883c6541a225b95e9b4a43` |

The zencodec PRs have dependencies on one another; do not cherry-pick an entire
stack and accidentally land unrelated timing/media API changes. Separate the
metadata source moves from any media contract decision.

### JPEG

Reuse `zenjpeg/src/container/metadata/{identify,mod,reassemble,tests,trailer,walk}.rs`
from PR 225, plus `tests/extended_metadata.rs`,
`tests/extended_metadata_exiftool.rs`, `tests/support/metadata_corpus.rs`, and
`fuzz/metadata_checks.rs` with its targets and regression cases. Keep the
existing independent-reader tests; do not replace them with round trips alone.

PR 210 adds `container/metadata.rs`, while PR 225 introduces a directory module
at that path. Reconcile the actual implementations, rather than allowing one
filesystem layout to overwrite the other. Preserve filtering once per logical
metadata component after assembly. Move generic namespace-aware XMP work to
zenmeta without moving JPEG APP framing there.

Carry forward the documented acceptance differences in `zenjpeg/docs/METADATA.md`:
SOS length/component handling, FF stuffing and restarts, TEM/SOI, DRI/DAC, MPF
offsets and trailers, ICC chunk numbering, and strict extended-XMP GUID/length/
MD5 validation versus decoder concatenation. Differential tests need both paths.

JPEG inventory has its own entropy scanner, malformed-input regressions,
bundled corpus tests and independent oracle. Reuse those unchanged when adding
the feature gate; moving a parser is not an opportunity to relax assertions.

### Apple metadata

Use the existing MakerNote and binary-plist parsing plus PR 35's richer view,
unknown-entry preservation, duplicate-tag checks, truncated-data tests and
critical-rational validation. Preserve the released compatibility functions'
behavior separately from new stricter operations. A stricter public operation
may reject what a legacy helper accepts; its name and result must make that
intent explicit.

The copyright holder authorized Apache-2.0 OR MIT on 2026-10-12 for controlled
source moved into zenmeta. Preserve third-party notices and dependency licenses.
The shared source record lives in [zenmeta provenance](../zenmeta/PROVENANCE.md).

### TIFF and RAW

Reuse PR 30's `zenifd/src/walk.rs` and `tags.rs`, including their inline tests.
They become zenmeta modules, not a second new published crate. Keep the existing
Rules specialization so TIFF and RAW do not falsely report each other's decoder
behavior. Move the dependency from zentiff's general `zencodec` feature to its
new `inventory` feature.

Retain `zentiff/src/inventory/mod.rs` and `zentiff/tests/inventory.rs`. Retain
zenraw's inventory module, `pregate_tests`, integration tests and fixture
builders. Tests for malformed directories, overlaps, ignored copies and data
the decoder returns to the caller remain consumer tests as well as shared-reader
tests where applicable.

### Rewrite plans outside zenpipe

PR 87 already has validated plans using borrowed output pieces, metadata entries
and runtime services. Reuse `zencodecs/src/metadata/{mod,exif,jpeg,png,jxl}.rs`,
`zencodecs/tests/metadata_scrub.rs` and `tools/metadata-services/src/lib.rs` by
responsibility. Extract codec planners first and make existing pipeline calls
delegate to them; then move genuinely generic metadata processing into zenmeta.

The output recipe must preserve compressed image bytes and update container
references. An inventory partition cannot substitute for this plan. Keep plan
validation before writing, resource/cancellation errors explicit and sink
partial-write behavior documented.

## Inventory migration across codecs

| Codec | Existing PR and head | Reuse/gating boundary |
|---|---|---|
| JPEG | [221](https://github.com/imazen/zenjpeg/pull/221), `2c9639748490c4ce60d4507932708c96c62743ac` | Inventory, entropy scanner, bundled corpus/review/oracle tests, fuzz target |
| HEIC | [57](https://github.com/imazen/heic/pull/57), `628ebc3056dcc8392d7f8761cb726e899e472de9` | Inventory module and inventory/review tests; retain item associations |
| TIFF | [zenextras 30](https://github.com/imazen/zenextras/pull/30), `601682d365d7b8d772bb045f45bb554005f41fb1` | Shared IFD traversal moves; TIFF Rules and tests stay |
| RAW | [20](https://github.com/imazen/zenraw/pull/20), `3f8bc711e9bb463a1f42c26331bf7d6bba4096dd` | RAW-specific rules and pregate/regression fixtures stay |
| PNG | [32](https://github.com/imazen/zenpng/pull/32), `bfa9b0567bc45c0066a8bd39fa0607387cfa0291` | Inventory and all review/cost tests; scanning dependency only when needed |
| WebP | [106](https://github.com/imazen/zenwebp/pull/106), `cd09b5bde59cbf9772624f62ed0fe4b7abac8b04` | Keep RIFF/WebP consumption rules in codec |
| GIF | [23](https://github.com/imazen/zengif/pull/23), `36c7ff1ddae01cdb8fcc803d5b77db0478e1834b` | Keep subblock/frame handling in codec |
| AVIF | [61](https://github.com/imazen/zenavif/pull/61), `d2f17783e39b9e13521cc7491f7d3c5657f29fc4` | Keep AVIF backend interpretation in codec |
| JPEG XL | [24](https://github.com/imazen/zenjxl/pull/24), `3fa2837f046150e585013e951e8ae7c6333ec7b7` | Keep container/codestream handling in codec |
| Bitmaps | [23](https://github.com/imazen/zenbitmaps/pull/23), `4c383dbf47c622f200b1df94f26ec4423386f564` | Preserve per-format decoder distinctions |
| JPEG 2000 | [zenextras 26](https://github.com/imazen/zenextras/pull/26), `30c1bc720748839112f9426dadf00cf45f6fa31c` | Keep box/codestream interpretation in codec |
| PDF/SVG | [zenextras 33](https://github.com/imazen/zenextras/pull/33), `56214382b7f35474798103994bb17fbf9e387803` | Preserve stated parser/renderer coverage limits |
| EXR | [zenextras 37](https://github.com/imazen/zenextras/pull/37), `6a047c5f822dad0f2e5d0a63ab83b9afe029445c` | Keep EXR structural interpretation in codec |

Detailed source/test inspection covered JPEG, HEIC, TIFF, RAW and PNG. The
remaining rows identify the live PRs and integration boundary; their individual
test suites and internal feature edges still require inspection during migration.

The supporting [zenflate scan PR 15](https://github.com/imazen/zenflate/pull/15),
head `0fa809a999889913a5fa9c1103337c6728d9b067`, has
`src/decompress/scan.rs`, `tests/scan.rs` and a scan fuzz target. Keep DEFLATE
mechanics in zenflate. A metadata crate does not need to own compression code.

## Landing sequence and acceptance gates

1. Record the published baseline and preserve existing unrelated fixes. The
   diffuse-white retention fix is commit `2b8eea3f` and has passed local tests,
   clippy, formatting and a registry semver comparison.
2. Review the zencodec-owned interchange types and unsupported-inventory
   signature. Compile a published-style external DecodeJob implementation whose
   native error has no `From<UnsupportedOperation>` implementation. Exercise the
   inherited default through generic and dynamic dispatch.
3. Establish zenmeta's license and module/feature manifest, then move existing
   XMP/Apple/audit/IFD source and tests in mechanical commits. No behavior changes
   in the move commits. Do not introduce a dependency back from zencodec.
4. Migrate JPEG and TIFF/RAW consumers to prove both general metadata and shared
   inventory use. Preserve published wrappers and test the actual decoder's
   selected metadata against inspection. Then migrate remaining codecs.
5. Extract the existing codec scrub planners from zencodecs and make pipeline
   methods delegate. Prove direct codec use in a small consumer with neither
   zenpipe nor zencodecs in its dependency graph.
6. Run the default/minimal and feature-enabled matrices, API comparison for every
   changed package, existing differential/oracle/fuzz regressions, and metadata
   preservation tests. Review new public APIs before release.

The inventory contract already distinguishes malformed bytes from operational
failure. Preserve it: malformed/truncated files produce a validated inventory
with malformed/unknown parts; unavailable support, cancellation and resource
limits are errors. Exhausting a work budget must not relabel unvisited bytes as
consumed. Partial interpretations must name their coverage limits.

Feature checks must prove absence as well as success. With inventory disabled,
no walker module or inventory-only dependency is compiled and the capability is
false; calling the method returns unsupported. Enabled builds advertise the
capability and return the same configured interpretation through both dispatch
paths. Metadata-only clients must work without image decoding/pipeline crates.

Maintain one parser owner, consumer-specific interpretation tests and shared
testkit checks. Do not attempt to keep correctness synchronized through copied
tag tables, copied parsers or prose alone. Where two interpretations intentionally
remain, pin their differences with existing regression fixtures and an explicit
policy at the shared parser boundary.
