# Gain-map re-compression fidelity — rate–distortion study (2026-06)

**Question.** When a codec embeds a gain map (UltraHDR / ISO 21496-1), what JPEG
quality should the gain-map image use? Are gain-map compression artifacts
"unexpectedly costly" to HDR fidelity?

**Answer.** Default gain-map JPEG quality = **q90** — the rate–distortion knee of
the *rendered* HDR. Cameras ship ~q94 (slightly conservative, near-optimal). Not
lossless (bytes above q95 are wasted), not aggressive-low (the render falls off
below ~q90 and the byte savings are already tiny). **Downsampling is the larger
size lever** (cameras use ¼-per-axis = 1/16 pixels). This is wired into the
`EncodeJob::with_gain_map_pixels` fidelity note.

Three metrics reconcile rather than contradict:
- **cvvdp (JND):** "would anyone notice?" → no (re-compression stays < 1 JOD even at
  5-stop headroom).
- **SSIM2 (continuous RD):** "what quality is optimal?" → **q90** render knee.
- **File size:** the lever is small — the gain map is **0.27–1.98 % of the file**,
  so q98→q90 saves ~0.84 % of total bytes.

---

## Primary result — SSIM2 rate–distortion (33 real Samsung UltraHDR images)

SSIMULACRA2 (continuous, 0–100, higher = better), in-process via
`zenmetrics_api::Metric` (Ssim2, CUDA). "render" = reconstructed HDR display-mapped
to SDR8 (`srgb_oetf(clamp(linear/boost, 0, 1))`, boost ≈ 4.92×) ref-vs-variant;
"gain-map plane" = SSIM2 of the pristine decoded gain map vs the JPEG-recompressed
one. Reference is the camera's *already-compressed* gain map → these are
**incremental** re-compression losses (hence q98 ≠ 100).

| q  | gm bytes | saved vs q98 | gm % file | **render SSIM2** | gain-map-plane SSIM2 | render SSIM2/KB |
|----|----------|--------------|-----------|------------------|----------------------|-----------------|
| 98 | 69 690   | —            | 1.98 %    | 95.35            | 95.94                | 0.020           |
| 95 | 53 124   | 23.8 %       | 1.50 %    | 95.02            | 95.15                | 0.032           |
| **90** | **40 297** | **42.2 %** | **1.14 %** | **94.60**     | 93.12                | **0.096**       |
| 80 | 25 618   | 63.2 %       | 0.72 %    | 93.20            | 86.03                | 0.167           |
| 75 | 21 697   | 68.9 %       | 0.61 %    | 92.54            | 83.62                | 0.397           |
| 50 | 14 391   | 79.3 %       | 0.41 %    | 89.64            | 76.48                | 0.549           |
| 30 | 11 924   | 82.9 %       | 0.34 %    | 88.29            | 72.81                | 0.897           |
| 20 | 9 398    | 86.5 %       | 0.27 %    | 86.02            | 66.57                | 0.897           |

**The knee.** Marginal render efficiency collapses across q90: q80→q90 earns
0.096 SSIM2/KB, q90→q95 only 0.032, q95→q98 0.020. Above q90 you buy ~13 KB per
0.4 SSIM2 (waste); below it each KB earns real fidelity and the render falls off
(95.4 → 86.0 by q20, a 9-point range). The gain-map **plane** keeps improving to a
q95 knee and degrades hard at low q (29-point range), but that degradation
**attenuates ~3× through reconstruction** (smooth, upsampled, applied only to
highlights) — which is exactly why cvvdp reads it as sub-JND.

---

## Supporting results

### Phase 0 — what cameras actually ship (34 Samsung UltraHDR, header probe)
¼-per-axis gain map (1/16 pixels, ~6 % area), **q94** (base ~q96, Δ ≈ +2), **1
channel** (grayscale luma), baseline. 100 % consistent. Dominant lever is
resolution, not quality.

