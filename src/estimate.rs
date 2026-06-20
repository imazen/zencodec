//! Unified resource estimation for encode/decode operations.
//!
//! Predicts an operation's **peak memory**, **wall time**, and **CPU-core
//! scaling** from three expandable inputs:
//!
//! 1. [`ImageChars`] — the image (dimensions + pixel format today; content
//!    class, frame count, HDR tier are future additions).
//! 2. the codec **config** — the [`EncoderConfig`](crate::encode::EncoderConfig)
//!    / [`DecoderConfig`](crate::decode::DecoderConfig) itself (it carries
//!    effort / quality / lossless / speed / thread intent).
//! 3. [`ComputeEnv`] — the hardware and conditions of computing (available
//!    cores now; available RAM, SIMD tier, load are future additions).
//!
//! [`ImageChars`] and [`ComputeEnv`] are **sealed, expandable builders**
//! (`#[non_exhaustive]`, constructed via `new` + `with_*`): new fields are
//! additive, so callers built today keep compiling.
//!
//! The codec answers via
//! [`EncoderConfig::estimate_encode_resources`](crate::encode::EncoderConfig::estimate_encode_resources)
//! (and the decode counterpart), returning a [`ResourceEstimate`]. Codecs with
//! a calibrated `heuristics` model override the default; the rest get a
//! conservative content-blind fallback.
//!
//! Wall time does **not** scale as `1/cores`: each codec carries a
//! [`ThreadingInfo`] (measured Amdahl fraction + the thread count beyond which
//! there is no further speedup), and [`ResourceEstimate::at_cores`] folds it in.

use zenpixels::PixelDescriptor;

/// Hardware + runtime conditions for a resource estimate.
///
/// Sealed/expandable builder — construct via [`ComputeEnv::new`] and refine
/// with the `with_*` setters. Carries the available core count today; new
/// fields (RAM, SIMD tier, load factor, GPU) are additive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ComputeEnv {
    available_cores: usize,
    available_ram_bytes: Option<u64>,
}

impl ComputeEnv {
    /// A single-core environment with unknown RAM (the conservative default).
    #[must_use]
    pub fn new() -> Self {
        Self {
            available_cores: 1,
            available_ram_bytes: None,
        }
    }

    /// Number of CPU cores available to the operation (≥ 1). On `std` callers
    /// typically pass `std::thread::available_parallelism()`.
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
}

impl Default for ComputeEnv {
    fn default() -> Self {
        Self::new()
    }
}

/// Characteristics of the image being encoded/decoded.
///
/// Sealed/expandable builder. Carries the dimensions and pixel format today;
/// future fields (content class, animation frame count, HDR tier depth) are
/// additive.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ImageChars {
    width: u32,
    height: u32,
    descriptor: PixelDescriptor,
    frame_count: u32,
}

impl ImageChars {
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

    /// Number of animation frames (≥ 1).
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
/// `max_useful_threads` (set by the codec's tile / strategy / block count) and
/// follows Amdahl's law with `parallel_fraction`. Peak working-set grows by
/// `mem_bytes_per_thread` per added worker.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ThreadingInfo {
    /// Whether the operation uses more than one core at all.
    pub parallel: bool,
    /// Threads beyond this yield no further speedup. 1 = serial.
    pub max_useful_threads: u32,
    /// Amdahl parallel fraction `p`; peak speedup is `1/(1-p)`. 0 = serial.
    pub parallel_fraction: f32,
    /// Extra peak working-set per added worker thread, in bytes.
    pub mem_bytes_per_thread: u64,
}

impl ThreadingInfo {
    /// A serial operation (no multi-core speedup, no per-thread memory).
    pub const SERIAL: Self = Self {
        parallel: false,
        max_useful_threads: 1,
        parallel_fraction: 0.0,
        mem_bytes_per_thread: 0,
    };

    /// A parallel operation with the given saturation, Amdahl fraction, and
    /// per-thread memory.
    #[must_use]
    pub fn parallel(max_useful_threads: u32, parallel_fraction: f32, mem_bytes_per_thread: u64) -> Self {
        Self {
            parallel: true,
            max_useful_threads: max_useful_threads.max(1),
            parallel_fraction,
            mem_bytes_per_thread,
        }
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
/// `peak_memory_bytes*` and `time_ms` are the **single-thread** figures plus
/// the carried [`ThreadingInfo`]; [`ResourceEstimate::at_cores`] re-scales them
/// for a given core count. Compare against
/// [`ResourceLimits`](crate::ResourceLimits) to decide whether to admit a job.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ResourceEstimate {
    /// Best-case peak memory (simple / low-entropy content), bytes.
    pub peak_memory_bytes_min: u64,
    /// Typical (≈ p50) peak memory for natural content, bytes.
    pub peak_memory_bytes: u64,
    /// Conservative upper-bound peak memory (worst content + margin), bytes.
    pub peak_memory_bytes_max: u64,
    /// Single-thread wall time in milliseconds (use [`at_cores`] to scale).
    ///
    /// [`at_cores`]: ResourceEstimate::at_cores
    pub time_ms: f32,
    /// Estimated output size in bytes.
    pub output_bytes: u64,
    /// How the operation scales across cores.
    pub threading: ThreadingInfo,
}

impl ResourceEstimate {
    /// Construct from explicit single-thread figures + a threading model.
    #[must_use]
    pub fn new(
        peak_memory_bytes_min: u64,
        peak_memory_bytes: u64,
        peak_memory_bytes_max: u64,
        time_ms: f32,
        output_bytes: u64,
        threading: ThreadingInfo,
    ) -> Self {
        Self {
            peak_memory_bytes_min,
            peak_memory_bytes,
            peak_memory_bytes_max,
            time_ms,
            output_bytes,
            threading,
        }
    }

