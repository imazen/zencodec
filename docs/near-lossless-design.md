# Fidelity: a generic encode-quality abstraction (lossy / near-lossless / lossless)

Status: implemented in this crate (types + `EncoderConfig` surface) for
[zencodec#12](https://github.com/imazen/zencodec/issues/12). Codec
implementations land separately in each codec crate.

#12 asked for `enum LosslessMode { Lossy, NearLossless, Lossless }`. After
walking the codecs and the trait idiom it became a single **`Fidelity` sum
type**, with the lossy arm an extensible **`LossyTarget`**, a codec-agnostic
**`NearLosslessBudget`**, and a rich **`FidelityMatch`** outcome. The types live
in `src/fidelity.rs` (full rustdoc there); this doc captures the *why* and the
per-codec mapping.

---

## 1. What each codec actually does (verified against source)

| Codec | True lossless? | Near-lossless mechanism | Native parameter | Error semantics |
|-------|----------------|-------------------------|------------------|-----------------|
| **WebP** (`zenwebp`) | yes (VP8L) | **adaptive pre-quant on the lossless path** | `near_lossless: u8`, 0–100, **100 = off** (`config.rs:707`) | max per-channel error ∈ {0,1,3,7,15,31}; non-smooth pixels only; borders preserved |
| **PNG** (`zenpng`) | yes (always) | **global LSB rounding** before filter+DEFLATE | `near_lossless_bits: u8`, 0–4 (`encode.rs:58`) | round to 2^b → max err 2^(b−1); every pixel; 8-bit only |
| **JXL** (`jxl-encoder`) | yes (modular, d=0) | (a) `lossy_palette` (error-diffused, no clean ceiling); (b) small distance = a *lossy* "visually lossless" zone | `with_lossy_palette(bool)`; distance | distance is a perception bound, not a per-channel ceiling |
| **AVIF** (`zenavif`) | yes (qindex 0) | **none** (low-QP lossy only) | `with_lossless(bool)` (`codec.rs:84`) | — |
| **JPEG** (`zenjpeg`) | **no** | none | quality only | q100 still lossy |
| **GIF** (`zengif`) | yes (≤256 colors) | none for pixels (`lossy_tolerance` is animation frame-diff) | — | palette reduction is the lossy step |

---

## 2. Three axes get conflated

A single "quality" or 3-state enum hides three independent properties:

1. **Coding mode** — lossy vs lossless *codestream*. The real fork.
2. **Near-lossless** — *bounded pre-quantization on a lossless path*. Currency: a
   **max per-channel L∞ error budget ε**. WebP and PNG implement exactly this.
3. **"Visually lossless"** — the *top of the lossy quality scale* (small distance
   / very high quality). Not a separate mode — a lossy target.

Near-lossless (axis 2, an L∞ ε ceiling on a lossless codestream) and
visually-lossless (axis 3, a perceptual target on a lossy codestream) are
different contracts and must stay distinct arms, not be merged.

The near-lossless metric is **L∞ per channel** (the worst single channel of the
worst pixel), **not** the mean OKLab ΔE / SSIM2 that `zenpng::QualityGate` and
`zenquant` use — those are soft, image-aggregate quality gates (a different
axis; mean ΔE 0.3 still allows individual pixels far off).

---

## 3. The types (see `src/fidelity.rs` for full rustdoc)

```rust
pub enum Fidelity {                       // #[non_exhaustive]
    Lossy(LossyTarget),
    NearLossless(NearLosslessBudget),
    Lossless,
}

pub enum LossyTarget {                    // #[non_exhaustive]
    Quality(f32),                         // 0–100, single-pass everywhere
    Distance(f32),                        // butteraugli; JXL single-pass, else iterative
    Metric { metric: QualityMetric, target: f32 }, // iterative convergence
    TargetBytes(u64), Bitrate(f32),       // iterative
}

pub enum QualityMetric { Ssimulacra2, Butteraugli, Dssim, Psnr } // #[non_exhaustive]

pub struct NearLosslessBudget(u16);       // max per-channel L∞ error, parts-per-65535
```

**Why a sum type, not a scalar.** A scalar where `100 == lossless` has the JPEG
footgun (`quality(100)` isn't lossless there) and a fragile float boundary. A sum
type keeps the regimes — and their different metrics — apart: `LossyTarget`
(perceptual) and `NearLosslessBudget` (L∞) are different quantities, which is
exactly why each is its own arm.

**Why `NearLosslessBudget` is parts-per-65535.** A codec-agnostic max-error
*fraction* (every value valid for every codec, resolved by rounding the
guarantee *down* at the codec's depth). `255 × 257 = 65535` makes 8- and 16-bit
both exact: `from_8bit_steps(2)` → `±2` at 8-bit, `±514` at 16-bit, no float
floor trap. Codecs only consume `budget.max_error_at_depth(depth)`.

**Why `LossyTarget` is non-exhaustive.** Its arms differ wildly in cost/support:
`Quality` is single-pass everywhere; `Metric`/`TargetBytes`/`Bitrate` need
iterative re-encoding only some codecs implement; `Distance` is single-pass on
JXL only. Ship `Quality` now, add the rest behind capability flags as
convergence machinery lands — without a breaking change.

---

## 4. The `EncoderConfig` surface

```rust
fn with_fidelity(self, f: Fidelity) -> Self;                 // infallible, best-effort, chainable
fn try_target_fidelity(&mut self, f: Fidelity) -> FidelityMatch; // fail-fast, rich outcome
fn resolved_target_fidelity(&self) -> Option<Fidelity>;     // what the codec resolved to
fn with_alpha_fidelity(self, a: Option<Fidelity>) -> Self;  // lossy color + lossless alpha
fn alpha_fidelity(&self) -> Option<Fidelity>;
```

Two setters, the Rust `x` / `try_x` convention:
- `with_fidelity` stays infallible and chainable (the common path; `Quality` is
  universally supported). Best-effort — verify via the getter.
- `try_target_fidelity` is the opt-in strict path, returning **`FidelityMatch`**:

```rust
pub enum FidelityMatch {                   // #[non_exhaustive]
    Supported,                  // honored exactly
    MetricTranslated(Fidelity), // a metric/distance target mapped to native scale
    TargetRaised(Fidelity),     // rounded up — fidelity ≥ request, contract holds
    TargetLowered(Fidelity),    // rounded down — still within the requested regime
    Lossless,                   // promoted to exact lossless
    Unsupported,                // not honorable even approximately
}
```

The rule: a codec may quietly give you *better* fidelity than asked
(`TargetRaised`, `Lossless`) but never silently *less* — a downgrade across the
lossy/lossless fence is `Unsupported`, not a silent substitution. `try_` is a
cheap up-front resolution (no encode); for iterative targets it confirms the
codec will *attempt* convergence, the *achieved* value is an encode output.

The legacy scalars stay as **derived sugar** — defaults bridge both ways, so a
codec implements either the legacy `with_generic_quality`/`with_lossless` pair
*or* `with_fidelity`/`resolved_target_fidelity` and gets the other for free.
Additive, non-breaking. (No `DynEncoderConfig` change: fidelity is set on the
concrete config before type-erasure, like quality/lossless today.)

Capabilities gain `near_lossless`, `supports_distance`, `supports_metric_target`,
`supports_size_target` so a pipeline can query before requesting.

---

## 5. Per-codec resolution

`resolved_target_fidelity()` / `try_target_fidelity()` report what the codec
**actually did** — honored, promoted, or demoted.

| Codec | `Lossy(target)` | `NearLossless(ε>0)` | `Lossless` | `caps.near_lossless` |
|---|---|---|---|---|
| WebP | native q / iter for non-Quality | **honored** ≤ ε → native level | Lossless | true |
| PNG | indexed/lossy via quality | **honored** ≤ ε → bits | Lossless | true |
| JXL | VarDCT distance | promote → Lossless¹ | Lossless | false |
| AVIF | native q | promote → Lossless | Lossless | false |
| GIF | palette | promote → Lossless | Lossless | false |
| JPEG | native q | demote → Lossy² | demote → Lossy² | false |

¹ JXL `lossy_palette` has no clean ε ceiling → stays a codec-specific knob, not
wired to the generic ε. ² JPEG has no lossless path; `Lossless`/`NearLossless`
report `FidelityMatch::Unsupported` (or, via the best-effort `with_fidelity`,
fall back to lossy — observable in the getter).

**ε → native (8-bit), rounding the guarantee down:** WebP ε∈{0,1,3,7,15,31} →
`near_lossless` {100,80,60,40,20,0}; PNG ε → `bits` where `2^(b-1) ≤ ε` (b≤4).

---

## 6. Where the field lives

`EncoderConfig` is a trait; zencodec exports only the **types**. The stored value
lives in each codec's own config struct — for WebP/PNG, **reuse the existing
near-lossless field** (`LosslessConfig.near_lossless: u8`,
`EncodeConfig.near_lossless_bits: u8`); don't add a parallel one. Map at the
trait-impl boundary, store once.

---

## 7. Status / next steps

- **This PR (zencodec):** the types + `EncoderConfig` methods + capability flags
  + tests. Default impls bridge to the legacy scalars so nothing breaks.
- **Follow-up (per codec crate):** WebP/PNG honor ε and set
  `caps.near_lossless = true`; AVIF/GIF/JXL promote `NearLossless`→`Lossless`;
  JPEG leaves the default. Round-trip tests assert the decoded
  max-channel-error ≤ requested ε for WebP/PNG. Iterative `LossyTarget` arms land
  with their capability flags and `FidelityMatch` reporting.
