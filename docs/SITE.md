# The visual guide

The guide lives in `docs/` on main. GitHub Actions builds and checks it, then
publishes an allowlisted artifact to GitHub Pages. There is no publishing branch.
The source is `site/index.html`, `site/content.mjs`, `site/styles.css`, and
`site/app.mjs`; `build-site.mjs` produces ignored `docs/_site/`.
`site/api.mjs` selects the source declarations used by each chapter and entry
point. `site/source.mjs` extracts those signatures and complete example scopes.

```sh
npm ci --prefix docs --ignore-scripts
just docs-site-check
just docs-site-serve
# In another terminal, with a browser installed:
just docs-site-browser
```

The server binds port 3537 on all interfaces. `SITE_URL` selects another preview
URL; `SITE_SCREENSHOTS` selects a durable screenshot directory. For an existing
Chromium installation, set `PLAYWRIGHT_CHROMIUM_EXECUTABLE`. Otherwise install
one with `cd docs && npx playwright install chromium`. Set `TMPDIR` to a
persistent scratch directory when running locally.

## Keeping examples truthful

The build extracts marked fragments from `examples/journeys.rs`, never a second
hand-copied code block. `perf/io-validation/tests/site_examples.rs` executes
that same file against pinned native QOI with strided, nonopaque pixels.
It checks exact samples across convenience, limited, preferred-layout,
metadata-policy composition, slice, positioned, async, whole-file bridge and
finish-first paths. The limit rejects just below the fixture's pixel count.
The returned pixel descriptor is checked as RGBA8 with unknown transfer; a
preference for sRGB does not relabel the native result.

Each `EXAMPLE` region encloses a standalone import/function scope compiled in
its own Rust module. Its `ANCHOR` region is the initial focus. Native details
controls expand the preceding imports and setup above, and the return path or
continuation below. The two controls are independent and remain in source
order. Copy-full includes both regions even when collapsed. Source tests check
that the three rendered pieces reconstruct the compiled scope exactly.

The entry selector covers config decode, probe and encode; configured job
encode; and `CodecSet` registration/decode, encode and transcode. The journeys
also show registry jobs and registry incremental dispatch. Every chapter has
signature panels, with the primary interface open initially. These include
receiver ownership, associated types, lifetimes and where bounds. They are
declaration excerpts with bodies omitted, not complete trait implementations.
Links point to the built revision's source. A missing or ambiguous declaration
fails the build. The extractor is a scanner for these selected source forms,
not a general Rust parser.

These are shared-interface examples with a small lossless codec fixture, not a
browser-format recommendation or proof that QOI retains EXIF/ICC metadata.
Media and implementor chapters link the current contracts and separately
qualified implementations; their prose is not presented as an executable codec.

Prepare the pinned harness as described in
[its README](../perf/io-validation/README.md), then run:

```sh
just docs-site-examples
```

The Documentation workflow runs these examples and the ownership control,
regenerates the transport CSV and requires an exact match, checks all site
anchors/local assets, and exercises all 15 chapters, deep links, copy controls,
keyboard navigation, the eight lab settings, and 390/768/1440px viewports.
It also checks all seven entry points, actual clipboard contents against the
compiled source, independent above/below expansion, and the expanded mobile
layout. The entry selector keeps its position while switching examples.
Deployment depends on both the browser/build job and Rust example job. Pull
requests validate without deployment. The build publishes only seven explicit
files; it does not upload the documentation directory or repository wholesale.

## Evidence and review

The lab reads the committed
[transport matrix](../benchmarks/io_transport_model_2026-10-25.csv); it does not
interpolate or run a second browser implementation of the model. It distinguishes
calculated network costs, exact pixel/ownership tests, and measured performance.
The colored flowcharts also carry role labels and text descriptions.

Local qualification (2026-10-25, r7900x, Rust 1.99.1): the native guide example
and all-target experimental-harness clippy passed. Full browser results are
recorded by the Documentation workflow on each deployed revision. The guide's
footer identifies the built commit. Scheduling and admission proposals stay
linked to their open issues; no public library API was introduced by this site.

