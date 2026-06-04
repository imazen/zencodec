# Near-lossless / lossless-mode: a generic cross-codec abstraction

Status: design proposal for [zencodec#12](https://github.com/imazen/zencodec/issues/12).
Date: 2026-06-03. Scope: the `EncoderConfig` fidelity surface.

This document maps how every zen codec actually treats lossless and
near-lossless, identifies why the naive three-state enum in #12 is not
expressive enough, and proposes a generic abstraction that fits all of them
without lying to the caller.

---

## 1. The request (#12)

> `with_lossless(bool)` can't express near-lossless modes that WebP, JXL, and
> PNG support.
> - WebP: `with_near_lossless(0-100)` pre-rounds pixels in the VP8L lossless path
> - JXL: distance 0.0-1.0 "perceptually lossless zone" (distinct from Modular lossless)
> - PNG: `with_near_lossless_bits(1-4)` rounds LSBs before DEFLATE
> - AVIF, JPEG, GIF have no near-lossless mode
>
> Proposal: add `enum LosslessMode { Lossy, NearLossless, Lossless }` to
> `EncoderConfig`; codecs without near-lossless treat `NearLossless` as
> high-quality lossy; deprecate `with_lossless(bool)`.

The instinct is right — `bool` is too coarse — but the survey below shows the
three bullet points are **not the same kind of thing**, and a parameterless
`NearLossless` variant throws away the one number (the error budget) that makes
near-lossless a *contract* instead of a vibe.

---

## 2. What each codec actually does (verified against source)

| Codec | True lossless? | "Near-lossless" mechanism | Native parameter | Error semantics |
|-------|----------------|---------------------------|------------------|-----------------|
| **WebP** (`zenwebp`) | yes (VP8L) | **adaptive pre-quantization on the lossless path** | `near_lossless: u8`, 0–100, **100 = off** | Guaranteed max per-channel error ∈ {0,1,3,7,15,31}; only non-smooth pixels touched; image borders never modified; multi-pass refinement. Requires lossless mode. |
| **PNG** (`zenpng`) | yes (always) | **global LSB rounding** before filter + DEFLATE | `near_lossless_bits: u8`, 0–4 | Round every channel to nearest multiple of 2^b → max error 2^(b−1). Uniform, every pixel. |
| **JXL** (`jxl-encoder`/`zenjxl`) | yes (modular, distance 0) | (a) `lossy_palette: bool` in modular; (b) *small butteraugli distance* = "visually lossless" — but that is a **lossy** codestream | `with_lossy_palette(bool)`; distance `-d` | Palette: error-diffused quantization, no clean ceiling. Distance: perception-bounded, **not** a per-channel ceiling, and not a lossless codestream. No `max_delta_error` knob is exposed (libjxl has it internally, unserialized). |
| **AVIF** (`zenavif`/`zenrav1e`) | yes (qindex 0) | **none** — only true-lossless or low-QP lossy | `with_lossless(bool)` + quality | No dedicated near-lossless preprocessing. |
| **JPEG** (`zenjpeg`) | **no** | none | quality only | Baseline only; q100 is still lossy (quantization > 0). |
| **GIF** (`zengif`) | yes (≤256 colors) | none for pixels; `lossy_tolerance` is **animation frame-diff** tolerance, not a pixel near-lossless mode | `lossy_tolerance: u8` | Palette reduction is the lossy step; LZW of indices is lossless. |

File references for the live APIs:
`zenwebp/src/encoder/vp8l/near_lossless.rs` + `src/codec.rs:144`;
`zenpng/src/optimize.rs:537` (`near_lossless_quantize`) + `src/codec.rs:115`;
`jxl-encoder/.../api.rs:1191` (`with_lossy_palette`);
`zenavif/src/codec.rs:430` (`with_lossless`);
`zengif/src/codec.rs:399` (`with_lossless` → `lossy_tolerance=0`).

---

## 3. The key insight: three axes are being conflated

`LosslessMode { Lossy, NearLossless, Lossless }` collapses **three independent
properties** into one enum:

1. **Coding mode** — is the *codestream* produced by a lossless coder or a lossy
   coder? This is the fundamental fork. PNG/GIF are structurally lossless;
   JPEG is structurally lossy; WebP/JXL/AVIF support both.

2. **Near-lossless = bounded pre-quantization on a lossless coding path.** A
   lossless coder applied to *deliberately, boundedly degraded* pixels. This is
   the **only** thing that is technically "near-lossless." Its natural,
   codec-independent currency is a **maximum per-channel error budget ε** (in
   sample LSBs). WebP and PNG implement exactly this. JXL's `lossy_palette` is a
   cousin (bounded, but error-diffused — no clean ε ceiling).

3. **"Visually lossless" = the top of the lossy quality scale.** JXL d ∈ [0.1,
   1.0], AVIF very-high-quality, JPEG q95+, WebP-lossy q95+. This is **not a
   separate mode** — it is `with_generic_quality()` near 100 (or a small
   distance). It already has a knob.

The defect in the naive enum is that it **merges axis 2 and axis 3.** The #12
bullet lists WebP/PNG (axis 2: an *ε ceiling on pixels*, lossless codestream)
alongside JXL distance 0.0–1.0 (axis 3: a *perception bound*, lossy codestream)
as if they were one mode. They are different contracts:

| | Axis 2 — near-lossless | Axis 3 — visually lossless |
|---|---|---|
| Guarantee | "no channel deviates by more than ε" | "no human can tell" |
| Parameter | ε in LSBs | quality / butteraugli distance |
| Codestream | **lossless** coder | **lossy** coder |
| Reproducible/exact-ish | yes, bit-bounded | no, perceptual |
| Who has it | WebP, PNG | JXL, AVIF, JPEG, WebP-lossy |

A generic abstraction must keep these separate, or it will mis-map JXL (its
"perceptually lossless zone" is reachable with the **existing quality knob**,
not a near-lossless mode) and over-promise on AVIF/JPEG.

---

## 4. Proposed abstraction

Two pieces, both small, both back-compatible.

### 4.1 `LosslessMode` — the coding-mode selector (carries the budget)

```rust
/// How faithfully the encoder reproduces the input.
///
/// This is the *coding-mode* axis. The "visually lossless" zone (a small
/// butteraugli distance / very high quality) is **not** here — it is the top
/// of the lossy quality scale, reachable with
/// [`with_generic_quality`](EncoderConfig::with_generic_quality).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LosslessMode {
    /// Lossy codestream. Fidelity is governed by quality / effort / distance.
    Lossy,

    /// Lossless codestream of pixels that were pre-quantized within a bounded
    /// per-channel error. The codec rounds pixel samples so the lossless coder
    /// compresses better, while guaranteeing the deviation ceiling below.
    ///
    /// `max_channel_error` is the **guaranteed ceiling**, in sample LSBs at the
    /// encoded integer bit depth (e.g. 0–255 for 8-bit, 0–65535 for 16-bit).
    /// `0` is equivalent to [`Lossless`](Self::Lossless). A codec must **never
    /// exceed** this ceiling: it rounds the budget *down* to the nearest level
    /// it can honor, never up. See [`EncoderConfig::lossless_mode`] for what was
    /// actually resolved.
    NearLossless { max_channel_error: u16 },

    /// Mathematically exact. Decoding reproduces the input sample-for-sample.
    Lossless,
}

impl LosslessMode {
    /// A sensible default near-lossless budget (ε = 2 LSB at 8-bit): visually
    /// transparent on photographic content, meaningfully smaller files. Use
    /// when you want "near-lossless" without choosing a number.
    pub const NEAR_LOSSLESS_DEFAULT: Self = Self::NearLossless { max_channel_error: 2 };
}
```

**Why ε (max per-channel error), not "bits" or "0–100":**

- It is physically meaningful and codec-independent: the maximum absolute
  deviation of *any* channel sample. A caller can reason about it without
  knowing the codec.
- It is a **contract**, not an opaque dial. "bits" (PNG) and "0–100" (WebP) are
  each codec's *private encoding* of the same ceiling.
- WebP and PNG already describe their effect as a max-error ceiling; ε is simply
  their common denominator. (WebP: `(1<<bits)-1`; PNG: `2^(bits-1)`.)

### 4.2 `EncoderConfig` additions

```rust
pub trait EncoderConfig: Clone + Send + Sync {
    // ... existing ...

    /// Set the coding-mode / fidelity for this encode.
    ///
    /// Default is a no-op (returns `self`). Codecs that support a fidelity
    /// choice override this. After calling, read [`lossless_mode`] to see what
    /// the codec actually resolved (it may promote or demote — see below).
    fn with_lossless_mode(self, _mode: LosslessMode) -> Self {
        self
    }

    /// The resolved coding mode, or `None` if the codec has no fidelity choice.
    ///
    /// Returns what the codec will *actually* do, which may differ from what was
    /// requested via [`with_lossless_mode`]:
    /// - **honored** — WebP/PNG return `NearLossless { ε' }` with `ε' <= ε`.
    /// - **promoted to `Lossless`** — a lossless-capable codec with no ε
    ///   mechanism (AVIF, GIF, JXL) returns exact `Lossless`. Fidelity is
    ///   *better* than asked; file is larger. Never worse than the contract.
    /// - **demoted to `Lossy`** — a codec with no lossless path (JPEG) returns
    ///   `Lossy`. This is the only case where the result is lossier than asked,
    ///   so it is observable here.
    ///
    /// Default forwards [`is_lossless`] for codecs that only know the bool axis.
    fn lossless_mode(&self) -> Option<LosslessMode> {
        self.is_lossless().map(|l| {
            if l { LosslessMode::Lossless } else { LosslessMode::Lossy }
        })
    }

    // `with_lossless(bool)` and `is_lossless()` stay (see §5). Default impls
    // now forward to the mode API so a codec only has to implement one side.
    fn with_lossless(self, lossless: bool) -> Self {
        self.with_lossless_mode(if lossless {
            LosslessMode::Lossless
        } else {
            LosslessMode::Lossy
        })
    }
    fn is_lossless(&self) -> Option<bool> {
        self.lossless_mode().map(|m| matches!(m, LosslessMode::Lossless))
    }
}
```

A codec implements **one** of the two pairs and gets the other for free. New
codecs implement `with_lossless_mode` + `lossless_mode`; existing codecs that
only implement `with_lossless` + `is_lossless` keep working unchanged (the
default `with_lossless_mode` is a no-op, so they simply ignore `NearLossless`
until they opt in — identical to today's behavior for any unknown setting).

### 4.3 `EncodeCapabilities` addition

```rust
// in struct EncodeCapabilities:
near_lossless: bool,          // honors an ε-bounded near-lossless path
near_lossless_min_error: u16, // finest non-zero ε it can actually honor (0 = n/a)

// const builder + getters mirroring the existing `with_lossless` / `lossless`:
pub const fn with_near_lossless(mut self, v: bool) -> Self { self.near_lossless = v; self }
pub const fn near_lossless(&self) -> bool { self.near_lossless }
```

`lossy` / `lossless` already exist; `near_lossless` slots in beside them so a
codec-agnostic pipeline can query support before requesting it.

### 4.4 `DynEncoderConfig` addition

```rust
fn set_lossless_mode(&mut self, mode: LosslessMode);
```

Blanket-implemented over `EncoderConfig` exactly like the existing `set_*`
forwarders in `traits/dyn_encoding.rs`.

---

## 5. Back-compat & the `with_lossless` deprecation question

#12 asks to deprecate `with_lossless(bool)`. **Recommendation: keep it, do not
deprecate.** Reasons:

- `bool` ↔ 3-state is lossy in only one direction (`bool` can't express
  `NearLossless`), and the proposal already adds `with_lossless_mode` for that.
  `with_lossless(true/false)` remains the correct, ergonomic call for the 90%
  case that just wants exact-vs-lossy.
- Deprecating a widely-used setter is churn (every codec crate + callers) for no
  expressive gain — the new method covers the gap additively.
- Wiring the defaults so each codec implements one side (§4.2) means there is no
  duplication to drift.

So: **additive only.** `with_lossless` / `is_lossless` keep their signatures and
semantics; `with_lossless_mode` / `lossless_mode` are the richer surface; nothing
is removed. This is a non-breaking minor release.

(If a future major release does want to collapse them, the migration is trivial
because `bool` is exactly the `{Lossy, Lossless}` subset.)

---

## 6. Per-codec mapping (the ε → native-parameter table)

ε is in 8-bit LSBs below. Codecs **round the guarantee down** — pick the largest
native level whose worst-case error does **not exceed** ε.

### WebP — honored
WebP's guaranteed max error is `(1<<bits)-1`, with `bits = 5 - quality/20`.
Invert ε to the *loosest* WebP level that still satisfies the ceiling:

| requested ε | WebP `near_lossless` | bits | actual max err | `lossless_mode()` returns |
|---|---|---|---|---|
| 0 | 100 (off) | 0 | 0 | `Lossless` |
| 1–2 | 80 | 1 | 1 | `NearLossless{1}` |
| 3–6 | 60 | 2 | 3 | `NearLossless{3}` |
| 7–14 | 40 | 3 | 7 | `NearLossless{7}` |
| 15–30 | 20 | 4 | 15 | `NearLossless{15}` |
| ≥31 | 0 | 5 | 31 | `NearLossless{31}` |

Requires the VP8L (lossless) path; `with_lossless_mode(NearLossless{..})`
implies lossless coding and sets `near_lossless` accordingly.

### PNG — honored
PNG rounds to nearest 2^b → max error `2^(b-1)`. Pick the largest `b ≤ 4` with
`2^(b-1) ≤ ε`:

| requested ε | PNG `near_lossless_bits` | actual max err | `lossless_mode()` returns |
|---|---|---|---|
| 0 | 0 | 0 | `Lossless` |
| 1 | 1 | 1 | `NearLossless{1}` |
| 2–3 | 2 | 2 | `NearLossless{2}` |
| 4–7 | 3 | 4 | `NearLossless{4}` |
| ≥8 | 4 | 8 | `NearLossless{8}` |

### JXL — promoted to `Lossless` (with a codec-specific escape hatch)
JXL has **no clean ε ceiling**. Its `lossy_palette` is error-diffused, so it
cannot promise "≤ ε per channel." The honest generic mapping is:
`capabilities.near_lossless = false`; `NearLossless{ε}` resolves to exact
`Lossless` (fidelity ≥ asked, never worse). `lossy_palette` stays a
**codec-specific extension** on `JxlEncoderConfig` (not wired to the generic ε),
because exposing it through ε would misreport its guarantee. JXL's "perceptually
lossless" use case is served by `with_generic_quality(~95–100)` / small distance
— axis 3, not this API.

### AVIF, GIF — promoted to `Lossless`
Lossless-capable, no ε mechanism. `NearLossless{ε}` → exact `Lossless`.

### JPEG — demoted to `Lossy`
No lossless path. `NearLossless{ε}` (and `Lossless`) → `Lossy` at a documented
high quality (≈ q95). This is the single case where the result is lossier than
the contract; it is observable via `lossless_mode()` returning `Lossy`.

### Summary of resolution policy

| Codec | `Lossy` | `NearLossless{ε>0}` | `Lossless` | `caps.near_lossless` |
|---|---|---|---|---|
| WebP | Lossy | **honored** (≤ ε) | Lossless | true |
| PNG | (indexed/lossy via quality) | **honored** (≤ ε) | Lossless | true |
| JXL | Lossy (VarDCT) | promote → Lossless | Lossless | false |
| AVIF | Lossy | promote → Lossless | Lossless | false |
| GIF | (palette) | promote → Lossless | Lossless | false |
| JPEG | Lossy | demote → Lossy | demote → Lossy | false |

The rule in one line: **honor if you can; otherwise promote to exact lossless
(fidelity-first) if you have a lossless path; demote to high-q lossy only if you
have no lossless path — and always report the truth via `lossless_mode()`.**

This refines #12's "treat NearLossless as high-quality lossy *for all* codecs
without near-lossless." For AVIF/GIF/JXL that would needlessly throw away
fidelity; promoting to exact lossless is the better default and keeps the "near"
in near-lossless. Only JPEG (no lossless path) actually has to demote.

---

## 7. Edge cases & scope

- **Bit depth.** ε is in LSBs at the encoded integer bit depth. A codec that
  encodes 8-bit interprets ε ∈ [0,255]; a 16-bit encode interprets ε ∈
  [0,65535]. Callers targeting a specific depth should set ε in that depth's
  units. (A future helper could accept a normalized fraction and scale, but the
  integer ceiling is the primitive.)
- **Float / HDR formats.** A per-channel integer LSB ceiling is undefined for
  `f32` pixels. For float formats `NearLossless` resolves to `Lossless` (or
  `Lossy` if no lossless path) and `near_lossless` capability is false.
- **Alpha.** ε applies per channel including alpha, matching WebP/PNG behavior
  (both quantize alpha alongside color). `with_alpha_quality` is orthogonal and
  unchanged.
- **`NearLossless{0}`** is exactly `Lossless`; codecs may normalize it to the
  `Lossless` variant in `lossless_mode()`.

---

## 8. Why not the alternatives

- **Parameterless `NearLossless`** (literal #12): throws away ε. Two callers
  asking for "near-lossless" get unpredictable, codec-defined error. Not a
  contract. (Kept as `NEAR_LOSSLESS_DEFAULT` for ergonomics, but the variant
  still carries the number.)
- **Expose each codec's native knob generically** (`bits`, `0–100`): leaks codec
  internals, doesn't compose, and the two scales are inverses of each other
  (WebP 100 = off, PNG 0 = off) — a trap.
- **Fold "visually lossless" into the enum** (a `VisuallyLossless` variant):
  re-merges axis 3 into axis 2. It's already `with_generic_quality(~98)`; a
  second path to the same lossy codestream is redundant and confuses "ε ceiling"
  with "perception bound."
- **A normalized [0,1] error fraction instead of LSBs**: more portable across
  depths but loses the exact integer ceiling WebP/PNG actually honor, and most
  near-lossless usage is 8-bit. The integer LSB is the primitive; a fraction can
  layer on top later.

---

## 9. Implementation checklist (when this lands — on clean `main`)

zencodec (this crate):
1. Add `LosslessMode` (in a new `src/fidelity.rs` or alongside the encode
   traits) + re-export at crate root.
2. Add `with_lossless_mode` / `lossless_mode` to `EncoderConfig` with the
   forwarding defaults in §4.2; redefine `with_lossless` / `is_lossless`
   defaults to forward (no signature change).
3. Add `near_lossless` (+ `near_lossless_min_error`) to `EncodeCapabilities`
   with const builder + getter.
4. Add `set_lossless_mode` to `DynEncoderConfig` + blanket impl.
5. Document in `docs/spec.md` (§ EncoderConfig) and README.
6. `cargo semver-checks` — this is additive, expect a **minor** bump.

Per codec (each in its own crate, its own commit):
7. WebP: implement `with_lossless_mode`/`lossless_mode`; map ε per §6; set
   `caps.near_lossless = true`.
8. PNG: same; map ε → bits per §6; `caps.near_lossless = true`.
9. AVIF, GIF, JXL: implement `lossless_mode` to promote `NearLossless`→
   `Lossless`; `caps.near_lossless = false`.
10. JPEG: implement `lossless_mode` to demote `NearLossless`/`Lossless`→`Lossy`.
11. Round-trip tests per codec asserting the resolved `lossless_mode()` and the
    actual decoded max-channel-error ≤ requested ε for WebP/PNG.

> Note: at the time of writing, `main` has a separate in-flight, already-pushed
> feature branch (`feat/metadata-policy`). This is a design doc only; the trait
> changes above should land after that branch reconciles, to avoid entangling
> two API changes in one minor.
