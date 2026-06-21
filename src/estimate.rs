//! Unified resource estimation for encode/decode operations.
//!
//! Predicts an operation's **peak memory**, **wall time**, and **CPU-core
//! scaling** from three expandable inputs:
//!
//! 1. [`ImageCharacteristics`] — the image (dimensions + pixel format today;
//!    content class, frame count, HDR tier are future additions).
//! 2. the codec **config** — the [`EncoderConfig`](crate::encode::EncoderConfig)
//!    / [`DecoderConfig`](crate::decode::DecoderConfig) itself (it carries
//!    effort / quality / lossless / speed / thread intent).
//! 3. [`ComputeEnvironment`] — the hardware and conditions of computing
//!    (available cores now; available RAM, SIMD tier, load are future
//!    additions).
//!
//! All four types are **sealed and growable**: fields are private and the
//! structs are `#[non_exhaustive]`, so new fields are additive — read through
//! the accessor methods, construct through the builders. Callers built today
//! keep compiling as the structs gain fields.
//!
//! The codec answers via
//! [`EncoderConfig::estimate_encode_resources`](crate::encode::EncoderConfig::estimate_encode_resources)
//! (and the decode counterpart), returning a [`ResourceEstimate`]. Codecs with
//! a calibrated `heuristics` model override the default; the rest get a
//! conservative content-blind fallback.
//!
//! Wall time does **not** scale as `1/cores`: each codec carries a
//! [`ThreadingInformation`] (measured Amdahl fraction + the thread count beyond
//! which there is no further speedup), and [`ResourceEstimate::at_cores`] folds
//! it in.

use zenpixels::PixelDescriptor;

/// The SIMD instruction tier a codec will dispatch to.
///
/// An optional hint on [`ComputeEnvironment`]: a wider/newer tier generally
/// means a faster encode/decode, so estimates can apply a per-tier time factor
/// (today they assume the calibration host's tier; the field carries the hint
/// for future tier-aware models). The variants mirror the `x86-64-vN`
/// microarchitecture levels and the archmage / magetypes token vocabulary, so
/// a caller that already detects a tier with archmage maps it trivially:
///
/// ```rust,ignore
/// use zencodec::estimate::{ComputeEnvironment, SimdTier};
/// // archmage tokens use the same x86-64-vN levels:
/// let tier = if archmage::X64V4Token::summon().is_some() { SimdTier::X86V4 }
///     else if archmage::X64V3Token::summon().is_some() { SimdTier::X86V3 }
///     else if archmage::X64V2Token::summon().is_some() { SimdTier::X86V2 }
///     else { SimdTier::X86V1 };
/// let env = ComputeEnvironment::new().with_cores(8).with_simd_tier(tier);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SimdTier {
    /// The SIMD tier is unknown — estimates use a conservative cross-tier
    /// **baseline average** (no assumption about the running hardware). Use
    /// [`CurrentHost`](SimdTier::CurrentHost) instead when estimating for the
    /// local machine.
    Unknown,
    /// The SIMD tier of the host actually running the estimate (≈ the
    /// calibration host's native tier). Use this when estimating for the local
    /// machine — distinct from [`Unknown`](SimdTier::Unknown), which assumes a
    /// cross-tier baseline average rather than the running hardware.
    CurrentHost,
    /// WebAssembly without SIMD128 (scalar wasm).
    Wasm,
    /// WebAssembly SIMD128.
    Wasm128,
    /// AArch64 / ARM NEON (archmage `NeonToken`).
    Neon,
    /// x86-64-v1 — SSE2 baseline.
    X86V1,
    /// x86-64-v2 — SSE4.2 (archmage `X64V2Token`).
    X86V2,
    /// x86-64-v3 — AVX2 + FMA (archmage `X64V3Token`).
    X86V3,
    /// x86-64-v4 — AVX-512 (archmage `X64V4Token`).
    X86V4,
}

/// Hardware + runtime conditions for a resource estimate.
///
/// Sealed and growable — construct via [`ComputeEnvironment::new`] and refine
/// with the `with_*` setters; read with the accessors. Carries the available
/// core count + an optional [`SimdTier`] today; new fields (RAM, load factor,
/// GPU) are additive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ComputeEnvironment {
    available_cores: usize,
    available_ram_bytes: Option<u64>,
    simd_tier: Option<SimdTier>,
}

