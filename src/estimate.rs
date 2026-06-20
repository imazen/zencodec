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
    /// The SIMD tier is unknown or indeterminate — estimates fall back to
    /// their calibration-host assumption. (Most real targets have at least a
    /// baseline vector ISA, so a pure-scalar tier is rarely worth modelling;
    /// prefer the concrete variants when the tier is known.)
    Unknown,
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

/// How an operation scales across CPU cores (measured per-codec).
///
/// Wall time does **not** scale as `1/cores`: speedup saturates at
/// [`max_useful_threads`](ThreadingInformation::max_useful_threads) (set by the
/// codec's tile / strategy / block count) and follows Amdahl's law with
/// [`parallel_fraction`](ThreadingInformation::parallel_fraction). Peak
/// working-set grows by
/// [`memory_bytes_per_thread`](ThreadingInformation::memory_bytes_per_thread)
/// per added worker.
///
/// Sealed and growable: construct via [`ThreadingInformation::SERIAL`] or
/// [`ThreadingInformation::parallel`], read with the accessors.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ThreadingInformation {
    parallel: bool,
    max_useful_threads: u32,
    parallel_fraction: f32,
    memory_bytes_per_thread: u64,
}

impl ThreadingInformation {
    /// A serial operation (no multi-core speedup, no per-thread memory).
    pub const SERIAL: Self = Self {
        parallel: false,
        max_useful_threads: 1,
        parallel_fraction: 0.0,
        memory_bytes_per_thread: 0,
    };

    /// A parallel operation with the given saturation (`max_useful_threads`,
    /// clamped to ≥ 1), Amdahl `parallel_fraction`, and per-thread memory.
    #[must_use]
    pub fn parallel(max_useful_threads: u32, parallel_fraction: f32, memory_bytes_per_thread: u64) -> Self {
        Self {
            parallel: true,
            max_useful_threads: max_useful_threads.max(1),
            parallel_fraction,
            memory_bytes_per_thread,
        }
    }

    /// Whether the operation uses more than one core at all.
    #[must_use]
    pub fn is_parallel(&self) -> bool {
        self.parallel
    }

    /// Threads beyond which there is no further speedup (1 = serial).
    #[must_use]
    pub fn max_useful_threads(&self) -> u32 {
        self.max_useful_threads
    }

    /// Amdahl parallel fraction `p` (peak speedup is `1/(1-p)`; 0 = serial).
    #[must_use]
    pub fn parallel_fraction(&self) -> f32 {
        self.parallel_fraction
    }

    /// Extra peak working-set per added worker thread, in bytes.
    #[must_use]
    pub fn memory_bytes_per_thread(&self) -> u64 {
        self.memory_bytes_per_thread
    }

    /// Threads that actually do work given `cores` available (clamped to
    /// `max_useful_threads`).
    #[must_use]
    pub fn effective_threads(&self, cores: usize) -> u64 {
        (cores.max(1) as u64).min(self.max_useful_threads.max(1) as u64)
    }

    /// Achieved wall-time speedup at `cores` (Amdahl, clamped). 1.0 = serial.
    #[must_use]
    pub fn speedup(&self, cores: usize) -> f32 {
        let n = self.effective_threads(cores);
        if !self.parallel || n <= 1 {
            return 1.0;
        }
        let p = self.parallel_fraction as f64;
        (1.0 / ((1.0 - p) + p / n as f64)) as f32
    }
}

/// Predicted resources for an encode (or decode) operation.
///
/// The peak-memory and time figures are the **single-thread** values plus the
/// carried [`ThreadingInformation`]; [`ResourceEstimate::at_cores`] re-scales
/// them for a given core count. Compare against
/// [`ResourceLimits`](crate::ResourceLimits) to decide whether to admit a job.
///
/// Sealed and growable: build via [`ResourceEstimate::new`] + the `with_*`
/// setters, read with the accessors.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ResourceEstimate {
    peak_memory_bytes_min: u64,
    peak_memory_bytes: u64,
    peak_memory_bytes_max: u64,
    time_ms: f32,
    output_bytes: u64,
    threading: ThreadingInformation,
}

impl ResourceEstimate {
    /// A single-thread estimate from the two essentials: the typical peak
    /// memory and the single-thread wall time. Min/max peak default to the
    /// typical value, output to 0, and threading to serial — refine with the
    /// `with_*` setters.
    #[must_use]
    pub fn new(peak_memory_bytes: u64, time_ms: f32) -> Self {
        Self {
            peak_memory_bytes_min: peak_memory_bytes,
            peak_memory_bytes,
            peak_memory_bytes_max: peak_memory_bytes,
            time_ms,
            output_bytes: 0,
            threading: ThreadingInformation::SERIAL,
        }
    }

    /// Set the best-case and worst-case peak-memory bounds (bytes).
    #[must_use]
    pub fn with_peak_range(mut self, min: u64, max: u64) -> Self {
        self.peak_memory_bytes_min = min;
        self.peak_memory_bytes_max = max;
        self
    }

    /// Set the estimated output size in bytes.
    #[must_use]
    pub fn with_output_bytes(mut self, bytes: u64) -> Self {
        self.output_bytes = bytes;
        self
    }

    /// Attach the operation's core-scaling model.
    #[must_use]
    pub fn with_threading(mut self, threading: ThreadingInformation) -> Self {
        self.threading = threading;
        self
    }

