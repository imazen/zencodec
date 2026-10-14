# New contract coverage

This is behavioral contract coverage, not a line-coverage percentage. The scope
is the unreleased timing, checked policy, inventory and rewrite changes reviewed
in [the published-API audit](zencodec-contract-audit-2026-10-13.md).

## Remaining gaps

- AVIF integration is blocked by interrupted, uncommitted adapter changes awaiting
  ownership approval. Its ravif, AOM and SVT configurations have not run these new
  helpers. Do not count an earlier native timing test as shared-suite coverage.
- Animation sink results expose no frame duration or index. This is an API gap,
  queued for the breaking release, rather than a check the harness can implement.
- Allocation-failure injection is absent. Capacity-overflow and typed resource-limit
  tests do not prove behavior at every allocator failure point.
- Required fidelity and a complete metadata-delivery report are queued API work.
  Checked policy acceptance alone cannot prove either. Unknown carriers require
  format-specific wire inspection; the metadata struct is not a complete inventory.
- HEIC coverage here uses the Rust backend. Runtime testing of every hardware
  decoder is a separate gate, even when its adapter compiles in CI.

## Approved testkit release boundary

The implementor-only testkit may break independently of zencodec. Version 0.2.0
carries the approved `RefDecodeJob` construction break from `857fab1`; use the
reference config's `job()` method instead of constructing the former unit value.
`just testkit-semver` evaluates the declared 0.2.0 release against crates.io 0.1.0.
It does not claim source compatibility across that breaking boundary. Zencodec's
separate checks against 0.1.26 remain enforcing. Publication is not authorized by
approval of this API break.

## Adapter matrix

The new applicable checks have passed locally for **17 of 18 format adapters** in
this matrix. RAW has additionally passed both rawloader and rawler configurations.
That fraction describes the fixture integrations below; it is not a claim that
all inputs, metadata fields, backends or API branches are covered.

Every row with inventory tests runs with the feature enabled and disabled. Every
consumer exposes `just shared-contract-check`; CI invokes the same tests. Pins
are temporary until the coordinated crates are published. These are review PRs,
not a claim that the changes are merged or published.

