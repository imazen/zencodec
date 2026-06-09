# zencodec 0.1.21 codec-integration sweep — 2026-06-08

Tracks adoption of the **metadata-retention** + **color-emission** API (merged in
zencodec PR #17 / 0.1.21, squash commit `23d4046`) across every zen* codec, the
"unexpected results" footguns each one has today, and the changes made per repo.

This is the canonical status doc for the sweep. Update the per-codec **Status** and
the **Changes made per repo** log as work lands. Source audits (one deep agent per
codec, 2026-06-08) back every finding below.

---

## How a codec adopts the new API

Two independent stories:

### 1. Metadata retention (mostly free)
`EncodeJob::with_metadata_policy(meta, MetadataPolicy)` is a **provided** trait
method — it filters via `Metadata::filtered` (which also reconciles the embedded
EXIF orientation tag against the authoritative `orientation` field) and then calls
the codec's `with_metadata`. **Codecs need no change for it to work.**

`with_metadata` is now `#[deprecated]`. It is still a **required** trait method
(no default), so every codec must keep implementing it — and Rust does **not** warn
at impl sites, only at **call** sites. So the migration surface is: change callers
(mostly in-repo tests) from `.with_metadata(meta)` to
`.with_metadata_policy(meta, MetadataPolicy::PreserveExact)` (exact round-trip
tests) or `MetadataPolicy::Web` (privacy-safe default).

### 2. Color emission (real per-codec work)
`resolve_color_emit` is a free function the **encoder calls internally**:

```rust
let policy = self.policy.resolve_color(ColorEmitPolicy::Balanced); // EncodePolicy → ColorEmitPolicy
let mut src = zencodec::SourceColor::default().with_channel_count(n);
if let Some(c)   = meta.cicp        { src = src.with_cicp(c).with_color_authority(ColorAuthority::Cicp); }
if let Some(icc) = &meta.icc_profile{ src = src.with_icc_profile(icc.clone()).with_color_authority(ColorAuthority::Icc); }
let plan = zencodec::resolve_color_emit(&src, Config::capabilities(), policy);
```

Lower the returned `ColorEmitPlan { cicp: Option<Cicp>, icc: IccDisposition }`:
- `plan.cicp` → the format's native CICP carrier (JXL enum color, AVIF/HEIC `nclx`,
  PNG `cICP`); formats with no carrier ignore it.
- `IccDisposition::KeepSource` → embed the source ICC bytes.
- `IccDisposition::SynthesizeFrom(cicp)` → `zenpixels_convert::icc_profile_for_primaries(cicp)`
  (const table; `None` for sRGB/BT.709 → embed nothing).
- `IccDisposition::Drop` → no ICC.

**Capabilities** the encoder must declare honestly (`zencodec::encode::EncodeCapabilities`):
`with_cicp(b)` (has any CICP carrier), `with_cicp_is_valid_carrier(b)` (standardized
& honored), `with_cicp_safe_sole_carrier(b)` (safe to ship CICP only + drop ICC).

#### Per-format capability truth table (2026 reliability findings)
| format | icc | exif | xmp | cicp | valid_carrier | sole_safe | native carrier |
|--------|-----|------|-----|------|---------------|-----------|----------------|
| JPEG   | ✓ | ✓ | ✓ | ✗ | ✗ | ✗ | none (synthesize ICC from CICP) |
| PNG    | ✓ | ✓ | ✓ | ✓ | ✓ | ✗ | `cICP` chunk |
| WebP   | ✓ | ✓ | ✓ | ✗ | ✗ | ✗ | none (synthesize ICC) |
| GIF    | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ | none (palette only) |
| AVIF   | ✓ | ✓ | ✓ | ✓ | ✓ | ✗ | `nclx` |
| HEIC   | ✓ | ✓ | ✓ | ✓ | ✓ | ✗ | `nclx` |
| JXL    | ✓ | ✓ | ✓ | ✓ | ✓ | **✓** | codestream enum color |
| TIFF   | ✓ | ✓ | ✓ | ✗ | ✗ | ✗ | none (synthesize ICC) |
| BMP/PNM/PFM/QOI/HDR/TGA/farbfeld | ✗* | ✗ | ✗ | ✗ | ✗ | ✗ | none (*BMPv5 ICC only, not wired) |

---

## ⛔ Cross-cutting blocker: zencodec 0.1.21 is unpublished