impl ComputeEnvironment {
    /// A single-core environment with unknown RAM and unspecified SIMD tier
    /// (the conservative default).
    #[must_use]
    pub fn new() -> Self {
        Self {
            available_cores: 1,
            available_ram_bytes: None,
            simd_tier: None,
        }
    }

    /// Number of CPU cores available to the operation (clamped to ≥ 1). On
    /// `std` callers typically pass `std::thread::available_parallelism()`.
    #[must_use]
    pub fn with_cores(mut self, cores: usize) -> Self {
        self.available_cores = cores.max(1);
        self
    }

    /// Physical RAM available to the operation, for memory-ceiling decisions.
    #[must_use]
    pub fn with_available_ram_bytes(mut self, bytes: u64) -> Self {
        self.available_ram_bytes = Some(bytes);
        self
    }

    /// The SIMD instruction tier the codec will dispatch to (e.g. detected via
    /// archmage on the caller's side). Estimates may use it to apply a
    /// per-tier time factor.
    #[must_use]
    pub fn with_simd_tier(mut self, tier: SimdTier) -> Self {
        self.simd_tier = Some(tier);
        self
    }

    /// Available CPU cores (≥ 1).
    #[must_use]
    pub fn cores(&self) -> usize {
        self.available_cores
    }

    /// Available RAM in bytes, if known.
    #[must_use]
    pub fn available_ram_bytes(&self) -> Option<u64> {
        self.available_ram_bytes
    }

    /// The SIMD tier hint, if specified.
    #[must_use]
    pub fn simd_tier(&self) -> Option<SimdTier> {
        self.simd_tier
    }
}

impl Default for ComputeEnvironment {
    fn default() -> Self {
        Self::new()
    }
}

/// Characteristics of the image being encoded/decoded.
///
/// Sealed and growable. Carries the dimensions and pixel format today; future
/// fields (content class, animation frame count overrides, HDR tier depth) are
/// additive.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ImageCharacteristics {
    width: u32,
    height: u32,
    descriptor: PixelDescriptor,
    frame_count: u32,
}

impl ImageCharacteristics {
    /// A still image of `width` × `height` with the given pixel format.
    #[must_use]
    pub fn new(width: u32, height: u32, descriptor: PixelDescriptor) -> Self {
        Self {
            width,
            height,
            descriptor,
            frame_count: 1,
        }
    }

    /// Number of animation frames (clamped to ≥ 1).
    #[must_use]
    pub fn with_frame_count(mut self, frames: u32) -> Self {
        self.frame_count = frames.max(1);
        self
    }

    /// Image width in pixels.
    #[must_use]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Image height in pixels.
    #[must_use]
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The pixel format of the source/decoded buffer.
    #[must_use]
    pub fn descriptor(&self) -> &PixelDescriptor {
        &self.descriptor
    }

    /// Animation frame count (1 for a still).
    #[must_use]
    pub fn frame_count(&self) -> u32 {
        self.frame_count
    }

    /// Pixel count (`width * height`).
    #[must_use]
    pub fn pixels(&self) -> u64 {
        self.width as u64 * self.height as u64
    }

    /// Size of one frame's tightly-packed pixel buffer in bytes
    /// (`pixels * bytes_per_pixel`).
    #[must_use]
    pub fn input_bytes(&self) -> u64 {
        self.pixels() * self.descriptor.bytes_per_pixel() as u64
    }
}

/// How an operation scales across CPU cores.
///
/// Wall time does **not** scale as `1/cores`: speedup saturates at
/// [`max_efficient_threads`](ThreadingInformation::max_efficient_threads) — the
/// knee of the scaling curve, set by the codec's tile / strategy / block count.
/// Below the knee, treat speedup as ~linear; above it, flat. The knee is
/// `Option`: `None` means "parallel, but the knee is unknown", and `at_cores`
/// then assumes linear scaling to all available cores.
///
/// Sealed and growable: construct via [`ThreadingInformation::SERIAL`],
/// [`ThreadingInformation::parallel`], or
/// [`ThreadingInformation::parallel_unknown_knee`]; read with the accessors.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ThreadingInformation {
    parallel: bool,
    max_efficient_threads: Option<u32>,
}