### Phase 1 — cvvdp on real 2.3-stop UltraHDR
Re-compression near-free: full q20→q98 span = 0.135 JOD. Decisive control:
reconstructing with *no HDR boost at all* vs full HDR costs only ≤ 0.87 JOD, so
gain-map quality (a sub-effect) is bounded below that. Phase-1's fixed 1000-nit
display clipped highlights (a leniency the next phase fixed).

### Phase 2 — cvvdp on synthetic high headroom (pristine, full-res gain maps)
Display peak scaled to content (`peak_nits = 203·2^H`) so highlights render instead
of clipping. **Rig gate:** the no-HDR control grows cleanly with headroom (mean
deficit 1.68 → 2.37 → 2.96 → 3.49 JOD at H=2..5; sanity exactly 10.0) — proof the
rig sees the headroom. Re-compression cost **steepens ~4× with headroom but never
crosses 1 JOD** anywhere in the grid:

| q (cvvdp deficit) | H2 (812 nit) | H3 (1624) | H4 (3248) | H5 (6496) |
|---|---|---|---|---|
| q5  | 0.142 | 0.273 | 0.417 | **0.560** |
| q40 | 0.030 | 0.074 | 0.130 | 0.187 |
| q80 | 0.003 | 0.011 | 0.021 | 0.032 |

Worst single cell of 336 = 0.96 JOD (mountain-lake, H5, q5).

---

## Caveats
- **q98 ≠ 100** (95.4/95.9): the reference is the camera's already-compressed gain
  map; values are incremental re-compression losses relative to it, consistent
  across all phases.
- **Render absolute magnitudes are display-map-specific** (the `/boost` map crushes
  the SDR base dark and emphasises highlights). The **knee location (q90) is the
  robust takeaway**; the magnitudes depend on the display map.
- **33/34 images** — one non-Samsung file lacked extractable ISO 21496-1 metadata.
- Synthetic high-headroom content (phase 2) follows real image structure but is not
  real high-headroom capture (none ≥ ~2 stops was available locally).

## Provenance
- **Corpus:** `~/work/codec-corpus/imazen-26` Samsung Galaxy Z Fold 7 / S25 Ultra
  UltraHDR (GContainer / ISO 21496-1), selected by gain-map signature.
- **Harness:** `~/work/zen/gainmap-fidelity-study/` — bins `gmstudy` (phase 0),
  `gmsweep` (phase 1), `highroom` (phase 2), `ssim2rd` (phase 3). Path-patched dev
  crate (ultrahdr-rs, ultrahdr-core, zenjpeg, zenmetrics-api).
- **Raw data** (in the harness dir; `*-MANIFEST*.json` carry the source commits):
  `ssim2-rd-2026-06-26.tsv` (396 rows), `highroom-sweep-2026-06-25.tsv` (336),
  `highroom-control-2026-06-25.tsv` (28), `results-2026-06-25.tsv` (phase 0),
  `calibration-table-2026-06-25.tsv`, `diagnostic-battery-2026-06-25.tsv`.
  The small control/probe TSVs (results, highroom-control, calibration,
  diagnostic) are mirrored under `benchmarks/gainmap-fidelity/`; the two large
  per-row TSVs (ssim2-rd 59 KB, highroom-sweep 41 KB) stay in the harness dir
  (> 30 KB — kept out of git).
- **Scorers:** cvvdp-gpu + ssim2-gpu via `zenmetrics_api` (CUDA, RTX-class), the
  same paths the `zen-metrics` CLI trusts (pycvvdp-matched within 0.005 JOD).
- **Date:** 2026-06-25 / -26.

## Design implication
- `EncodeJob::with_gain_map_pixels` documents the **q90 RD knee** as the fidelity
  default (a smooth single-channel control signal; downsampling is the size lever).
- No headroom-aware gain-map *quality* logic is warranted — quality is a minor lever
  on both axes (sub-JND perceptually, ≤ ~2 % of file by bytes). The load-bearing
  levers are gain-map *presence/content* and *headroom*, not its JPEG quality.
