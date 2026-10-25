# The visual guide

The guide lives in `docs/` on main. GitHub Actions builds and checks it, then
publishes an allowlisted artifact to GitHub Pages. There is no publishing branch.
The source is `site/index.html`, `site/content.mjs`, `site/styles.css`, and
`site/app.mjs`; `build-site.mjs` produces ignored `docs/_site/`.

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