impl ThreadingInformation {
    /// A serial operation (no multi-core speedup).
    pub const SERIAL: Self = Self {
        parallel: false,
        max_efficient_threads: Some(1),
    };

    /// A parallel operation whose speedup saturates at `max_efficient_threads`
    /// (the knee of the scaling curve; clamped to ≥ 1).
    #[must_use]
    pub fn parallel(max_efficient_threads: u32) -> Self {
        Self {
            parallel: true,
            max_efficient_threads: Some(max_efficient_threads.max(1)),
        }
    }

    /// A parallel operation with **no known knee**: it scales, but the codec
    /// does not declare where added cores stop helping, so `effective_threads`
    /// (and thus [`at_cores`](ResourceEstimate::at_cores)) assumes linear
    /// scaling to all available cores.
    #[must_use]
    pub fn parallel_unknown_knee() -> Self {
        Self {
            parallel: true,
            max_efficient_threads: None,
        }
    }

    /// Whether the operation uses more than one core at all.
    #[must_use]
    pub fn is_parallel(&self) -> bool {
        self.parallel
    }

    /// The knee of the scaling curve: threads beyond which added cores stop
    /// yielding worthwhile speedup (`Some(1)` = serial, `None` = parallel with
    /// an unknown knee). This is the *efficient* cap, not a hard concurrency
    /// limit.
    #[must_use]
    pub fn max_efficient_threads(&self) -> Option<u32> {
        self.max_efficient_threads
    }

    /// Threads that actually do useful work given `cores` available: clamped to
    /// `max_efficient_threads` when the knee is known, else all `cores`.
    #[must_use]
    pub fn effective_threads(&self, cores: usize) -> u64 {
        let cores = cores.max(1) as u64;
        match self.max_efficient_threads {
            Some(knee) => cores.min(knee.max(1) as u64),
            None => cores,
        }
    }
}

/// Predicted resources for an encode (or decode) operation.
///
/// Every field is `Option` — a codec fills in what it models and leaves the
/// rest `None` (the trait default is [`ResourceEstimate::unknown`], all-`None`).
/// When a [`ThreadingInformation`] is carried, [`ResourceEstimate::at_cores`]
/// re-scales `wall_ms` for a given core count (leaving `cpu_ms` and peak
/// unchanged). Compare against [`ResourceLimits`](crate::ResourceLimits) to
/// decide whether to admit a job.
///
/// Sealed and growable: build via [`new`](ResourceEstimate::new) /
/// [`unknown`](ResourceEstimate::unknown) + the `with_*` setters, read with the
/// accessors.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ResourceEstimate {
    peak_memory_bytes_est: Option<u64>,
    peak_memory_bytes_max: Option<u64>,
    wall_ms: Option<u64>,
    cpu_ms: Option<u64>,
    threading: Option<ThreadingInformation>,
}

impl ResourceEstimate {
    /// An empty estimate — every field `None`. This is what a codec that does
    /// not model its resource use returns (and the trait default); codecs that
    /// do model fill in what they can with the `with_*` setters. Read fields
    /// back as `Option`.
    #[must_use]
    pub fn unknown() -> Self {
        Self {
            peak_memory_bytes_est: None,
            peak_memory_bytes_max: None,
            wall_ms: None,
            cpu_ms: None,
            threading: None,
        }
    }

    /// An estimate from the two essentials: the typical (estimated) peak memory
    /// and the wall time. Everything else — `peak_memory_bytes_max`, `cpu_ms`,
    /// and `threading` — is left `None`; refine with the `with_*` setters. With
    /// no `threading`, [`at_cores`](ResourceEstimate::at_cores) is a no-op until
    /// you add one.
    #[must_use]
    pub fn new(peak_memory_bytes_est: u64, wall_ms: u64) -> Self {
        Self {
            peak_memory_bytes_est: Some(peak_memory_bytes_est),
            peak_memory_bytes_max: None,
            wall_ms: Some(wall_ms),
            cpu_ms: None,
            threading: None,
        }
    }

