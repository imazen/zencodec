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

**Dev bridge (the "cargo patch to this folder"):** add a *gitignored*
`.cargo/config.toml` with
```toml
[patch.crates-io]
zencodec = { path = "/home/lilith/work/zen/zencodec" }
```
0.1.21 satisfies the existing `^0.1.x` requirement, so **no `Cargo.toml` version
bump is needed to build+test**. The patch lets us develop and e2e-test the
integration now, while keeping each codec's committed manifest clean.

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
| 1 | zenjxl | 0.1.19→.20 | provisional `@` already on merged API | finalize, jpeg_lossy color, test migrations | M (mostly done) | ⬜ in progress |
| 2 | zenjpeg | 0.1.20 | provisional emit-integ on OLD API | rewrite vs new API (descriptor threading blueprint exists) | M | ⬜ |
| 3 | zenavif | 0.1.19 | clean | **fix apply_descriptor_color CICP-override (High)** + color-emit wiring | M | ⬜ |
| 4 | zenpng | 0.1.19 | clean | caps flags + resolve_color_emit + decode orientation extraction | M | ⬜ |
| 5 | zenwebp | 0.1.13 | far behind | dep bump + color-emit (synthesize ICC) + drop bespoke gating | M | ⬜ |
| 6 | zentiff | 0.1.19 | clean | **metadata currently DROPPED entirely (High)** — wire real embedding | M–L | ⬜ |
| 7 | zengif | 0.1.19 | already honest | dep bump + doc notes + optional testkit | S | ⬜ |
| 8 | zenbitmaps | 0.1.20 | already honest | dep bump + optional polish | S | ⬜ |

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

### zenjxl
- _pending_

### zenjpeg
- _pending_

### zenavif
- _pending_

### zenpng
- _pending_

### zenwebp
- _pending_

### zentiff
- _pending_

### zengif
- _pending_

### zenbitmaps
- _pending_