    /// Re-scale wall time and peak memory for `cores` available CPU cores
    /// using the carried [`ThreadingInfo`]: `time_ms` is divided by the
    /// measured (saturating) speedup and the peaks gain the per-thread working
    /// set. `time_ms` on `self` must be the single-thread figure.
    #[must_use]
    pub fn at_cores(&self, cores: usize) -> Self {
        let sp = self.threading.speedup(cores) as f64;
        let extra = self
            .threading
            .mem_bytes_per_thread
            .saturating_mul(self.threading.effective_threads(cores).saturating_sub(1));
        Self {
            peak_memory_bytes_min: self.peak_memory_bytes_min.saturating_add(extra),
            peak_memory_bytes: self.peak_memory_bytes.saturating_add(extra),
            peak_memory_bytes_max: self.peak_memory_bytes_max.saturating_add(extra),
            time_ms: (self.time_ms as f64 / sp) as f32,
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
    pub fn conservative(image: &ImageChars) -> Self {
        let input = image.input_bytes().saturating_mul(image.frame_count() as u64);
        // Fixed overhead + a few input-buffers of working set; deliberately loose.
        let fixed: u64 = 16 << 20;
        let typical = fixed.saturating_add(input.saturating_mul(3));
        Self {
            peak_memory_bytes_min: fixed.saturating_add(input.saturating_mul(2)),
            peak_memory_bytes: typical,
            peak_memory_bytes_max: fixed.saturating_add(input.saturating_mul(8)),
            // ~50 Mpix/s placeholder throughput; codecs override with measured.
            time_ms: (image.pixels().saturating_mul(image.frame_count() as u64) as f64
                / 50_000.0) as f32,
            output_bytes: input / 4,
            threading: ThreadingInfo::SERIAL,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> PixelDescriptor {
        PixelDescriptor::RGB8_SRGB
    }

    #[test]
    fn compute_env_builder_clamps_and_defaults() {
        assert_eq!(ComputeEnv::new().cores(), 1);
        assert_eq!(ComputeEnv::new().with_cores(0).cores(), 1);
        assert_eq!(ComputeEnv::default().with_cores(16).cores(), 16);
        assert_eq!(
            ComputeEnv::new().with_available_ram_bytes(1 << 30).available_ram_bytes(),
            Some(1 << 30)
        );
    }

    #[test]
    fn image_chars_sizes() {
        let im = ImageChars::new(1024, 768, desc());
        assert_eq!(im.pixels(), 1024 * 768);
        assert_eq!(im.input_bytes(), 1024 * 768 * 3);
        assert_eq!(im.with_frame_count(0).frame_count(), 1);
    }

    #[test]
    fn serial_speedup_is_one() {
        let ti = ThreadingInfo::SERIAL;
        assert_eq!(ti.speedup(1), 1.0);
        assert_eq!(ti.speedup(28), 1.0);
        assert_eq!(ti.effective_threads(28), 1);
    }

    #[test]
    fn parallel_speedup_saturates_and_clamps() {
        let ti = ThreadingInfo::parallel(8, 0.9, 2_000_000);
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
        let ti = ThreadingInfo::parallel(8, 0.9, 2_000_000);
        let base = ResourceEstimate::new(100, 200, 400, 1000.0, 50, ti);
        let scaled = base.at_cores(8);
        // time drops by the speedup
        assert!(scaled.time_ms < base.time_ms);
        // peak grows by mem_bytes_per_thread * (8-1)
        assert_eq!(scaled.peak_memory_bytes, 200 + 2_000_000 * 7);
    }

    #[test]
    fn conservative_is_serial_and_input_scaled() {
        let est = ResourceEstimate::conservative(&ImageChars::new(1000, 1000, desc()));
        assert!(!est.threading.parallel);
        assert!(est.peak_memory_bytes >= 1000 * 1000 * 3);
        assert_eq!(est.at_cores(28).time_ms, est.time_ms); // serial: no change
    }
}