    /// Set the conservative upper-bound peak memory (bytes).
    #[must_use]
    pub fn with_peak_max(mut self, max: u64) -> Self {
        self.peak_memory_bytes_max = Some(max);
        self
    }

    /// Set the total CPU-time estimate in milliseconds (work summed across all
    /// threads). Unlike `wall_ms`, it is **not** divided down by core count.
    #[must_use]
    pub fn with_cpu_ms(mut self, cpu_ms: u64) -> Self {
        self.cpu_ms = Some(cpu_ms);
        self
    }

    /// Attach the operation's core-scaling model.
    #[must_use]
    pub fn with_threading(mut self, threading: ThreadingInformation) -> Self {
        self.threading = Some(threading);
        self
    }

    /// Typical (≈ p50) estimated peak memory for natural content, bytes.
    #[must_use]
    pub fn peak_memory_bytes_est(&self) -> Option<u64> {
        self.peak_memory_bytes_est
    }

    /// Conservative upper-bound peak memory (worst content + margin), bytes.
    #[must_use]
    pub fn peak_memory_bytes_max(&self) -> Option<u64> {
        self.peak_memory_bytes_max
    }

    /// Predicted **wall-clock** time in milliseconds (single-thread unless
    /// produced by [`at_cores`](ResourceEstimate::at_cores)).
    #[must_use]
    pub fn wall_ms(&self) -> Option<u64> {
        self.wall_ms
    }

    /// Predicted total **CPU** time in milliseconds (work summed across all
    /// threads). Unaffected by [`at_cores`](ResourceEstimate::at_cores).
    #[must_use]
    pub fn cpu_ms(&self) -> Option<u64> {
        self.cpu_ms
    }

    /// How the operation scales across cores.
    #[must_use]
    pub fn threading(&self) -> Option<ThreadingInformation> {
        self.threading
    }

    /// Re-scale the predicted **wall** time for `cores` available CPU cores
    /// using the carried [`ThreadingInformation`]: `wall_ms` is divided by the
    /// effective thread count — linear speedup up to
    /// [`max_efficient_threads`](ThreadingInformation::max_efficient_threads),
    /// the knee of the scaling curve. `self` must carry the single-thread time.
    /// Peak memory and CPU time are unchanged. A `None` `wall_ms` or `None`
    /// threading is returned unscaled.
    #[must_use]
    pub fn at_cores(&self, cores: usize) -> Self {
        let mut out = *self;
        if let (Some(wall), Some(threading)) = (self.wall_ms, self.threading) {
            let n = threading.effective_threads(cores).max(1);
            out.wall_ms = Some(wall / n);
        }
        out
    }

