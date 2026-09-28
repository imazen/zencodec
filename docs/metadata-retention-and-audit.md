# Display-preserving metadata retention and inspection

This change builds on EXIF PRs [123](https://github.com/imazen/zencodec/pull/123),
[124](https://github.com/imazen/zencodec/pull/124), and
[125](https://github.com/imazen/zencodec/pull/125). It does not replace their
category policy, borrowing entry iterator, or offset-safe EXIF writer.

## Contract

1. Extract the rendering interpretation **before** removing its source carrier.
   ISO 21496-1 parameters are authoritative when present. Legacy Apple headroom
   comes from its MakerNote. Malformed authoritative data must not become a
   default curve or a fallback to a different source.
2. Apply `MetadataPolicy` to source metadata. `Web` retains EXIF attribution;
   `ColorAndRotation` removes attribution. Both drop source XMP and MakerNotes.
3. Retain the base/gain-map pixels and typed `GainMapParams`. A container writer
   generates fresh gain-map discovery, parameters, lengths and offsets. Dropping
   source XMP must not switch off this required output signaling.
4. Filter encoded auxiliary images too. Removing only the primary APP1 leaves
   private XMP, comments or MakerNotes in the secondary JPEG. Old MPF offsets,
   thumbnails and trailing auxiliary files must not be copied blindly.
5. Refuse an unsupported rendering contract. Color conversion, orientation
   baking and falling back to SDR are separate, explicit pixel operations.

```rust
use zencodec::{MetadataPolicy, display_metadata};
# fn example(source: &zencodec::Metadata, params: &zencodec::GainMapParams)
# -> Result<(), display_metadata::Error> {
let retained = display_metadata::filter_for_gain_map(
    source, params, &MetadataPolicy::ColorAndRotation,
)?;
// Send retained metadata AND params to a gain-map-aware container writer.
# Ok(()) }
```

This guard does not serialize a container or certify arbitrary metadata private.
Ordinary `Metadata::filtered` remains useful for ordinary images. Required
container metadata belongs to the codec, rather than a blanket XMP exception.

## Read, audit and diff

The `metadata-audit` feature adds no third-party dependency. `Report::exif`
produces directory/tag/occurrence paths, TIFF types/counts, decoded numbers and
escaped text or hex bytes. Unknown tags and duplicate valid entries remain
visible. MakerNotes are explicitly opaque. The existing EXIF parser salvages
valid entries and does not traverse arbitrary SubIFDs, so every EXIF report
contains a coverage finding: an empty diff is **not** proof that the original
files or all vendor metadata are equivalent. Keep originals for forensic use.

The `xmp` feature adds roxmltree with defaults disabled, and includes auditing.
`Report::xmp` records elements, expanded namespace names, attributes, text,
comments and processing instructions. Unknown application properties survive
inspection. Prefix aliases compare equally; this is a structural XML diff, not
full RDF graph canonicalization (attribute vs element forms, bags, aliases and
qualified values need not compare equally).

```rust
# #[cfg(feature = "xmp")] {
use zencodec::metadata_audit::Report;
let before = Report::xmp("<x secret='one'/>");
let after = Report::xmp("<x secret='two'/>");
for finding in before.findings().iter().chain(after.findings()) {
    eprintln!("coverage: {finding:?}");
}
for change in before.diff(&after) { println!("{change:?}"); }
# }
```

Executable packet inspector, with optional comparison:

```sh
cargo run --example audit_metadata --features xmp -- xmp before.xmp after.xmp
cargo run --example audit_metadata --features xmp -- exif metadata.exif
```

These inputs are extracted packets, not whole image files. Container enumeration
remains with each codec; JPEG's marker iterator exposes opaque APP carriers and
all component boundaries. The Apple borrowing entry view lives in
`ultrahdr_core::metadata::apple::AppleMakerNote`: numeric tag/type/count/raw value
and an error per unreadable entry. Other vendor schemas are not silently guessed.

Selected XMP property reads are stricter than inspection: namespace identities,
empty-subject RDF Description, scalar or sequence shape, finite validated gain
parameters. Duplicate declarations, conflicting subjects, qualifiers and invalid
numeric values refuse instead of choosing one. XMP parsing rejects DTDs and
external entities, caps source at 16 MiB, nodes at 65,536 and nesting at 64.
Audit expansion has a separate 16 MiB budget. All operations are explicit and
metadata-only; none scan pixels or clone 40–100 MB decoded image allocations.
Encoded-JPEG filtering necessarily walks/copies compressed bytes, and the
current assembler makes additional encoded-buffer copies. It is not zero-copy.

## Choosing implementation dependencies

| Component | Choice and utility |
|---|---|
| EXIF | Keep existing borrowing parser/pruner and #125 iterator. kamadak-exif 0.6.1 is already a dev-only differential oracle; it offers mature tag names/display and read support, but not the existing retention/rewrite contract. |
| XML | Reuse roxmltree 0.21.1 for namespace resolution, entities and XML syntax. Implement only bounded XMP selection/audit on top. The old JPEG prefix/string scanner could mistake unrelated `Item:Length` or miss renamed namespaces. |
| Full XMP | xmpkit 0.1.6 provides a much broader RDF editing and file-handler API. Its minimal `core` build is plausible for a separate editor; it is not needed in every codec. Default features also pull PDF and threaded/file support. |
| MakerNotes | Reuse and harden the existing Apple decoder; expose its borrowed raw entries. No new all-vendor decoder in the codec dependency graph. A broad metadata tool can layer vendor interpretation over opaque entries. |

Primary references: [roxmltree](https://github.com/RazrFalcon/roxmltree),
[kamadak-exif](https://github.com/kamadak/exif-rs),
[xmpkit](https://github.com/cavivie/xmpkit),
[ExifTool Rust port](https://github.com/Le-Syl21/exiftool-rs),
[Android Ultra HDR container requirements](https://developer.android.com/media/platform/hdr-image-format).

Do not mistake xmpkit (pure Rust) for Adobe's C++ SDK bindings. Also do not
interpret broad advertised vendor support as verified correctness for private
MakerNote rewriting: unknown layouts/offset bases need dedicated fixtures.

## Compile measurements

See [raw runs](measurements/metadata-builds.json). Linux x86_64, recorded rustc/CPU
in the JSON; `cargo build --lib --offline --locked -j4`, three independent target
directories per case, dependency downloads excluded. The filesystem/OS caches
are warm: "cold" means fresh Cargo artifacts, not a reboot. The edited case
changes the small consumer, not the dependency. These are development builds,
not release/LTO timings. No concurrent builds ran during measurement.

| Consumer dependency | Cold median | Warm median | Edited consumer median |
|---|---:|---:|---:|
| zencodec #125 baseline | 2.169 s | 0.012 s | 0.020 s |
| zencodec new, default features disabled | 2.182 s | 0.012 s | 0.020 s |
| zencodec + metadata-audit | 2.220 s | 0.012 s | 0.021 s |
| zencodec + xmp | 2.306 s | 0.013 s | 0.021 s |
| roxmltree alone | 0.432 s | 0.010 s | 0.020 s |
| kamadak-exif alone | 0.358 s | 0.011 s | 0.018 s |
| xmpkit core, defaults disabled | 1.997 s | 0.015 s | 0.024 s |
| xmpkit default features | 8.302 s | 0.031 s | 0.039 s |

The core default delta (~0.6%) is smaller than run variation. XMP adds ~0.14 s
(~6.3%) to this small zencodec build; independent dependency times do not add
linearly because Cargo compiles in parallel. XML/audit are off by default in
zencodec; zenjpeg opts into XMP for its existing container parser.

Reproduce with `python3 scripts/measure-metadata-builds.py --work /tmp/metadata-builds`.
Exact resolved versions and dependency trees are in the raw report. The script
archives the baseline and snapshots the current source, without worktrees.

## Explicit limits

- Arbitrary source XMP preservation/merging is not implemented by the privacy
  encoder. Keeping it may retain identity/history/stale offsets, so that request
  refuses. Required output gain-map XMP is always regenerated.
- ICC profiles remain opaque rendering dependencies and can contain identifying
  tags. Removing or replacing arbitrary ICC without color conversion is unsafe.
- HDR-base and alternate-color-space XMP output is refused by the new path until
  its writer can represent those contracts. CLL/mastering metadata that the JPEG
  path cannot carry also refuses; it is not silently stripped.
- No automatic pixel scans, quantization, gamut conversions or SDR fallback.
- EXIF and XML reports expose structural changes, not a whole-file privacy
  certificate. Signed provenance cannot survive a rewrite as a valid signature.

### Actual JPEG consumer and broad vendor-reader comparison

[Raw codec runs](measurements/metadata-codec-builds.json) use the same method,
with `zenjpeg` default features plus `ultrahdr`. Source snapshots compare
zenjpeg `8f703a6e` and this implementation, each with the corresponding zencodec
baseline/new implementation. The same zenanalyze revision is pinned in both.

| Consumer | Cold median | Warm median | Edited consumer median |
|---|---:|---:|---:|
| zenjpeg before | 8.935 s | 0.028 s | 0.038 s |
| zenjpeg after | 8.881 s | 0.028 s | 0.039 s |
| exiftool-rs 0.8.2 (default features) | 14.818 s | 0.027 s | 0.025 s |

The JPEG difference (-0.6%) is within observed variation: **no measurable cold
build regression in this sample**, not a claim that more code compiles faster.
The broad ExifTool Rust port advertises many vendor decoders and file formats;
its ~14.8 s cold build is a reasonable cost for a dedicated auditing application,
but disproportionate for mandatory codec metadata retention. Its full vendor
correctness and rewriting behavior were not audited here. Keep it outside the
hot codec dependency graph; the native reports deliberately expose the boundary
between decoded fields and unknown vendor data.

## Review and landing order

| Repository | PR | Dependency |
|---|---|---|
| zencodec | [#128](https://github.com/imazen/zencodec/pull/128) | Stacked on existing #125 → #124 → #123 |
| zenjpeg | [#210](https://github.com/imazen/zenjpeg/pull/210) | #128; exact root/fuzz pins |
| ultrahdr | [#35](https://github.com/imazen/ultrahdr/pull/35) | #128 and zenjpeg #210 |
| heic | [#51](https://github.com/imazen/heic/pull/51) | Hardened Apple parser from ultrahdr #35 |
| zenpipe/zencodecs | [#84](https://github.com/imazen/zenpipe/pull/84) | All above |

The codec PRs target their own `main`; they are cross-repository dependencies,
not accidental branches of unrelated feature PRs. No crate is published by this
change. Before publishing, land the EXIF stack in order, retarget #128 as its
parents land, then publish the shared API and advance dependency version floors
before removing the temporary git patches.

Zenpipe's shared `cargo superwork ci-clone --add-paths` workflow checks out
sibling `main`s and rewrites dependency pins to those paths. That workflow cannot
exercise a cross-repository PR stack until its parents land. The standalone
pinned builds and their explicit test commands are the implementation validation;
do not treat a substituted-main CI run as testing these exact dependency commits.
The existing planar migration branch and an unrelated AVIF animation/API-snapshot
failure are separate prerequisite CI work in zenpipe, not suppressed here.

### Other affected consumers

[Raw consumer runs](measurements/metadata-consumer-builds.json) and
[snapshot/case generator](../scripts/metadata-consumer-build-cases.py) cover
ultrahdr-core without default features, HEIC with backend-rust/std/zencodec, and
zencodecs without defaults plus jpeg-ultrahdr. Three fresh artifact directories
per case, four jobs, no concurrent builds, same development build method.

| Consumer | Before cold median | After cold median | Difference |
|---|---:|---:|---:|
| ultrahdr-core | 3.312 s | 3.330 s | +0.5% |
| HEIC adapter | 5.770 s | 5.086 s | -11.9% |
| zencodecs JPEG/UltraHDR dispatch | 8.420 s | 8.622 s | +2.4% |

HEIC includes the ultrahdr-core 0.5 → 0.6 dependency migration, so its reduction
must not be attributed to the new parser alone. Warm medians remain 18–53 ms;
edited consumer medians 25–62 ms. These small samples are compile-cost estimates,
not precise performance guarantees.

The zencodecs comparison uses `--offline` without `--locked`: Cargo repeatedly
wanted to normalize unused patch records between fetch/tree/build, even after an
untimed preparation build. Both before/after use the same unlocked/offline method;
raw results record that difference and resolved dependency trees. Failed locked
attempts are excluded, and none of their target directories were reused for the
reported cold runs. Other reported comparisons use locked builds.

```sh
python3 scripts/metadata-consumer-build-cases.py \
  --zen-root /path/to/zen --work /tmp/metadata-consumers
python3 scripts/measure-metadata-builds.py \
  --work /tmp/metadata-consumer-builds --extra-cases /tmp/metadata-consumers/cases.json \
  --only ultrahdr-core-before ultrahdr-core-after heic-before heic-after
python3 scripts/measure-metadata-builds.py \
  --work /tmp/metadata-dispatch-builds --extra-cases /tmp/metadata-consumers/cases.json \
  --only zencodecs-before zencodecs-after --unlocked
```


### Validation and remaining CI gates

The metadata-focused local gates pass: core workspace all-feature tests/clippy,
no_std XMP check, JPEG full library suite (1,166 passed, three ignored), Apple
inspection (nine), UltraHDR production assembly (ten), HEIC adapter (122), and
zencodecs JPEG/HEIC library suite (154). The reconstruction test compares decoded
HDR bytes exactly, not only parsed metadata. Pinned JPEG/HEIC/UltraHDR and default
zencodecs clippy pass with warnings denied; reduced-feature dispatch emits
existing unused-code warnings. The public JPEG API snapshots are regenerated.

The ARM JPEG encoder quality-floor failure is also present on the exact upstream
main baseline: `baseline_444_opt Q50`, zensim 44.6 versus floor 47.0
([baseline CI](https://github.com/imazen/zenjpeg/actions/runs/36344883310/job/108691957491)).
This task does not lower that threshold or alter the ordinary JPEG pixel encoder.
Zenpipe's shared CI prerequisites are described above. These PRs are reviewable
implementations, not a claim that the whole ecosystem is ready to publish.