crates.io tops out at **0.1.20** (the commit *before* PR #17). Every codec depends
on a published 0.1.13–0.1.20, so **none can see** `resolve_color_emit`,
`ColorEmitPolicy`, `with_metadata_policy`, `EncodePolicy::resolve_color`, or the
`with_cicp_is_valid_carrier`/`with_cicp_safe_sole_carrier` builders today.

**Dev bridge (the "cargo patch to this folder") — ⚠ TEMPORARY, must be removed:**
a single workspace-level override at **`/home/lilith/work/zen/.cargo/config.toml`**:
```toml
paths = ["/home/lilith/work/zen/zencodec"]
```
This redirects zencodec to the local 0.1.21 for *every* codec under `~/work/zen/`
at once. 0.1.21 satisfies the existing `^0.1.x` requirements, so **no `Cargo.toml`
version bump is needed to build+test**. That dir is not inside any git repo, so
there is zero tracked-file impact.

Mechanism note: the legacy `paths` override is used, **not** `[patch.crates-io]` —
a `[patch.crates-io]` table in a `.cargo/config.toml` is silently *not applied*
(cargo falls back to the registry version), whereas `paths` reliably replaces
zencodec by name across the whole graph. It emits one cosmetic
"path override / dep mismatch" warning (zencodec 0.1.21 added `kamadak-exif`).
**⚠ This file must be reverted to just the wasm `runner` line when the sweep is
done / 0.1.21 publishes — otherwise every `~/work/zen` build silently uses local
zencodec.**

**Landing strategy (two layers):**
1. **Now (this sweep):** patch → implement → e2e test → report. Integration is
   validated by *dogfooding* the API before it's frozen by release. Codec changes
   are held as **provisional** (not pushed to a codec's `main` — they'd break CI
   against the published dep).
2. **Later (user-gated):** publish zencodec 0.1.21 (full release ceremony, user
   approval). Then each codec lands a real commit: bump dep to `0.1.21`, drop the
   dev patch, push.

This order is deliberate: if integration surfaces an API gap, we fix zencodec
*before* publishing — not after.

---

## Priority / status

| # | codec | dep | readiness | key work | effort | status |
|---|-------|-----|-----------|----------|--------|--------|
| 1 | zenjxl | 0.1.19→.20 | provisional `@` already on merged API | finalize + jpeg_lossy color + test migrations | M | ✅ done (provisional `f7be9506`) — color-emit + **jpeg_lossy ICC preservation** + migrations |
| 2 | zenjpeg | 0.1.20 | provisional emit-integ on OLD API | rewrite vs new API (descriptor threading blueprint exists) | M | ✅ done (provisional `b5df9c72`) — decode double-rotation fix + color-emit + caps |
| 3 | zenavif | 0.1.19 | clean | **fix apply_descriptor_color CICP-override (High)** + color-emit wiring | M | ✅ done (provisional `9c71eff0`) — CICP-override fixed + color-emit + caps |
| 4 | zenpng | 0.1.19 | clean | caps flags + resolve_color_emit + decode orientation extraction | M | ✅ done (provisional `e7bd4e66`) — caps + color-emit + decode orientation |
| 5 | zenwebp | 0.1.13 | far behind | dep bump + color-emit (synthesize ICC) + drop bespoke gating | M | ✅ done (provisional `3a981325`) — ICC synthesis + policy path |
| 6 | zentiff | 0.1.19 | clean | **metadata currently DROPPED entirely (High)** — wire real embedding | M–L | ✅ done (provisional `58e69cb1`) — real embedding + **proper EXIF decomposition (IFD0/EXIF/GPS)** + caps + color-emit |
| 7 | zengif | 0.1.19 | already honest | verify compatible/honest (no code change) | S | ✅ verified green vs 0.1.21 — no change needed |
| 8 | zenbitmaps | 0.1.20 | already honest | verify compatible/honest (no code change) | S | ✅ verified green vs 0.1.21 — no change needed |

**Sweep result: all 8 codecs covered. 6 carry provisional (unpushed) integration commits; 2 (gif, bitmaps) are already honest and verified compatible. Every suite green against patched 0.1.21. Nothing pushed — landing is gated on publishing zencodec 0.1.21 (see Landing checklist).**

Secondary (decode-mostly / wrappers, audit in wave 2 if wanted): heic, zenraw,
mozjpeg-rs, ultrahdr, zenpdf, zenjp2, zensvg.

---

## Per-codec findings + plans

> Severity legend: **High** = wrong pixels / silent data loss / privacy leak on a
> default path. **Med** = ignored policy / capability dishonesty. **Low** = polish.

### 1. zenjxl  — impl `src/codec.rs`, `src/decode.rs`, `src/jpeg_lossy.rs`
Provisional color work already lives in the working tree on top of `5371af6b` and
**already targets the merged `resolve_color_emit`/`ColorEmitPlan`/`IccDisposition`
API** (correct symbol names, `policy.resolve_color(Balanced)`, caps with
`cicp_safe_sole_carrier(true)`). The `--emit-integ` worktree is the older
removed-API version — nothing to salvage (its `cicp_to_jxl_color_encoding` is
byte-identical to `@`'s).

- **High** — provisional color integration is **uncommitted working-tree edits**;
  fragile. Commit it first.
- **Med** — `src/jpeg_lossy.rs` recompress paths emit bare codestreams with **no
  color signaling** — a Display-P3/AdobeRGB source JPEG is silently relabeled sRGB.
- **Med** — 5 test call sites use deprecated `with_metadata` (codec.rs:2322, 2851,
  2888, 2924, 3015).
- **Low** — animation encode path embeds no enum color / ICC (color drop for animated JXL).

**Plan:** keep `resolve_jxl_color` + caps as-is (already correct); commit the
working tree; preserve source color in `jpeg_lossy.rs`; migrate 5 test sites to
`with_metadata_policy(.., PreserveExact)`. Decode already populates cicp/icc/orientation.

### 2. zenjpeg  — impl `zenjpeg/zenjpeg/src/codec.rs` (`build_request_from` @498)
- **High** — encode embeds EXIF/ICC/XMP with **no retention policy** on the default
  path → GPS/camera/timestamp leak.
- **High** — decode bakes orientation upright, reports `Identity`, but **emits the
  original EXIF blob with the stale orientation tag** (codec.rs:1962+1969, 2263-2267,
  1211-1212) → double-rotation in any consumer that re-applies the tag.
- **Med** — `meta.cicp` never read on encode; ICC embedded unconditionally (redundant
  sRGB ICC bloat; no `ColorEmitPolicy`).
- **Low** — `SourceColor` under-populated on decode; `correct_color` hardcodes `Cicp::SRGB`.

**Plan:** thread the pixel `PixelDescriptor` into `build_request_from` (blueprint in
`zenjpeg--emit-integ` commit e18883cf — reuse the plumbing, rename to the new API);
call `resolve_color_emit` (JPEG caps: all cicp flags false) → JPEG has no carrier so
only ICC `KeepSource`/`SynthesizeFrom`/`Drop` matters (fixes redundant-sRGB bloat);
reconcile decode orientation via `helpers::set_exif_orientation(.., 1)` before
re-attaching the EXIF blob; reuse the emit-integ test shapes.

### 3. zenavif  — impl `src/codec.rs`; parser `zenavif-parse` (unaffected)
- **High** — `apply_descriptor_color` (codec.rs:823-831, called @1112) **unconditionally
  overwrites** `transfer_characteristics`/`color_primaries` from the pixel descriptor,
  ignoring a caller's `Metadata.cicp`. Caller sets Display-P3, descriptor reads sRGB →
  wrong `nclx` written → recolored image. (zencodec known-issue #2, **confirmed**.)
- **High (same root)** — the descriptor path sets primaries+transfer but **never
  `matrix_coefficients`** → internally-inconsistent nclx (stale MC).
- **Med** — no `resolve_color_emit`; `EncodePolicy::resolve_color` unused; sRGB ICC
  never shed alongside nclx.
- **Med** — default `with_metadata` path leaks EXIF/XMP; impl @517 needs
  `#[allow(deprecated)]` after the bump.
- **Low** — missing `with_cicp_is_valid_carrier(true)`/`with_cicp_safe_sole_carrier(false)`.

**Plan:** fix `apply_descriptor_color` to only fill from the descriptor when the
metadata CICP left that axis unspecified (and set MC); better — route color through
a `SourceColor`+`resolve_color_emit` single-source-of-truth; add the two cap flags.
Decode is already correct (`convert_native_info` populates full Cicp incl. MC+range).
**Note:** do NOT edit zenavif-parse (unaffected). This is zenavif's own repo.

### 4. zenpng  — impl `src/codec.rs`; chunk writer `src/encoder/metadata.rs`
- **High** — `EncodeCapabilities` omits `with_cicp_is_valid_carrier(true)` /
  `with_cicp_safe_sole_carrier(false)` (codec.rs:350) → `resolve_color_emit` sees
  PNG's `cICP` as an invalid carrier and silently drops it.
- **High** — `apply_encode_policy` (codec.rs:2443) ignores `policy.color` entirely;
  `resolve_color_emit` never called → `ColorEmitPolicy` has zero effect; redundant
  sRGB iCCP never dropped.
- **Med** — default `with_metadata` path = privacy footgun; test call site codec.rs:5581.
- **Med** — decode parses `eXIf` but **never extracts orientation** into
  `ImageInfo.orientation` (stays `Identity` while the blob says e.g. 6) (codec.rs:2387).

**Plan:** add the two cap flags (matches `caps_png()` fixture); wire `resolve_color_emit`
into the metadata-build path feeding `PngWriteMetadata`; extract the eXIf orientation
tag into `ImageInfo.orientation` on decode (PNG never bakes — report the stored tag).

### 5. zenwebp  — impl `src/codec.rs` (feature-gated)
- **High** — dep is `0.1.13` (8 patches behind); bump is the gate for everything.
- **High** — default `with_metadata` path embeds EXIF/XMP verbatim (privacy footgun).
- **Med** — bespoke `EncodePolicy.resolve_icc/exif/xmp` gating (codec.rs:497-522)
  reinvents retention all-or-nothing; replace with `MetadataPolicy`/`Metadata::filtered`
  (gives web-safe sub-field EXIF filtering: keep orientation+rights, drop GPS).
- **Med** — no CICP→ICC synthesis: a CICP-only source becomes untagged sRGB-assumed.

**Plan:** bump to 0.1.21; thread a `cicp: Option<Cicp>` into `WebpEncodeJob`; call
`resolve_color_emit` in `do_encode` (WebP caps: cicp false) → `SynthesizeFrom` an ICC
for CICP-only sources; drop the bespoke gating, rely on `with_metadata_policy`; set
`source_color.color_authority` on decode.

### 6. zentiff  — impl `src/codec.rs` (feature-gated); core `src/encode.rs`
- **High** — `with_metadata` stores into a field literally named `_metadata`, and
  `encoder()` **drops it** (codec.rs:203-219); encode writes **no** ICC/EXIF/XMP/
  orientation tags. Round-trip decode→encode **silently strips all metadata + color**.
- **High** — the blessed `with_metadata_policy` therefore embeds nothing (data-fidelity
  + privacy footgun: user believes filtered metadata is written; nothing is).
- **Med** — `DecodeCapabilities` omits `with_multi_image(true)` though decode emits
  `ImageSequence::Multi` (dishonesty).
- **Med** — `EncodeCapabilities` omits `with_icc/exif/xmp(true)`.
- **Med** — no `resolve_color_emit` / CICP→ICC synthesis.
- **Low** — decode puts bit_depth/channel_count on `ImageInfo`, not `SourceColor`.

**Plan:** carry `Metadata` into `TiffCodecEncoder` (kill the dead `_metadata`); write
ICC (tag 34675), EXIF (native sub-IFD via tag 34665 — re-parse the blob into entries,
**not** opaque bytes), XMP (700), orientation (274 in IFD0); add the missing caps;
wire `resolve_color_emit` (TIFF caps: cicp false → synthesize ICC). **Preserve the
embedded-EXIF-blob vs native-IFD distinction** (decode's `serialize_exif_ifd` is
correct; encode must write back as a native sub-IFD). TIFF has no CICP carrier.

### 7. zengif  — impl `src/codec.rs` (feature-gated)  — **already honest**
Caps correctly all-false for icc/exif/xmp/cicp (matches the `minimal` testkit
archetype). No `with_metadata` call sites anywhere. Decode fabricates no color
metadata. **Plan:** bump dep to 0.1.21; add a doc line that `resolve_color_emit` is
deliberately skipped (no carriers); optionally add a `zencodec-testkit`
`check_capability_honesty` test to lock it in. No behavioral change.

### 8. zenbitmaps  — impl `src/codec/*.rs` (feature-gated)  — **already honest**
All formats correctly declare no metadata/color caps; `with_metadata` is a no-op
(honest — these formats can't store it); no call sites; decode synthesizes only
*derivable* CICP (sRGB / BT.709-linear), which is honest, not fabricated.
- **Low** — one-shot `Decode::decode()` omits the CICP that `probe()` populates
  (two paths, two answers) — `mod.rs:214`.
- **Info** — BMPv5 ICC / v4 gamma parsed-and-discarded on decode (always reports sRGB).

**Plan:** bump dep; optionally thread CICP onto the one-shot decode output for parity;
doc note for BMPv5. No required behavioral change.

---

## Changes made per repo (filled as work lands)

> Each entry: provisional jj change id + commit, files touched, tests added, e2e
> result. Provisional = NOT pushed to the codec's `main` (gated on zencodec 0.1.21
> release). The dev patch lives in a gitignored `.cargo/config.toml`.

### zencodec (this repo)
- `docs/codec-integration-2026-06-08.md` — this report (the sweep tracker).

### zenjxl  ✅ main integration + jpeg_lossy color done (provisional, unpushed)
- **Change:** `f7be9506` (jj `vroqozzq`) on top of zenjxl `main` (38b7eadb) — **not pushed** (gated on zencodec 0.1.21 release).
- **Dev patch:** gitignored `.cargo/config.toml` → `paths = ["/home/lilith/work/zen/zencodec"]` (the `[patch.crates-io]`-in-config form does *not* apply here; the `paths` override does).
- **Files:** `src/codec.rs` (color-emit integration, already provisional from prior work; + 5 test migrations), `.gitignore` (+`.cargo/config.toml`).
- **Color:** `resolve_jxl_color` → `resolve_color_emit` + `EncodePolicy::resolve_color(Balanced)`; `JXL_ENCODE_CAPS` declares `cicp` + `cicp_is_valid_carrier` + `cicp_safe_sole_carrier(true)` (JXL is the only sole-safe carrier).
- **Metadata:** 5 test sites `with_metadata` → `with_metadata_policy(.., PreserveExact)` (verbatim semantics preserved); deprecation warnings now **0**.
- **E2E:** `cargo test --features zencodec` → **48 lib + 8 validate + doctests pass**; `metadata_cicp_round_trips_via_enum_color` + `icc_from_structured_color` green.
- **jpeg_lossy color — FIXED (2026-06-08, "edit siblings"):** investigation showed the **Coarsen** path already preserves the source ICC (jxl-encoder `encode_jpeg_to_jxl_with_effort` → `extract_icc`+`write_icc`); only the **Reencode** path (`convenience::encode_rgb8`, pixels-only) dropped it. Fixed entirely in zenjxl: extract the source APP2 ICC once (`zenjpeg::extract_icc_profile`) and embed it via the full `EncodeRequest`+`with_icc_profile` on the Reencode path; untagged JPEG stays sRGB (correct). Tests: `reencode_preserves_source_icc` (the fix), `coarsen_preserves_source_icc` (locks the existing behavior), `reencode_untagged_jpeg_stays_srgb`. No jxl-encoder edit needed.
- **moxcms misgating — FIXED (2026-06-08):** `extract_icc_profile`'s re-export was `#[cfg(feature = "moxcms")]` in zenjpeg despite doing no color management. Ungated it (`zenjpeg` `decoder/mod.rs`) — pushed to **zenjpeg main `6d59a2d6`** (standalone fix, independent of 0.1.21). zenjxl's `jpeg-lossy` feature now requires only `zenjpeg/decoder` (not `zenjpeg/moxcms`); `cargo tree` confirms moxcms is **dev-dep-only** (test fixture). Landing note: a *published* zenjxl jpeg-lossy needs a zenjpeg release carrying the ungate (it builds today via the path-dep) — or switch the call to `zenjpeg::color::icc::extract_icc_profile` (`color` is `pub`, so that path works against published zenjpeg without the ungate).

### zenjpeg  ✅ (provisional `b5df9c72`, unpushed)
- **Change:** `b5df9c72` (jj `nwkkptnw`) on `@` over main `9fe32816` — **not pushed**. Files: `zenjpeg/zenjpeg/src/codec.rs` (+326), `zenjpeg/zenjpeg/Cargo.toml` (+19), `Cargo.lock` (+1).
- **HIGH fix — decode double-rotation:** new `reconcile_baked_orientation` rewrites the embedded EXIF orientation tag to Identity (via `helpers::set_exif_orientation`) when the decoder bakes upright and reports `Identity`; applied in `Decode::decode` + `streaming_decoder` (OutputInfo paths carry no EXIF blob → untouched). Tests: `decode_auto_orient_rewrites_exif_tag_no_double_rotation`, `decode_preserve_orientation_keeps_exif_tag`.
- **Color-emit:** `PixelDescriptor` threaded through all 6 encode entry points into `build_request_from`; builds `SourceColor`, calls `resolve_color_emit` (JPEG caps, no CICP carrier), applies ICC disposition (KeepSource / SynthesizeFrom via `zenpixels_convert::icc_profiles::icc_profile_for_primaries` / Drop) — fixes ignored `meta.cicp` + always-embedded redundant sRGB ICC. Tests: `encode_color_emit_{synthesizes_icc_from_nonsrgb_cicp, drops_redundant_srgb_cicp, keeps_source_icc}`.
- **Caps:** `JPEG_ENCODE_CAPS` explicit `.with_cicp(false).with_cicp_is_valid_carrier(false).with_cicp_safe_sole_carrier(false)`. **Metadata:** 2 test sites → `with_metadata_policy(.., PreserveExact)`; `#[allow(deprecated)]` on the kept `with_metadata` impl. **Decode SourceColor:** `source_color_from_header` now sets channel_count + bit_depth.
- **Dep:** added `zenpixels-convert` (optional, tied to `zencodec` feature) — needed for ICC synthesis; lands with the work.
- **E2E:** `cargo test -p zenjpeg --features zencodec` → ~2103 passed, **0 failed**.
- **Scope note (review):** also added `required-features = ["target-zq"]` to two crate-`cfg`'d examples (`zq_calibrate`, `zq_pareto_calibrate`) that fail `cargo test` on main without it — a minimal pre-existing-build-bug fix, separable from the integration.
- **API note:** real signature is `helpers::set_exif_orientation(&[u8], Orientation) -> Option<Vec<u8>>`.

### zenavif  ✅ (provisional `9c71eff0`, unpushed) — Known-Issue #2 fixed
- **Change:** `9c71eff0` (jj `owoouqzx`) over main `46fc4bf7` — **not pushed**. Files: `src/codec.rs` (+294), `Cargo.toml` (+14), `Cargo.lock`.
- **HIGH fix — `apply_descriptor_color` CICP override (zencodec Known-Issue #2):** `AvifEncoder` gained a `caller_cicp` field; `apply_descriptor_color` now fills a primaries/transfer axis from the descriptor ONLY when the caller's `Metadata.cicp` left it unspecified (H.273 sentinels 0/2), and writes a coherent matrix. Caller CICP now wins. Tests: `caller_cicp_wins_over_descriptor_color` (P3 metadata over sRGB descriptor → nclx primaries=12, the regression), `descriptor_drives_cicp_without_caller_cicp` (fallback).
- **Color-emit:** `resolve_avif_color` builds `SourceColor`, `resolve_color_emit` (AVIF caps), lowers `plan.cicp`→nclx + ICC KeepSource/SynthesizeFrom/Drop; wired in `encoder()` + `animation_frame_encoder()`.
- **Caps:** `.with_cicp_is_valid_carrier(true).with_cicp_safe_sole_carrier(false)`; `#[allow(deprecated)]` on `with_metadata`.
- **E2E:** `cargo test --features zencodec,encode` → 83 lib + all integration suites green, **0 failed**.
- **Caveats (review):** (1) emitted nclx `matrix_coefficients` = BT.601 (6), honest for zenravif's RGB→YCbCr path (not the CICP's Identity). (2) **⚠ MUST-NOT-LAND:** the change relaxes `zenanalyze`/`zenpredict` path-dep version reqs (0.2.1/0.1.0 → 0.2.0, clearly commented "worktree-only") because the local siblings drifted — an environmental workaround, revert before landing. (3) a pre-existing stray `cargo fmt` edit in `src/yuv_convert.rs` (forgotten `zenavif--emit-integ` workspace) left untouched. `zenavif-parse` untouched.
- **Action:** zenavif owner can clear Known-Issue #2 from zencodec CLAUDE.md once this lands.

### zenpng  ✅ (provisional `e7bd4e66`, unpushed)
- **Change:** `e7bd4e66` (jj `lvqvvvzz`) over main `6156d550` — **not pushed**. Files: `src/codec.rs` (+202), `src/encode.rs` (+10, `ColorType::channels()`). No Cargo.toml change (zenpixels-convert already available).
- **Caps:** `PNG_ENCODE_CAPS` += `.with_cicp_is_valid_carrier(true).with_cicp_safe_sole_carrier(false)` (cICP is a valid, non-sole-safe carrier — was being silently dropped by `resolve_color_emit`).
- **Color-emit:** `apply_encode_policy` now builds `SourceColor` (channel count threaded through 6 sites), calls `resolve_color_emit`, lowers the plan onto `PngWriteMetadata` (cICP + iCCP KeepSource/SynthesizeFrom/Drop). Under Balanced a redundant sRGB iCCP drops while cICP stays.
- **Decode orientation:** `convert_info` parses the eXIf orientation tag (`helpers::parse_exif_orientation`) and sets `ImageInfo.orientation` (PNG never bakes → reports the stored tag). Tests: `decode_reports_exif_orientation`, `decode_without_exif_reports_identity_orientation`.
- **Metadata:** 1 test site → `with_metadata_policy(.., PreserveExact)`.
- **E2E:** `cargo test --features zencodec` → 664 passed, **0 failed**. Nothing deferred.

### zenwebp  ✅ (provisional `3a981325`, unpushed)
- **Change:** `3a981325` (jj `msknvkwv`) over main `9477d448` — **not pushed**. Files: `src/codec.rs` (+233), `Cargo.toml` (+8).
- **Color-emit + ICC synthesis (the key fix):** `WebpEncodeJob`/`WebpEncoder` gained internal `cicp`/`color_policy`; `do_encode` builds `SourceColor`, `resolve_color_emit` (WebP caps, no CICP carrier), lowers ICC: KeepSource / `SynthesizeFrom`→synthesize an ICC for CICP-only sources (previously silently dropped → untagged sRGB) / Drop. Test: `cicp_only_source_synthesizes_icc` + 3 guards (`srgb→no ICC`, `KeepSource`, decode authority).
- **Metadata path:** removed the bespoke all-or-nothing `EncodePolicy.resolve_icc/exif/xmp` gating; retention now flows through the provided `with_metadata_policy`→`Metadata::filtered` (enables sub-field EXIF web-filtering). `#[allow(deprecated)]` on the kept `with_metadata`.
- **Caps:** explicit `.with_cicp(false)`. **Decode:** `to_image_info` sets `color_authority = Icc` when ICCP present.
- **Dep:** `zenpixels-convert` promoted dev-dep → optional dep (tied to `zencodec`), `default-features = false` (wasm-clean). No public-API change.
- **E2E:** `cargo test --features zencodec` → 647 passed, **0 failed** (+ wasm32 build + clippy clean). Nothing deferred.

### zentiff  ✅ (provisional `58e69cb1`, unpushed) — HIGH metadata-drop fixed + proper EXIF decomposition
- **Change:** `58e69cb1` (jj `spmyztpy`) over `zenextras` main `ce54a751` — **not pushed**. Files: `zentiff/src/codec.rs`, `src/encode.rs`, `src/decode.rs`, `tests/zencodec_integration.rs`, `Cargo.toml`, `CHANGELOG.md`; `zenextras/Cargo.lock`.
- **HIGH fix — metadata was dropped entirely:** the old `with_metadata` stored into a dead `_metadata` field and `encoder()` ignored it → encode wrote NO ICC/EXIF/XMP/orientation (decode→encode silently stripped everything). Now carries `Metadata`+`EncodePolicy` into `TiffCodecEncoder` and writes ICC (tag 34675), XMP (700), orientation (274, IFD0).
- **EXIF decomposition (2026-06-08 revision — "integrate, don't blob-embed"):** the first pass re-emitted only the blob's IFD0 as one sub-IFD (correct for TIFF round-trip, but for a *foreign* JPEG/WebP/PNG blob it dropped the real EXIF tags behind the 0x8769 pointer + GPS behind 0x8825, and misrouted IFD0 camera tags). Replaced with a full IFD-tree walker (`decompose_exif_blob`/`follow_ifd_pointer`/`parse_ifd_at`) that follows 0x8769→**EXIF sub-IFD (34665)** and 0x8825→**GPS sub-IFD (34853)** and routes IFD0 descriptive tags → output **IFD0**. Decode side (`read_exif_bytes`) folds IFD0 descriptive tags back in so round-trip stays faithful. Tests: `foreign_exif_blob_decomposes_into_correct_native_ifds` (byte-walks the output to prove each tag lands in the correct IFD), `metadata_web_policy_roundtrip_keeps_icc_orientation_strips_gps`.
- **Caps:** Encode += `.with_icc/exif/xmp(true)`; Decode += `.with_multi_image(true)`.
- **Color-emit:** `lower_metadata` → `resolve_color_emit` (TIFF caps, no CICP carrier); CICP-only source synthesizes an ICC. Tests: `cicp_only_source_synthesizes_icc_on_encode`, `srgb_cicp_only_source_embeds_no_icc`, `metadata_default_no_metadata_is_byte_identical`.
- **Dep:** added `zenpixels-convert` (optional, `zencodec` feature); bumped `zenpixels` min → 0.2.11.
- **E2E:** `cargo test --features zencodec` → 68 passed, **0 failed**.
- **DEFERRED (review):** EXIF tags of TIFF type UNDEFINED (7) — ExifVersion/UserComment/MakerNote — are re-emitted as type BYTE (1) because `image-tiff`'s public `write_tag` API has no raw-undefined-bytes constructor. Value bytes identical, only the type code differs; nothing malformed, most readers accept either. Exact type preservation needs an upstream image-tiff API. **`Cargo.lock` also carries pre-existing zenpdf/zenpixels bumps made by another process — left intact (not reverted).**

### zengif  ✅ verified — already honest, no code change
- Audit confirmed GIF correctly declares all metadata/color caps false (palette-only), has zero `with_metadata` call sites, and fabricates no color metadata on decode. **No code change needed.**
- **E2E:** `cargo test --features zencodec` → green vs patched 0.1.21 (19 + 15 + … passed, **0 failed**) — confirms the new API doesn't break it.
- **Landing:** bump the dep floor to 0.1.21 when it publishes (optional — `^0.1.19` already resolves it). Optional polish (not done): a doc note that `resolve_color_emit` is deliberately skipped, and a `zencodec-testkit` `check_capability_honesty` test.

### zenbitmaps  ✅ verified — already honest, no code change
- Audit confirmed PNM/PFM/farbfeld/QOI/HDR/TGA/BMP correctly declare no metadata/color caps; `with_metadata` is an honest no-op; no call sites; decode synthesizes only *derivable* CICP (sRGB / BT.709-linear). **No code change needed.**
- **E2E:** `cargo test --features zencodec,bmp,qoi,hdr,tga` → green vs patched 0.1.21 (1+37+13+100+46+5 passed, **0 failed**).
- **Low/Info (optional, not done):** one-shot `Decode::decode()` omits the CICP that `probe()` populates (parity); BMPv5 ICC parsed-and-discarded on decode.
- **Landing:** bump the dep floor to 0.1.21 when it publishes (optional).

---

## Landing checklist (when zencodec 0.1.21 publishes)

Nothing here is on any codec's `main` yet. To land, **per codec** (gated on the
0.1.21 release + the usual release ceremony):

1. **Publish zencodec 0.1.21** (full ceremony: CI green all platforms → tag →
   GitHub release → `cargo publish`; user-approved). This unblocks everything.
2. **Remove the dev bridge:** revert `/home/lilith/work/zen/.cargo/config.toml`
   to just the wasm `runner` line.
3. For each provisional codec change (`b5df9c72` zenjpeg, `9c71eff0` zenavif,
   `e7bd4e66` zenpng, `3a981325` zenwebp, `b3c87c0c` zentiff, `8a618923` zenjxl):
   bump the codec's `zencodec` dep floor to `0.1.21`, then push to its `main`.
4. **gif / bitmaps:** optional dep-floor bump to `0.1.21`; no code change.

### Must-not-land / review-before-landing
- **zenavif** — revert the `zenanalyze`/`zenpredict` path-dep version relaxations
  (0.2.0 → back to 0.2.1/0.1.0) once the local siblings are back on their
  published versions. These are an environmental worktree-only workaround.
- **zenjpeg** — the two `required-features = ["target-zq"]` example fixes are a
  pre-existing build-bug fix; can land as-is or be split into their own commit.
- **zentiff** — `Cargo.lock` carries pre-existing zenpdf/zenpixels bumps made by
  another process; confirm those are intended before pushing the lockfile.

### Done since first report (2026-06-08, "do both, edit siblings" + "fix the misgating")
- **zenjxl `jpeg_lossy.rs` color — FIXED** (`f7be9506`). Coarsen already preserved
  ICC; the Reencode path now embeds the source ICC via `EncodeRequest`. No
  jxl-encoder edit was needed (the deferral over-estimated the scope).
- **zentiff EXIF — now properly decomposed** (`58e69cb1`): IFD0/EXIF(34665)/GPS(34853)
  routing instead of the round-trip-only blob re-emit. Correct for foreign blobs.
- **zenjpeg `extract_icc_profile` moxcms misgating — FIXED + PUSHED** to zenjpeg
  main (`6d59a2d6`). Standalone (no 0.1.21 dependency); the only thing landed on a
  codec's `main` in this whole sweep. zenjxl's `jpeg-lossy` dropped the
  `zenjpeg/moxcms` pull (moxcms now dev-dep-only).

### Deferred / review-before-landing (reported, not implemented)
- **zentiff EXIF UNDEFINED→BYTE** type-code downgrade (ExifVersion/UserComment/
  MakerNote) — `tiff 0.11.3`'s `write_tag` has no raw-undefined constructor;
  value bytes are identical, only the type code differs. Faithful types need an
  upstream `image-tiff`/`tiff` API + a patch (zentiff uses crates.io `tiff`).

### Verification record (all green vs patched 0.1.21, 0 failures)
zenjxl 48+8 · zenjpeg ~2103 · zenavif 83+integration · zenpng 664 · zenwebp 647 ·
zentiff 68 · zengif 19+15 · zenbitmaps 202. Logs in `/tmp/<codec>-integ-test.log`.

---

## 2026-06-09 — transfer-aware ICC synthesis: `synthesize_icc_for_cicp` switch

Every codec's `SynthesizeFrom(cicp)` lowering used
`zenpixels_convert::icc_profiles::icc_profile_for_primaries(cicp.color_primaries_enum())`
— **primaries-only**, so it would hand a BT.2020-**PQ** source the SDR-TRC
Rec.2020 profile (a silently mis-tagged transfer). Replaced with the new
transfer-aware `synthesize_icc_for_cicp(cicp) -> SynthesizedIcc`.

### zenpixels-convert — `synthesize_icc_for_cicp` (routed as a PR, not direct to main)
- **PR: imazen/zenpixels#37** (branch `feat/icc-profile-for-cicp`), assigned
  lilith. CI fully green (Clippy, Feature powerset, Format, MSRV, every Test
  platform incl windows-11-arm / i686 / macOS-Intel, WASM, Coverage).
- Adds `icc_profiles::synthesize_icc_for_cicp(Cicp) -> SynthesizedIcc` + the
  `#[non_exhaustive]` `SynthesizedIcc { Profile(Cow<'static,[u8]>), NotNeeded,
  NeedsCms, CmsUnsupported }`. Promotes `icc_profile_for` to `pub`; documents the
  HDR mis-tag hazard on `icc_profile_for_primaries`.
- **No-mis-tag guarantee:** moxcms's `try_from::<u8>` never errors (reserved codes
  fold into `Reserved`) and `new_from_cicp` discards its validity bool, silently
  returning a TRC-less base profile for Reserved/Unspecified codes. The cms path
  therefore gates on a populated `red_trc` (set only after every primaries +
  white-point + transfer-curve gate passes) → unrepresentable CICP yields
  `CmsUnsupported`, never a degenerate/mis-tagged profile.
- 4 unit tests covering all `SynthesizedIcc` outcomes, cfg-split for cms-moxcms.
- **Separate commit on zenpixels main (`dfd0de1`):** `cargo fmt` of 4 gamut_clip
  files — main's Format CI had been red since 2026-06-01. The PR branch sits on
  top so its own Format job passes.

### zencodec — `IccDisposition::SynthesizeFrom` doc (landed on main `33295a8`)
Module "Lowering the plan" bullet + the variant doc now point at
`synthesize_icc_for_cicp` and spell out the best-effort contract (bundled coverage
Display-P3 + SDR BT.2020 vs cms-moxcms PQ/HLG; any non-`Profile` outcome embeds no
ICC and lets the CICP carrier convey color — never fabricate or mis-tag).

### Codec switches (PROVISIONAL — folded into each codec's existing 0.1.21 commit)
All sites switched `icc_profile_for_primaries(…color_primaries_enum())` →
`match synthesize_icc_for_cicp(cicp) { Profile(b) => embed b, _ => no ICC }`:

| Codec | Commit | Site shape | e2e synth test |
|---|---|---|---|
| zenwebp | `0109d6c4` | `do_encode`; hoisted `synth_holder` (`ImageMetadata` borrows the bytes, must outlive `req.encode()`) | `cicp_only_source_synthesizes_icc` ✓ |
| zenpng | `6262ea07` | `apply_encode_policy`; owned `Arc<[u8]>` | **added** `cicp_only_display_p3_synthesizes_icc` (encode→decode roundtrip) ✓ |
| zenavif | `80ea8500` | color-resolve fn; owned `Arc<[u8]>`. nclx is sole-safe → synth is non-default (fires under Compatibility) | all color/cicp tests ✓ (no dedicated synth test — non-default path) |
| zentiff | `0824b1f0` | `synth_icc_from_cicp` helper; owned `Vec<u8>`. TIFF has no CICP carrier → ICC is the *only* wide-gamut path | `cicp_only_source_synthesizes_icc_on_encode`, `srgb_cicp_only_source_embeds_no_icc` ✓ |

- **zenjpeg: out of scope** — no CICP→ICC synth call site exists.
- The color path is `#[cfg(feature = "zencodec")]` in every codec, so default
  builds don't compile it. Built+tested each with the feature on
  (`--features zencodec`; `encode,zencodec` for zenavif; `-p zentiff --features
  zencodec`).

### Dev bridge broadened
`/home/lilith/work/zen/.cargo/config.toml` `paths` now also overrides
`zenpixels-convert` → local (0.2.11, with `synthesize_icc_for_cicp`). **TEMPORARY.**
Forced an `archmage`/`magetypes` `0.9.23 → 0.9.26` Cargo.lock bump in
zenwebp/zenpng/zenavif (local zenpixels-convert requires `^0.9.26`; the codecs
allow `^0.9.15` so it's compatible and pre-aligns for the eventual publish).
zentiff/zenextras needed no bump.

### Landing checklist addendum (additionally gated on a zenpixels-convert publish)
1. Merge **zenpixels#37**, then publish a zenpixels-convert release carrying
   `synthesize_icc_for_cicp` (≥ 0.2.12, full ceremony, user-approved).
2. Bump each codec's `zenpixels-convert` dep floor to that version alongside the
   `zencodec` 0.1.21 floor bump (step 3 of the original checklist), then push to
   its `main`.
3. Remove **both** `paths` entries from the dev `config.toml` once both deps
   (zencodec 0.1.21 + zenpixels-convert ≥ 0.2.12) are published.
