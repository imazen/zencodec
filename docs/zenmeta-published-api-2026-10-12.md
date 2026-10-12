# Published metadata API audit — 2026-10-12

The proposed [zenmeta design](zenmeta-design.md) does not require a breaking
zencodec release. Preserve the published EXIF/policy/color/gain-map surface and
give any new trait method a compatible default. Inventory and the richer
metadata PRs are not in the published versions inspected here.

This is an audit of released source and selected development changes, not a
claim that the proposed migration has passed compatibility checks.

## Method and release baseline

Versions were resolved through the crates.io API, then downloaded with
`cargo read <crate> --path-only`. Public modules, definitions, re-exports and
Cargo features were inspected in those release artifacts. A GitHub main branch
or open PR is not evidence of a published API.

| Package | Published version inspected | Release source |
|---|---|---|
| zencodec | 0.1.26 | [source](https://docs.rs/crate/zencodec/0.1.26/source/) |
| zenjpeg | 0.8.4 | [source](https://docs.rs/crate/zenjpeg/0.8.4/source/) |
| ultrahdr-core | 0.6.0 | [source](https://docs.rs/crate/ultrahdr-core/0.6.0/source/) |
| heic | 0.1.6 | [source](https://docs.rs/crate/heic/0.1.6/source/) |
| zentiff | 0.1.2 | [source](https://docs.rs/crate/zentiff/0.1.2/source/) |
| zenraw | 0.2.0 | [source](https://docs.rs/crate/zenraw/0.2.0/source/) |
| zenpng | 0.1.4 | [source](https://docs.rs/crate/zenpng/0.1.4/source/) |
| zenwebp | 0.4.4 | [source](https://docs.rs/crate/zenwebp/0.4.4/source/) |
| zengif | 0.7.3 | [source](https://docs.rs/crate/zengif/0.7.3/source/) |
| zenavif | 0.1.6 | [source](https://docs.rs/crate/zenavif/0.1.6/source/) |
| zenjxl | 0.2.1 | [source](https://docs.rs/crate/zenjxl/0.2.1/source/) |
| zenbitmaps | 0.1.5 | [source](https://docs.rs/crate/zenbitmaps/0.1.5/source/) |
| zenpdf | 0.2.0 | [source](https://docs.rs/crate/zenpdf/0.2.0/source/) |
| zenflate | 0.4.0 | [source](https://docs.rs/crate/zenflate/0.4.0/source/) |

The registry returned 404 for zenmeta, zenifd, zenjpegai, zenexr, zenjp2 and
zensvg at this observation. Do not infer that a similarly named repository has
no code: several are workspace members with open implementation PRs.

Pinned checksums for the principal extraction sources, as reported by crates.io:

```text
zencodec      0.1.26 be4b0d4108a8f062c85090bf8d0e6dd00fea976ffacc0e4f3cb6b03284a75531
zenjpeg       0.8.4  4fc00260b76e859d71f31ceab888e6691508569e69ed6eeed3a42ef9791cfd7f
ultrahdr-core 0.6.0  af411a89dd994324c442cea3fa0ded10aac0550cadb9a5cb2ba03c9596a4ed05
heic          0.1.6  4922930855bf6756c7a9caf637c6bca37f851927d4c89a0117beb254614f79a3
zentiff       0.1.2  9842fe70d95e8fc2240e183b6ebc341db715a2438725c0c08490ca0804d77cd4
```

## zencodec: keep what has shipped

The 0.1.26 archive identifies source commit
`998edf5f5152f675bbe3b4e959470dffe1536c36` in `.cargo_vcs_info.json`.
Its [public API](https://docs.rs/zencodec/0.1.26/zencodec/) already includes:

- `pub mod exif`, with root re-exports `Exif`, `ExifPolicy`, `Retention` and
  `TextEncoding`;
- `Metadata::filtered`, `Exif::filtered`, the metadata policy types/fields and
  encoder job `with_metadata_policy`, plus its dynamic setter;
- orientation parsing/writing helpers, ICC/CICP handling and color planning;
- ISO 21496-1 gain-map types, parsers, validation and serializers;
- `CodecError`, `At`, `CategorizedError` and the error inspection machinery.

Removing this implementation wholesale to zenmeta would either break callers or
require the dependency direction we want to avoid. Keep the useful baseline in
zencodec and reuse it. Do not turn `Metadata::filtered` into a differently shaped
fallible API or promise that it sanitizes arbitrary container bytes.

Absent from 0.1.26: the inventory module and trait methods, inventory capability,
`Exif::entries`/`EntryRef`, and XMP/display-metadata/audit modules. The development
inventory work has merged, but merged is not published. In particular, changing
the new `Result<Option<Inventory>, _>` signature before release is possible.

The existing `DecodeJob::Error` bound does not provide an error constructor.
Requiring `From<UnsupportedOperation>` or a new required method would break
existing implementors. Use a constructible error for the new method's default;
the shared `At<CodecError>` is available in the published baseline. The existing
`UnsupportedOperation` is non-exhaustive, so adding an Inventory variant does
not require downstream exhaustive matches to change.

### Compiler-backed check of today's tree

At commit `2b8eea3f9263793e939341fd90fc710621d7650e`, with package version
0.1.27, this command passed against the registry baseline:

```sh
cargo semver-checks check-release -p zencodec \
  --baseline-version 0.1.26 --all-features --color never
```

Tool version: cargo-semver-checks 0.50.0. Report: 196 checks passed, 58 skipped;
summary: `no semver update required`. This checks the current tree, not future
extractions, arbitrary behavioral compatibility, or all feature combinations.

The same commit passed workspace tests, zencodec all-target clippy with warnings
denied, and scoped formatting checks. Workspace tests reported existing
deprecated API uses in testkit's usage tests. Existing ignored documentation
examples were not changed.

## zenjpeg: new container APIs, old extras APIs

The 0.8.4 artifact has no `container` module. The new container XMP and extended
metadata work can be relocated before publication. However, these published
surfaces must remain:

- `decode::extras` and `encode::extras`;
- `PreserveConfig`, `IccPreserve`, `PreservedSegment`, `PreservedMpfImage`,
  `MpfDirectory`, `MpfEntry`, and `StandardProfile`;
- `DecodedExtras` accessors for EXIF, XMP, ICC and gain maps;
- `EncoderSegments` EXIF/XMP/ICC mutation, XMP modification and gain-map methods.

Keep existing method signatures and behavior while delegating implementation.
The [extended-metadata PR](https://github.com/imazen/zenjpeg/pull/225) explicitly
records decoder/inspector differences. Strict extended-XMP validation cannot
silently replace the legacy decoder's concatenation behavior.

## ultrahdr-core: Apple and binary plist APIs have shipped

The 0.6.0 package exposes root Apple HDR helpers and the public
`metadata::apple` module, including its tag constants, `AppleHdrInfo` fields and
methods. It also exposes `metadata::bplist::{PlistValue, parse_bplist}`.

Keep those public definitions and paths when moving the parser. Wrapper methods
can delegate and convert. A recursive `PlistValue` conversion must cover every
variant and preserve its existing parser's success/failure behavior. Do not
change public type identity through a re-export from zenmeta, or maintain a
second parser to avoid the conversion. Preserve licensing and notices.

ISO gain-map helpers already delegate to zencodec. They do not justify another
ISO parser in zenmeta.

## Other codec inventory APIs

No inventory API was found in the published codec sources listed above. Their
inventory PRs remain migration work for git consumers, not a crates.io break.
The proposed zenflate scanning helpers are also absent from zenflate 0.4.0.
Existing decode, metadata transport and native error APIs still need to remain
compatible when the walkers acquire new feature gates.

The complete source ownership and test movement plan is in
[zenmeta-migration.md](zenmeta-migration.md). Source inspection of these packages
is not a substitute for running semver-checks on each changed codec after its
implementation is migrated.

## Release conclusion

Keep zencodec on its compatible release line. Do not bump a leading non-zero
version component merely because a source file changes ownership. Re-run the
registry comparison on each final package and test old external implementors.
Coordinate unpublished PR dependencies explicitly; existing git consumers can
still break even though the released API remains intact.

## Coordinated animation dependency

PNG main already uses exact rational animation timing from zencodec `20310944`.
The metadata integration retains those existing additions so the consumer
can use one shared contract source. Its millisecond constructors/accessors
remain available. The first combined semver check caught changed numeric
discriminants from inserting `AnimationTiming` between released variants;
the combined source appends the new variant to preserve those values. This
is a compatibility repair to an unreleased proposal, not a video feature.

After that repair, `just metadata-semver` passes all 196 enabled checks
against 0.1.26 (58 skipped by the tool). Workspace tests, isolated zenmeta
features, all-target clippy, Rust 1.88 and WASI checks pass. The API snapshot
runner also records zenmeta's own feature-dependent surface and verifies
that repository instruction files are excluded from its package.

## Integration evidence at c1c3292f

The integration source `c1c3292ff10f386191780ad407526951165da47a`
was checked with `just metadata-error-check`, including the registry comparison
above: 196 checks passed and 58 were skipped. The review head `d2407559`
only adds migration documentation after that source revision. Re-run this gate
if source changes again; this is not a guarantee for future changes.

| Change relative to 0.1.26 | Compatibility assessment |
|---|---|
| New inventory module, capability and default trait operation | Additive; existing DecodeJob implementations acquire a default unsupported result without a new associated-error bound |
| Inventory returns a value or typed error, rather than Option | Change to an unpublished git API only |
| EXIF entries iterator and policy categories | Additive source API; existing non-exhaustive policy remains owned by zencodec |
| EXIF filtering retains validated color and separates identifiers/time offsets | Observable behavior change; recorded in CHANGELOG, not certified by semver-checks |
| Opaque MakerNotes dropped during EXIF rewriting | Observable behavior change; byte-exact KEEP_ALL remains available |
| HDR diffuse-white survives retained HDR metadata | Behavior repair recorded in CHANGELOG |
| Shared parsers extracted to zenmeta | Published zencodec types and useful baseline remain; codec wrappers retain their own public type identity |
| Combined animation timing additions | Released UnsupportedOperation discriminants preserved; check caught and repaired the initial drift |

The tool does not prove behavioral equivalence, arbitrary feature combinations,
or downstream associated-type compatibility. Codec audits therefore also
inspect associated error types manually. Their existing main-branch breaks
must not be attributed to the inventory addition or silently omitted from
release notes.

### Published-version recheck

On 2026-10-12, [docs.rs latest](https://docs.rs/crate/zencodec/latest) still
resolved to 0.1.26, matching this audit's downloaded release artifact. A fresh
crates.io API request was inaccessible and lib.rs returned HTTP 403; neither
failed request is treated as additional registry evidence.

### Rewrite contracts

The additional `metadata_rewrite` module contains only shared requests, runtime
service interfaces and completed plan/report types. Existing published items
are unchanged. After adding this module, `just metadata-semver` again passed
196 checks with 58 skipped against 0.1.26. Codec planner integration is separate
validation; the new `Plan::from_parts` constructor requires codec validation
and does not itself certify arbitrary container data.