```
examples and clippy: rc=0 0s | peak-RSS 0.20GiB | min-avail 16109MiB | peak-load 0.33
browser: rc=0 5s | peak-RSS 0.22GiB | min-avail 15855MiB | peak-load 1.16
```

Context revision qualification (2026-10-25): the two native guide tests and
all-target harness clippy passed. Source reconstruction/signature tests and
the browser interactions described above passed. Wrapper observations:

```
examples and clippy: rc=0 0s | peak-RSS 0.20GiB | min-avail 18196MiB | peak-load 0.26
browser: rc=0 6s | peak-RSS 0.24GiB | min-avail 17780MiB | peak-load 1.02
```

## CodecSet fixture coverage

The guide's executable scopes include config `probe`/`decode`/`encode`,
job-level encode, registry registration and detection, registry encode and
per-operation dyn jobs, and registry incremental dispatch. The registry fixture
reuses one set for native QOI and BMP and compares exact strided RGB samples.
Unrecognized input and a set-level pixel limit are exercised too.

The pinned bitmap adapters also reproduce a concrete transcode limitation:
`decode` returns an unknown-transfer descriptor, while the QOI encoder advertises
sRGB descriptors. `CodecSet::transcode` correctly refuses that mismatch with
`TranscodeBarrier::PixelDescriptor`. This is recorded in
[zenbitmaps issue 28](https://github.com/imazen/zenbitmaps/issues/28). The example
shows the actual refusal boundary; it does not bypass the guard or silently
relabel pixels. The test accepts a future successful path only with exact
sample parity. Expanded examples each have their own compiled import scope.

## Compact examples and sequential reader assessment

Code panels prioritize the complete-example copy and a direct source link.
Imports/setup and continuation still expand independently in source order.
Additional copy and native-test links are under More. The primary signature
remains open, with methods selected for the operation being taught. In
particular, decoder registration does not also show encoder registration bounds.

Rust highlighting uses [Shiki](https://shiki.style/guide/install) at build time.
Only static token markup is deployed; no highlighter or grammar is loaded by the
reader. Browser checks compare rendered and copied text to the complete compiled
scope, including whitespace. The signature check ignores color spans while
requiring the same error type text.

The [sequential learning protocol](evaluation/README.md) fixes four native coding
oracles and two conceptual rubrics before the reader assessment. Baseline and
revised readers receive visible and expanded packets separately, without source
lookup or compiler feedback. This measures a restricted model workflow, not a
human success rate. Targeted desktop/mobile captures use `just guide-visuals`.

Compact-layout verification: source/build checks and the existing clipboard,
inline expansion, navigation and responsive browser checks passed on 2026-10-25.
The registry entry measured 707.640625 CSS px at 1440 px viewport width in the
first revised capture. Raw captures stay with the assessment artifacts; the
completed comparison is recorded separately from these layout checks.

```
browser: rc=0 6s | peak-RSS 0.18GiB | min-avail 18976MiB | peak-load 0.80
visuals: rc=0 4s | peak-RSS 0.19GiB | min-avail 18940MiB | peak-load 0.47
```

The blind reader identified a debugging defect in the earlier driver samples:
`map_err(|e| e.to_string())` discarded structured causes. All six samples now
propagate with `?`. A source-timeout regression test first failed on the old
sample and now recovers the original `std::io::Error` kind and message. Native
example/policy tests and Clippy passed:

```
examples and clippy: rc=0 1s | peak-RSS 0.20GiB | min-avail 19466MiB | peak-load 0.10
```

The JPEG progression now shows the actual retention preset in the initial
example and an executable public-plan ICC refusal before the private resolver
adapter. The latter expands as an adapter design detail. The implemented guard
has its own typed application error, checks before any write, and uses borrowed
plan chunks. Baseline/progressive native tests require zero output on the ICC
refusal and scan-byte plus independent decoded-pixel equality on accepted input.
It is a retained-ICC requirement, not a universal PII certification. This change
adds no exported library API.

```
JPEG tests and clippy: rc=0 2s | peak-RSS 0.24GiB | min-avail 19389MiB | peak-load 0.97
```

The JPEG dependency bridge still emits the previously recorded root-crate HDR
alias deprecations; the guide change does not suppress them.