    /// A conservative, content- and codec-blind fallback for operations
    /// without a calibrated model: peak ≈ input buffer + a generous working
    /// multiple, serial. Real codecs override
    /// [`EncoderConfig::estimate_encode_resources`](crate::encode::EncoderConfig::estimate_encode_resources)
    /// with their `heuristics` model.
    #[must_use]
    pub fn conservative(image: &ImageCharacteristics) -> Self {
        let input = image
            .input_bytes()
            .saturating_mul(image.frame_count() as u64);
        let fixed: u64 = 16 << 20;
        let typical = fixed.saturating_add(input.saturating_mul(3));
        // ~50 Mpix/s placeholder throughput; codecs override with measured.
        let wall_ms = image.pixels().saturating_mul(image.frame_count() as u64) / 50_000;
        Self::new(typical, wall_ms)
            .with_peak_max(fixed.saturating_add(input.saturating_mul(8)))
            .with_cpu_ms(wall_ms)
            .with_threading(ThreadingInformation::SERIAL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> PixelDescriptor {
        PixelDescriptor::RGB8_SRGB
    }

    #[test]
    fn compute_environment_builder_clamps_and_defaults() {
        assert_eq!(ComputeEnvironment::new().cores(), 1);
        assert_eq!(ComputeEnvironment::new().with_cores(0).cores(), 1);
        assert_eq!(ComputeEnvironment::default().with_cores(16).cores(), 16);
        assert_eq!(
            ComputeEnvironment::new()
                .with_available_ram_bytes(1 << 30)
                .available_ram_bytes(),
            Some(1 << 30)
        );
        // SIMD tier defaults to unspecified; the builder sets it.
        assert_eq!(ComputeEnvironment::new().simd_tier(), None);
        assert_eq!(
            ComputeEnvironment::new()
                .with_simd_tier(SimdTier::X86V3)
                .simd_tier(),
            Some(SimdTier::X86V3)
        );
    }

    #[test]
    fn image_characteristics_sizes() {
        let im = ImageCharacteristics::new(1024, 768, desc());
        assert_eq!(im.pixels(), 1024 * 768);
        assert_eq!(im.input_bytes(), 1024 * 768 * 3);
        assert_eq!(im.with_frame_count(0).frame_count(), 1);
    }

    #[test]
    fn serial_threading_is_one_thread() {
        let ti = ThreadingInformation::SERIAL;
        assert_eq!(ti.effective_threads(28), 1);
        assert_eq!(ti.max_efficient_threads(), Some(1));
        assert!(!ti.is_parallel());
    }

    #[test]
    fn parallel_effective_threads_saturate_at_the_knee() {
        let ti = ThreadingInformation::parallel(8);
        assert!(ti.is_parallel());
        assert_eq!(ti.max_efficient_threads(), Some(8));
        assert_eq!(ti.effective_threads(4), 4); // below the knee
        assert_eq!(ti.effective_threads(28), 8); // clamped to the knee
        assert_eq!(
            ThreadingInformation::parallel(0).max_efficient_threads(),
            Some(1)
        ); // >= 1
    }

    #[test]
    fn parallel_unknown_knee_scales_to_all_cores() {
        let ti = ThreadingInformation::parallel_unknown_knee();
        assert!(ti.is_parallel());
        assert_eq!(ti.max_efficient_threads(), None);
        assert_eq!(ti.effective_threads(28), 28); // no knee -> all cores
        assert_eq!(ti.effective_threads(1), 1);
    }

    #[test]
    fn at_cores_scales_wall_to_the_knee_and_leaves_peak_and_cpu() {
        let base = ResourceEstimate::new(200, 1000)
            .with_peak_max(400)
            .with_cpu_ms(1000)
            .with_threading(ThreadingInformation::parallel(8));
        // wall / effective_threads(4) = 1000 / 4
        assert_eq!(base.at_cores(4).wall_ms(), Some(250));
        // beyond the knee, no further gain: / 8
        assert_eq!(base.at_cores(28).wall_ms(), Some(125));
        // peak memory and CPU time are unchanged by at_cores
        assert_eq!(base.at_cores(4).peak_memory_bytes_est(), Some(200));
        assert_eq!(base.at_cores(4).peak_memory_bytes_max(), Some(400));
        assert_eq!(base.at_cores(4).cpu_ms(), Some(1000));
    }

    #[test]
    fn new_sets_only_peak_est_and_wall() {
        let est = ResourceEstimate::new(200, 1000);
        assert_eq!(est.peak_memory_bytes_est(), Some(200));
        assert_eq!(est.wall_ms(), Some(1000));
        // everything else opts in via setters
        assert_eq!(est.peak_memory_bytes_max(), None);
        assert_eq!(est.cpu_ms(), None);
        assert_eq!(est.threading(), None);
        // no threading -> at_cores is a no-op
        assert_eq!(est.at_cores(8).wall_ms(), Some(1000));
    }

    #[test]
    fn unknown_is_all_none_and_at_cores_is_a_noop() {
        let est = ResourceEstimate::unknown();
        assert_eq!(est.peak_memory_bytes_est(), None);
        assert_eq!(est.peak_memory_bytes_max(), None);
        assert_eq!(est.wall_ms(), None);
        assert_eq!(est.cpu_ms(), None);
        assert_eq!(est.threading(), None);
        assert_eq!(est.at_cores(28), est);
    }

    #[test]
    fn conservative_is_serial_and_input_scaled() {
        let est = ResourceEstimate::conservative(&ImageCharacteristics::new(1000, 1000, desc()));
        assert_eq!(est.threading().map(|t| t.is_parallel()), Some(false));
        assert!(est.peak_memory_bytes_est().unwrap() >= 1000 * 1000 * 3);
        // serial: at_cores leaves wall unchanged
        assert_eq!(est.at_cores(28).wall_ms(), est.wall_ms());
    }
}