| Adapter | Checked policy | Inventory dispatch and input boundary | Rational timing | Rewrite | Review |
|---|---|---|---|---|---|
| JPEG | Encode + decode | Pass | Not animated | Strip + preserve, base + gain map | [226](https://github.com/imazen/zenjpeg/pull/226) |
| PNG/APNG | Encode + decode | Pass | Exact + ordinary, bounds/rejection | Strip + preserve | [43](https://github.com/imazen/zenpng/pull/43) |
| JPEG XL | Encode + decode | Pass | Exact + ordinary, bounds/rejection | Strip + preserve | [27](https://github.com/imazen/zenjxl/pull/27) |
| WebP | Encode + decode | Pass | Exact + ordinary, bounds/rejection | No rewrite implementation | [113](https://github.com/imazen/zenwebp/pull/113) |
| GIF | Encode + decode | Pass | Typed refusal; legacy frames survive | No rewrite implementation | [28](https://github.com/imazen/zengif/pull/28) |
| PNM, farbfeld, BMP, QOI, TGA, Radiance HDR (6) | Encode + decode | Pass | Not animated | No rewrite implementation | [30](https://github.com/imazen/zenbitmaps/pull/30) |
| RAW/DNG | Decode, rawloader + rawler | Pass on both | Not animated | No rewrite implementation | [24](https://github.com/imazen/zenraw/pull/24) |
| HEIC | Decode, Rust backend | Pass | No rational encoder | No rewrite implementation | [60](https://github.com/imazen/heic/pull/60) |
| TIFF | Encode + decode | Pass | No rational encoder | No rewrite implementation | [44](https://github.com/imazen/zenextras/pull/44) |
| JP2/J2K | Decode, both carriers | Pass | No encoder | No rewrite implementation | [44](https://github.com/imazen/zenextras/pull/44) |
| PDF | Decode | Pass | Pages, no encoder | No rewrite implementation | [44](https://github.com/imazen/zenextras/pull/44) |
| SVG | Decode | Pass | No encoder | No rewrite implementation | [44](https://github.com/imazen/zenextras/pull/44) |
| AVIF | Pending | Pending | Pending all encoder backends | No rewrite implementation | Ownership approval pending |

EXR has no `DecoderConfig` implementation, so it is outside this adapter count.
Its existing native inventory feature/disabled-operation and every-poll timeout
regressions also passed. GIF pixel encoding is tested with an explicit quantizer;
the no-quantizer configuration still runs decoder policy and inventory checks.
Bitmap and TIFF fixtures use strided rows; Radiance uses native linear f32 input.

The consumer policy tests admit `none()` and require typed rejection of unsupported
requests. They do not claim that every codec implements metadata denial. The
reference codec exercises supported delivery/denial and all 27 tri-state
ICC/EXIF/XMP combinations. Each decode-policy field is also tested independently.

## Reusable guarantees and their evidence

| Contract | Check and evidence |
|---|---|
| Exact representability, overflow and rejected-frame atomicity | `check_animation_timing`, explicit codec-clock cases with accepted frames surrounding rejected requests |
| Ordinary rounding and returned duration | The same helper with explicit 33/34/33 ms expectations; rejected requests must not advance phase |
| Borrowed/owned/erased timing, index and pixels; EOF | `check_animation_durations`, invoked by both timing helpers |
| Unsupported rational timing | `check_animation_timing_unsupported`, typed `AnimationTiming` through both dispatch paths, followed by successful legacy frames |
| Encode policy admission and atomicity | `check_encode_policy_admission`, builder/mutable/erased parity, explicit metadata expectations, decoded-pixel comparison to the initial state after rejection |
| Decode policy admission and atomicity | `check_decode_policy_admission`, typed errors and builder/mutable/erased pixel/metadata parity |
| Inventory optionality and dispatch | `check_inventory_dispatch`, including feature-disabled typed errors |
| Inventory byte cap | `check_inventory_input_limit`, typed InputSize at length minus one and success at exact length, generic and erased |
| Rewrite planning and limits | `check_metadata_rewrite`, chunks/materialization, report bounds, determinism, exact input/output budgets, typed input/output/carrier caps, pre-cancellation |
| Rewrite payload and retention | Caller observer plus format-specific wire checks and independent readers |
| Nested plan provenance, packet expansion, regenerated budgets | Existing zenmeta `tests/rewrite.rs`, reused rather than reimplemented in each codec |
| Inventory topology and typed resource failures | Existing zencodec inventory regression tests, plus consumer feature and mutation suites |
| CodecSet dispatch and transcode guards | Existing testkit `tests/codec_set.rs`, including checked policy through registry-created jobs |

Harness regression tests reject wrong timing, missing/extra frames, incorrect
metadata expectations and wrong admission results. Rewrite fault injections
bypass input/output/carrier/cancellation checks individually; each is detected.
The harness borrows full row ranges instead of requiring the newer
`PixelSlice::clone()`, retaining compatibility with zenpixels 0.2.14.

Independent readbacks run during this work: libjxl for rewritten JXL; libwebp via
Pillow for WebP timing/pixels; image-png for APNG and PNG rewrite pixels;
libjpeg-turbo and ExifTool for JPEG base/gain-map payloads and metadata. A codec
round trip alone is not the evidence for those output claims.

## Findings that changed implementation

- PNG PreserveExact rewriting dropped XMP when no XMP editor was supplied.
  The existing packet is now kept; strip mode still removes it. The shared test
  failed before the fix.
- GIF unsupported-operation errors lost their typed cause. The error now exposes
  its source, so callers and the shared timing-refusal check can inspect it.
- HEIC and its HDR helper had incompatible zencodec git revisions. The
  [HDR pin alignment](https://github.com/imazen/ultrahdr/pull/37) keeps gain-map
  metadata types unified during this unpublished integration.
- Animation sinks cannot expose source timing. A frame-aware sink result is
  recorded in the [breaking queue](../CHANGELOG.md#queued-breaking-changes).