    /// Best-case peak memory (simple / low-entropy content), bytes.
    #[must_use]
    pub fn peak_memory_bytes_min(&self) -> u64 {
        self.peak_memory_bytes_min
    }

    /// Typical (≈ p50) peak memory for natural content, bytes.
    #[must_use]
    pub fn peak_memory_bytes(&self) -> u64 {
        self.peak_memory_bytes
    }

    /// Conservative upper-bound peak memory (worst content + margin), bytes.
    #[must_use]
    pub fn peak_memory_bytes_max(&self) -> u64 {
        self.peak_memory_bytes_max
    }

    /// Wall time in milliseconds (single-thread unless produced by
    /// [`at_cores`](ResourceEstimate::at_cores)).
    #[must_use]
    pub fn time_ms(&self) -> f32 {
        self.time_ms
    }

    /// Estimated output size in bytes.
    #[must_use]
    pub fn output_bytes(&self) -> u64 {
        self.output_bytes
    }

    /// How the operation scales across cores.
    #[must_use]
    pub fn threading(&self) -> ThreadingInformation {
        self.threading
    }

    /// Re-scale wall time and peak memory for `cores` available CPU cores
    /// using the carried [`ThreadingInformation`]: `time_ms` is divided by the
    /// measured (saturating) speedup and the peaks gain the per-thread working
    /// set. `self` must carry the single-thread time.
    #[must_use]
    pub fn at_cores(&self, cores: usize) -> Self {
        let speedup = self.threading.speedup(cores) as f64;
        let extra = self
            .threading
            .memory_bytes_per_thread
            .saturating_mul(self.threading.effective_threads(cores).saturating_sub(1));
        Self {
            peak_memory_bytes_min: self.peak_memory_bytes_min.saturating_add(extra),
            peak_memory_bytes: self.peak_memory_bytes.saturating_add(extra),
            peak_memory_bytes_max: self.peak_memory_bytes_max.saturating_add(extra),
            time_ms: (self.time_ms as f64 / speedup) as f32,
            output_bytes: self.output_bytes,
            threading: self.threading,
        }
    }

    /// A conservative, content- and codec-blind fallback for operations
    /// without a calibrated model: peak ≈ input buffer + a generous working
    /// multiple, serial. Real codecs override
    /// [`EncoderConfig::estimate_encode_resources`](crate::encode::EncoderConfig::estimate_encode_resources)
    /// with their `heuristics` model.
    #[must_use]
    pub fn conservative(image: &ImageCharacteristics) -> Self {
        let input = image.input_bytes().saturating_mul(image.frame_count() as u64);
        let fixed: u64 = 16 << 20;
        let typical = fixed.saturating_add(input.saturating_mul(3));
        // ~50 Mpix/s placeholder throughput; codecs override with measured.
        let time_ms = (image.pixels().saturating_mul(image.frame_count() as u64) as f64 / 50_000.0) as f32;
        Self::new(typical, time_ms)
            .with_peak_range(
                fixed.saturating_add(input.saturating_mul(2)),
                fixed.saturating_add(input.saturating_mul(8)),
            )
            .with_output_bytes(input / 4)
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
            ComputeEnvironment::new().with_simd_tier(SimdTier::X86V3).simd_tier(),
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
    fn serial_speedup_is_one() {
        let ti = ThreadingInformation::SERIAL;
        assert_eq!(ti.speedup(1), 1.0);
        assert_eq!(ti.speedup(28), 1.0);
        assert_eq!(ti.effective_threads(28), 1);
        assert!(!ti.is_parallel());
    }

    #[test]
    fn parallel_speedup_saturates_and_clamps() {
        let ti = ThreadingInformation::parallel(8, 0.9, 2_000_000);
        assert!(ti.is_parallel());
        assert_eq!(ti.max_useful_threads(), 8);
        assert_eq!(ti.memory_bytes_per_thread(), 2_000_000);
        assert!(ti.speedup(1) == 1.0);
        // amdahl(0.9, 4) = 1/(0.1+0.225) ≈ 3.08
        let s4 = ti.speedup(4);
        assert!(s4 > 3.0 && s4 < 3.2, "got {s4}");
        // beyond max_useful_threads, no further gain
        assert_eq!(ti.speedup(8), ti.speedup(28));
        assert_eq!(ti.effective_threads(28), 8);
    }

    #[test]
    fn at_cores_scales_time_and_grows_peak() {
        let ti = ThreadingInformation::parallel(8, 0.9, 2_000_000);
        let base = ResourceEstimate::new(200, 1000.0)
            .with_peak_range(100, 400)
            .with_output_bytes(50)
            .with_threading(ti);
        let scaled = base.at_cores(8);
        assert!(scaled.time_ms() < base.time_ms());
        // peak grows by memory_bytes_per_thread * (8-1)
        assert_eq!(scaled.peak_memory_bytes(), 200 + 2_000_000 * 7);
    }

    #[test]
    fn conservative_is_serial_and_input_scaled() {
        let est = ResourceEstimate::conservative(&ImageCharacteristics::new(1000, 1000, desc()));
        assert!(!est.threading().is_parallel());
        assert!(est.peak_memory_bytes() >= 1000 * 1000 * 3);
        assert_eq!(est.at_cores(28).time_ms(), est.time_ms()); // serial: no change
    }
}
