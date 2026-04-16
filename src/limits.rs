//! Resource limits for codec operations.
//!
//! [`ResourceLimits`] defines caps on resource usage. [`LimitExceeded`]
//! is returned when a check fails. Use the `check_*` methods for
//! parse-time rejection (fastest — reject before any pixel work).

/// Threading policy for codec operations.
///
/// Two variants: [`Sequential`](Self::Sequential) (no parallelism) and
/// [`Parallel`](Self::Parallel) (use rayon, the default). Codecs call
/// [`is_parallel()`](Self::is_parallel) and that's their only decision.
///
/// # How threading works
///
/// Zen codecs use [rayon](https://docs.rs/rayon) for parallelism. Rayon's
/// work-stealing scheduler distributes `par_iter()` / `join()` work across
/// a thread pool. The key design rule:
///
/// **Codecs never create thread pools. The caller owns the pool.**
///
/// By default, rayon's global pool (sized to the number of CPU cores) is
/// used. This is almost always what you want — multiple concurrent
/// encode/decode operations share the same pool, and rayon's work-stealing
/// automatically balances load across them.
///
/// # Controlling thread count
///
/// Thread count is controlled externally via [`rayon::ThreadPool::install()`]:
///
/// ```ignore
/// use rayon::ThreadPoolBuilder;
///
/// // 4-thread pool for this server
/// let pool = ThreadPoolBuilder::new().num_threads(4).build().unwrap();
///
/// // All rayon work inside this closure uses the 4-thread pool
/// pool.install(|| {
///     let output = encoder.encode(pixels)?;
///     Ok(output)
/// });
/// ```
///
/// Inside `install()`, `rayon::current_num_threads()` returns the pool's
/// thread count. Codecs that size their work chunks (e.g., MCU row batches
/// in JPEG) adapt automatically.
///
/// # Server pattern: shared pool across concurrent requests
///
/// For servers processing many images concurrently, use **one shared pool**
/// for all encode/decode work. Rayon's work-stealing means idle threads
/// from a finished encode immediately help with in-progress encodes:
///
/// ```ignore
/// use std::sync::LazyLock;
/// use rayon::{ThreadPool, ThreadPoolBuilder};
///
/// static ENCODE_POOL: LazyLock<ThreadPool> = LazyLock::new(|| {
///     ThreadPoolBuilder::new()
///         .num_threads(num_cpus::get())
///         .build()
///         .unwrap()
/// });
///
/// // Every request uses the same pool
/// fn handle_request(data: &[u8]) -> Vec<u8> {
///     ENCODE_POOL.install(|| {
///         let decoded = decoder.decode(data).unwrap();
///         encoder.encode(decoded).unwrap().into_vec()
///     })
/// }
/// ```
///
/// Don't create a pool per request or per encode — that defeats
/// work-stealing and creates OS thread overhead.
///
/// # Sequential mode
///
/// `ThreadingPolicy::Sequential` tells codecs to skip `par_iter()`
/// entirely and use `iter()`. No rayon calls, zero overhead. Use this
/// for deterministic output, single-threaded benchmarks, or
/// environments where threading is undesirable (WASM, embedded).
///
/// # Native-threaded codecs (rav1d, zenrav1e)
///
/// Some codecs spawn OS threads internally instead of using rayon (the
/// AV1 decoder/encoder inherited this from dav1d/rav1e). These codecs
/// interpret `Sequential` as "1 thread" and `Parallel` as "use your
/// default thread count." The `pool.install()` pattern has no effect
/// on these codecs — they manage their own threads. Control their
/// thread count via codec-specific config (e.g.,
/// `AvifEncoderConfig::with_threads(4)`).
///
/// # For codec implementors
///
/// Check [`is_parallel()`](Self::is_parallel) once, then use `par_iter()`
/// or `iter()` accordingly:
///
/// ```ignore
/// let threading = limits.threading;
/// if threading.is_parallel() {
///     rows.par_chunks_mut(chunk_size).for_each(|chunk| process(chunk));
/// } else {
///     rows.chunks_mut(chunk_size).for_each(|chunk| process(chunk));
/// }
/// ```
///
/// Do **not** create `ThreadPool`s inside codec code. Use the ambient
/// pool — the caller controls which pool that is via `install()`.
/// Codecs that need to size work batches should call
/// `rayon::current_num_threads()` at the point of splitting, not at
/// configuration time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum ThreadingPolicy {
    /// No parallelism. Codecs must use sequential code paths (`iter()`,
    /// not `par_iter()`). Zero rayon overhead.
    ///
    /// For native-threaded codecs, this means `threads = 1`.
    Sequential,

    /// Use the ambient rayon pool (default). The caller controls thread
    /// count by choosing which pool to `install()` into. Codecs should
    /// use `par_iter()` / `rayon::join()` freely — rayon routes work to
    /// whatever pool is active.
    ///
    /// For native-threaded codecs, this means "use your default thread count."
    #[default]
    Parallel,

    /// Equivalent to [`Sequential`](Self::Sequential).
    #[deprecated(since = "0.2.0", note = "use ThreadingPolicy::Sequential")]
    SingleThread,

    /// Deprecated. Rayon codecs cannot reliably cap thread count from inside —
    /// use `Sequential` or `Parallel` and control threads via `pool.install()`.
    #[deprecated(since = "0.2.0", note = "use Sequential or Parallel")]
    LimitOrSingle {
        /// Maximum thread count before falling back to single-threaded.
        max_threads: u16,
    },

    /// Deprecated. See [`LimitOrSingle`](Self::LimitOrSingle).
    #[deprecated(since = "0.2.0", note = "use Sequential or Parallel")]
    LimitOrAny {
        /// Preferred maximum thread count (advisory, not enforced).
        preferred_max_threads: u16,
    },

    /// Equivalent to [`Parallel`](Self::Parallel).
    #[deprecated(since = "0.2.0", note = "use ThreadingPolicy::Parallel")]
    Balanced,

    /// Equivalent to [`Parallel`](Self::Parallel).
    #[deprecated(since = "0.2.0", note = "use ThreadingPolicy::Parallel")]
    Unlimited,
}

#[allow(deprecated)]
impl ThreadingPolicy {
    /// Whether the codec should use parallel code paths.
    ///
    /// Returns `false` for `Sequential` (and deprecated `SingleThread`,
    /// `LimitOrSingle { max_threads: 1 }`). Returns `true` for everything else.
    ///
    /// **Rayon-based codecs** (zenjpeg, jxl-encoder, zenjxl-decoder, zenpng):
    /// use this to choose between `par_iter()` and `iter()`. Thread count
    /// comes from the ambient rayon pool — the caller controls it via
    /// `pool.install(|| ...)`.
    ///
    /// **Native-threaded codecs** (rav1d-safe, zenrav1e): use this to decide
    /// between single-threaded (`false` → 1 thread) and multi-threaded
    /// (`true` → codec's default thread count).
    pub fn is_parallel(self) -> bool {
        match self {
            Self::Sequential | Self::SingleThread => false,
            Self::LimitOrSingle { max_threads } if max_threads <= 1 => false,
            _ => true,
        }
    }
}

/// Resource limits for encode/decode operations.
///
/// Used to prevent DoS attacks and resource exhaustion. All fields are optional;
/// `None` means no limit for that resource.
///
/// Codecs enforce what they can — not all codecs support all limit types.
/// Use the `check_*` methods for caller-side validation before decode/encode.
///
/// # Example
///
/// ```
/// use zencodec::ResourceLimits;
///
/// let limits = ResourceLimits::none()
///     .with_max_pixels(100_000_000)
///     .with_max_memory(512 * 1024 * 1024);
/// ```
///
/// Typical usage with a decoder:
///
/// ```ignore
/// // Parse-time rejection (before any pixel work)
/// let info = config.probe_header(data)?;
/// limits.check_image_info(&info)?;
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ResourceLimits {
    /// Maximum pixels in a single frame (width × height).
    ///
    /// This is a **per-frame** limit. For animations, each frame is checked
    /// independently. To limit the cumulative pixel count across all frames,
    /// use [`max_total_pixels`](Self::max_total_pixels).
    pub max_pixels: Option<u64>,
    /// Maximum memory allocation in bytes.
    pub max_memory_bytes: Option<u64>,
    /// Maximum encoded output size in bytes (encode only).
    pub max_output_bytes: Option<u64>,
    /// Maximum image width in pixels.
    pub max_width: Option<u32>,
    /// Maximum image height in pixels.
    pub max_height: Option<u32>,
    /// Maximum input data size in bytes (decode only).
    pub max_input_bytes: Option<u64>,
    /// Maximum number of animation frames.
    pub max_frames: Option<u32>,
    /// Maximum total animation duration in milliseconds.
    pub max_animation_ms: Option<u64>,
    /// Maximum pixels across **all frames** (width × height × frame_count).
    ///
    /// A 1000×1000 animation with 200 frames has 200 million total pixels.
    /// [`max_pixels`](Self::max_pixels) would pass each 1M-pixel frame
    /// individually — this field catches the cumulative cost.
    ///
    /// Checked by [`check_image_info`](Self::check_image_info) when
    /// `frame_count` is known; for unknown frame counts, use
    /// [`check_total_pixels`](Self::check_total_pixels) incrementally.
    pub max_total_pixels: Option<u64>,
    /// Threading policy for the codec.
    ///
    /// Defaults to [`ThreadingPolicy::Parallel`].
    pub threading: ThreadingPolicy,
}

// All primitives, no pointers — but Option<u64> niche optimization and
// enum discriminant alignment can differ between 32-bit and 64-bit.
#[cfg(target_pointer_width = "64")]
const _: () = assert!(core::mem::size_of::<ResourceLimits>() == 128);

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_pixels: None,
            max_memory_bytes: None,
            max_output_bytes: None,
            max_width: None,
            max_height: None,
            max_input_bytes: None,
            max_frames: None,
            max_animation_ms: None,
            max_total_pixels: None,
            threading: ThreadingPolicy::Parallel,
        }
    }
}

impl ResourceLimits {
    /// No limits (all fields `None`), parallel threading (ambient pool).
    pub fn none() -> Self {
        Self::default()
    }

    /// Set maximum total pixels.
    pub fn with_max_pixels(mut self, max: u64) -> Self {
        self.max_pixels = Some(max);
        self
    }

    /// Set maximum memory allocation in bytes.
    pub fn with_max_memory(mut self, bytes: u64) -> Self {
        self.max_memory_bytes = Some(bytes);
        self
    }

    /// Set maximum encoded output size in bytes.
    pub fn with_max_output(mut self, bytes: u64) -> Self {
        self.max_output_bytes = Some(bytes);
        self
    }

    /// Set maximum image width in pixels.
    pub fn with_max_width(mut self, width: u32) -> Self {
        self.max_width = Some(width);
        self
    }

    /// Set maximum image height in pixels.
    pub fn with_max_height(mut self, height: u32) -> Self {
        self.max_height = Some(height);
        self
    }

    /// Set maximum input data size in bytes (decode only).
    pub fn with_max_input_bytes(mut self, bytes: u64) -> Self {
        self.max_input_bytes = Some(bytes);
        self
    }

    /// Set maximum number of animation frames.
    pub fn with_max_frames(mut self, frames: u32) -> Self {
        self.max_frames = Some(frames);
        self
    }

    /// Set maximum total animation duration in milliseconds.
    pub fn with_max_animation_ms(mut self, ms: u64) -> Self {
        self.max_animation_ms = Some(ms);
        self
    }

    /// Set maximum total pixels across all frames.
    pub fn with_max_total_pixels(mut self, max: u64) -> Self {
        self.max_total_pixels = Some(max);
        self
    }

    /// Set threading policy.
    pub fn with_threading(mut self, policy: ThreadingPolicy) -> Self {
        self.threading = policy;
        self
    }

    /// Current threading policy.
    pub fn threading(&self) -> ThreadingPolicy {
        self.threading
    }

    /// Whether any limits are set (including non-default threading).
    pub fn has_any(&self) -> bool {
        self.max_pixels.is_some()
            || self.max_memory_bytes.is_some()
            || self.max_output_bytes.is_some()
            || self.max_width.is_some()
            || self.max_height.is_some()
            || self.max_input_bytes.is_some()
            || self.max_frames.is_some()
            || self.max_animation_ms.is_some()
            || self.max_total_pixels.is_some()
            || !matches!(self.threading, ThreadingPolicy::Parallel)
    }

    // --- Validation methods ---

    /// Check image dimensions against `max_width`, `max_height`, and `max_pixels`.
    pub fn check_dimensions(&self, width: u32, height: u32) -> Result<(), LimitExceeded> {
        if let Some(max) = self.max_width
            && width > max
        {
            return Err(LimitExceeded::Width { actual: width, max });
        }
        if let Some(max) = self.max_height
            && height > max
        {
            return Err(LimitExceeded::Height {
                actual: height,
                max,
            });
        }
        if let Some(max) = self.max_pixels {
            let pixels = width as u64 * height as u64;
            if pixels > max {
                return Err(LimitExceeded::Pixels {
                    actual: pixels,
                    max,
                });
            }
        }
        Ok(())
    }

    /// Check a memory estimate against `max_memory_bytes`.
    pub fn check_memory(&self, bytes: u64) -> Result<(), LimitExceeded> {
        if let Some(max) = self.max_memory_bytes
            && bytes > max
        {
            return Err(LimitExceeded::Memory { actual: bytes, max });
        }
        Ok(())
    }

    /// Check input data size against `max_input_bytes`.
    pub fn check_input_size(&self, bytes: u64) -> Result<(), LimitExceeded> {
        if let Some(max) = self.max_input_bytes
            && bytes > max
        {
            return Err(LimitExceeded::InputSize { actual: bytes, max });
        }
        Ok(())
    }

    /// Check encoded output size against `max_output_bytes`.
    pub fn check_output_size(&self, bytes: u64) -> Result<(), LimitExceeded> {
        if let Some(max) = self.max_output_bytes
            && bytes > max
        {
            return Err(LimitExceeded::OutputSize { actual: bytes, max });
        }
        Ok(())
    }

    /// Check frame count against `max_frames`.
    pub fn check_frames(&self, count: u32) -> Result<(), LimitExceeded> {
        if let Some(max) = self.max_frames
            && count > max
        {
            return Err(LimitExceeded::Frames { actual: count, max });
        }
        Ok(())
    }

    /// Check animation duration against `max_animation_ms`.
    pub fn check_animation_ms(&self, ms: u64) -> Result<(), LimitExceeded> {
        if let Some(max) = self.max_animation_ms
            && ms > max
        {
            return Err(LimitExceeded::Duration { actual: ms, max });
        }
        Ok(())
    }

    /// Check total pixels across all frames against `max_total_pixels`.
    pub fn check_total_pixels(&self, total: u64) -> Result<(), LimitExceeded> {
        if let Some(max) = self.max_total_pixels
            && total > max
        {
            return Err(LimitExceeded::TotalPixels { actual: total, max });
        }
        Ok(())
    }

    /// Check [`ImageInfo`](crate::ImageInfo) from `probe_header()` against all
    /// applicable limits. This is the fastest rejection point — call it
    /// immediately after probing, before any pixel work.
    ///
    /// Checks: `max_width`, `max_height`, `max_pixels`, `max_frames`,
    /// `max_total_pixels`.
    pub fn check_image_info(&self, info: &crate::ImageInfo) -> Result<(), LimitExceeded> {
        self.check_dimensions(info.width, info.height)?;
        if let Some(max) = self.max_frames
            && let Some(count) = info.frame_count()
            && count > max
        {
            return Err(LimitExceeded::Frames { actual: count, max });
        }
        if let Some(max) = self.max_total_pixels
            && let Some(count) = info.frame_count()
        {
            let total = info.width as u64 * info.height as u64 * count as u64;
            if total > max {
                return Err(LimitExceeded::TotalPixels { actual: total, max });
            }
        }
        Ok(())
    }

    /// Check [`OutputInfo`](crate::decode::OutputInfo) against dimension limits.
    ///
    /// Checks: `max_width`, `max_height`, `max_pixels`.
    pub fn check_output_info(&self, info: &crate::OutputInfo) -> Result<(), LimitExceeded> {
        self.check_dimensions(info.width, info.height)
    }
}

/// A resource limit was exceeded.
///
/// Returned by [`ResourceLimits::check_dimensions()`] and related methods.
/// Each variant carries the actual value and the limit that was exceeded,
/// enabling useful error messages.
///
/// Implements [`core::error::Error`] so codecs can wrap it in their own
/// error types.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LimitExceeded {
    /// Image width exceeded `max_width`.
    Width {
        /// Actual width.
        actual: u32,
        /// Maximum allowed.
        max: u32,
    },
    /// Image height exceeded `max_height`.
    Height {
        /// Actual height.
        actual: u32,
        /// Maximum allowed.
        max: u32,
    },
    /// Pixel count exceeded `max_pixels`.
    Pixels {
        /// Actual pixel count.
        actual: u64,
        /// Maximum allowed.
        max: u64,
    },
    /// Memory exceeded `max_memory_bytes`.
    Memory {
        /// Estimated memory in bytes.
        actual: u64,
        /// Maximum allowed.
        max: u64,
    },
    /// Input data size exceeded `max_input_bytes`.
    InputSize {
        /// Actual input size in bytes.
        actual: u64,
        /// Maximum allowed.
        max: u64,
    },
    /// Encoded output exceeded `max_output_bytes`.
    OutputSize {
        /// Actual or estimated output size in bytes.
        actual: u64,
        /// Maximum allowed.
        max: u64,
    },
    /// Frame count exceeded `max_frames`.
    Frames {
        /// Actual frame count.
        actual: u32,
        /// Maximum allowed.
        max: u32,
    },
    /// Animation duration exceeded `max_animation_ms`.
    Duration {
        /// Actual duration in milliseconds.
        actual: u64,
        /// Maximum allowed.
        max: u64,
    },
    /// Total pixels across all frames exceeded `max_total_pixels`.
    TotalPixels {
        /// Actual total pixel count (width × height × frames).
        actual: u64,
        /// Maximum allowed.
        max: u64,
    },
}

impl core::fmt::Display for LimitExceeded {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Width { actual, max } => write!(f, "width {actual} exceeds limit {max}"),
            Self::Height { actual, max } => write!(f, "height {actual} exceeds limit {max}"),
            Self::Pixels { actual, max } => {
                write!(f, "pixel count {actual} exceeds limit {max}")
            }
            Self::Memory { actual, max } => {
                write!(f, "memory {actual} bytes exceeds limit {max}")
            }
            Self::InputSize { actual, max } => {
                write!(f, "input size {actual} bytes exceeds limit {max}")
            }
            Self::OutputSize { actual, max } => {
                write!(f, "output size {actual} bytes exceeds limit {max}")
            }
            Self::Frames { actual, max } => {
                write!(f, "frame count {actual} exceeds limit {max}")
            }
            Self::Duration { actual, max } => {
                write!(f, "duration {actual}ms exceeds limit {max}ms")
            }
            Self::TotalPixels { actual, max } => {
                write!(f, "total pixels {actual} exceeds limit {max}")
            }
        }
    }
}

impl core::error::Error for LimitExceeded {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_has_no_limits() {
        let limits = ResourceLimits::none();
        assert!(!limits.has_any());
    }

    #[test]
    fn builder_sets_limits() {
        let limits = ResourceLimits::none()
            .with_max_pixels(1_000_000)
            .with_max_memory(512 * 1024 * 1024);
        assert!(limits.has_any());
        assert_eq!(limits.max_pixels, Some(1_000_000));
        assert_eq!(limits.max_memory_bytes, Some(512 * 1024 * 1024));
        assert!(limits.max_output_bytes.is_none());
    }

    #[test]
    fn animation_limits() {
        let limits = ResourceLimits::none()
            .with_max_frames(100)
            .with_max_animation_ms(30_000);
        assert!(limits.has_any());
        assert_eq!(limits.max_frames, Some(100));
        assert_eq!(limits.max_animation_ms, Some(30_000));
    }

    #[test]
    fn has_any_includes_animation_fields() {
        let limits = ResourceLimits::none().with_max_frames(10);
        assert!(limits.has_any());

        let limits = ResourceLimits::none().with_max_animation_ms(5000);
        assert!(limits.has_any());
    }

    #[test]
    fn threading_policy_default() {
        let limits = ResourceLimits::none();
        assert_eq!(limits.threading(), ThreadingPolicy::Parallel);
        assert!(!limits.has_any());
    }

    #[test]
    fn threading_policy_sequential() {
        let limits = ResourceLimits::none().with_threading(ThreadingPolicy::Sequential);
        assert!(limits.has_any());
        assert_eq!(limits.threading(), ThreadingPolicy::Sequential);
        assert!(!limits.threading().is_parallel());
    }

    #[test]
    fn threading_policy_parallel() {
        let limits = ResourceLimits::none().with_threading(ThreadingPolicy::Parallel);
        assert!(!limits.has_any());
        assert!(limits.threading().is_parallel());
    }

    #[allow(deprecated)]
    #[test]
    fn threading_policy_legacy_single_thread() {
        let limits = ResourceLimits::none().with_threading(ThreadingPolicy::SingleThread);
        assert!(limits.has_any());
        assert!(!limits.threading().is_parallel());
    }

    #[allow(deprecated)]
    #[test]
    fn threading_policy_legacy_limit_or_single() {
        let limits = ResourceLimits::none()
            .with_threading(ThreadingPolicy::LimitOrSingle { max_threads: 4 });
        assert!(limits.has_any());
        assert!(limits.threading().is_parallel());
    }

    #[allow(deprecated)]
    #[test]
    fn threading_policy_legacy_limit_or_single_1() {
        let limits = ResourceLimits::none()
            .with_threading(ThreadingPolicy::LimitOrSingle { max_threads: 1 });
        assert!(!limits.threading().is_parallel());
    }

    #[allow(deprecated)]
    #[test]
    fn threading_policy_legacy_balanced() {
        let limits = ResourceLimits::none().with_threading(ThreadingPolicy::Balanced);
        assert!(limits.has_any());
        assert!(limits.threading().is_parallel());
    }

    #[allow(deprecated)]
    #[test]
    fn threading_policy_legacy_unlimited() {
        assert!(ThreadingPolicy::Unlimited.is_parallel());
    }

    // --- Validation tests ---

    #[test]
    fn check_dimensions_pass() {
        let limits = ResourceLimits::none()
            .with_max_width(1920)
            .with_max_height(1080)
            .with_max_pixels(2_073_600);
        assert!(limits.check_dimensions(1920, 1080).is_ok());
        assert!(limits.check_dimensions(100, 100).is_ok());
    }

    #[test]
    fn check_dimensions_width_exceeded() {
        let limits = ResourceLimits::none().with_max_width(1920);
        let err = limits.check_dimensions(1921, 1080).unwrap_err();
        assert_eq!(
            err,
            LimitExceeded::Width {
                actual: 1921,
                max: 1920
            }
        );
    }

    #[test]
    fn check_dimensions_height_exceeded() {
        let limits = ResourceLimits::none().with_max_height(1080);
        let err = limits.check_dimensions(1920, 1081).unwrap_err();
        assert_eq!(
            err,
            LimitExceeded::Height {
                actual: 1081,
                max: 1080
            }
        );
    }

    #[test]
    fn check_dimensions_pixels_exceeded() {
        let limits = ResourceLimits::none().with_max_pixels(1_000_000);
        let err = limits.check_dimensions(1001, 1000).unwrap_err();
        assert_eq!(
            err,
            LimitExceeded::Pixels {
                actual: 1_001_000,
                max: 1_000_000
            }
        );
    }

    #[test]
    fn check_dimensions_no_limits_always_passes() {
        let limits = ResourceLimits::none();
        assert!(limits.check_dimensions(100_000, 100_000).is_ok());
    }

    #[test]
    fn check_memory_pass_and_fail() {
        let limits = ResourceLimits::none().with_max_memory(512 * 1024 * 1024);
        assert!(limits.check_memory(256 * 1024 * 1024).is_ok());
        let err = limits.check_memory(1024 * 1024 * 1024).unwrap_err();
        assert!(matches!(err, LimitExceeded::Memory { .. }));
    }

    #[test]
    fn check_input_size_pass_and_fail() {
        let limits = ResourceLimits::none().with_max_input_bytes(10 * 1024 * 1024);
        assert!(limits.check_input_size(5 * 1024 * 1024).is_ok());
        let err = limits.check_input_size(20 * 1024 * 1024).unwrap_err();
        assert!(matches!(err, LimitExceeded::InputSize { .. }));
    }

    #[test]
    fn check_output_size_pass_and_fail() {
        let limits = ResourceLimits::none().with_max_output(1024);
        assert!(limits.check_output_size(512).is_ok());
        let err = limits.check_output_size(2048).unwrap_err();
        assert!(matches!(err, LimitExceeded::OutputSize { .. }));
    }

    #[test]
    fn check_frames_pass_and_fail() {
        let limits = ResourceLimits::none().with_max_frames(100);
        assert!(limits.check_frames(50).is_ok());
        let err = limits.check_frames(200).unwrap_err();
        assert_eq!(
            err,
            LimitExceeded::Frames {
                actual: 200,
                max: 100
            }
        );
    }

    #[test]
    fn check_animation_ms_pass_and_fail() {
        let limits = ResourceLimits::none().with_max_animation_ms(30_000);
        assert!(limits.check_animation_ms(15_000).is_ok());
        let err = limits.check_animation_ms(60_000).unwrap_err();
        assert!(matches!(err, LimitExceeded::Duration { .. }));
    }

    #[test]
    fn check_image_info_dimensions_and_frames() {
        use crate::{ImageFormat, ImageInfo};
        let limits = ResourceLimits::none()
            .with_max_width(4096)
            .with_max_pixels(16_000_000)
            .with_max_frames(100);

        let info = ImageInfo::new(3840, 2160, ImageFormat::Avif).with_sequence(
            crate::ImageSequence::Animation {
                frame_count: Some(50),
                loop_count: None,
                random_access: false,
            },
        );
        assert!(limits.check_image_info(&info).is_ok());

        let big = ImageInfo::new(5000, 4000, ImageFormat::Jpeg);
        let err = limits.check_image_info(&big).unwrap_err();
        assert!(matches!(err, LimitExceeded::Width { .. }));

        let many_frames = ImageInfo::new(100, 100, ImageFormat::Gif).with_sequence(
            crate::ImageSequence::Animation {
                frame_count: Some(200),
                loop_count: None,
                random_access: false,
            },
        );
        let err = limits.check_image_info(&many_frames).unwrap_err();
        assert_eq!(
            err,
            LimitExceeded::Frames {
                actual: 200,
                max: 100
            }
        );
    }

    #[test]
    fn limit_exceeded_display() {
        use alloc::format;
        let err = LimitExceeded::Width {
            actual: 5000,
            max: 4096,
        };
        assert_eq!(format!("{err}"), "width 5000 exceeds limit 4096");

        let err = LimitExceeded::InputSize {
            actual: 20_000_000,
            max: 10_000_000,
        };
        assert_eq!(
            format!("{err}"),
            "input size 20000000 bytes exceeds limit 10000000"
        );

        let err = LimitExceeded::Duration {
            actual: 60_000,
            max: 30_000,
        };
        assert_eq!(format!("{err}"), "duration 60000ms exceeds limit 30000ms");
    }

    #[test]
    fn limit_exceeded_is_error() {
        fn assert_error<E: core::error::Error>(_: &E) {}
        let err = LimitExceeded::Width {
            actual: 5000,
            max: 4096,
        };
        assert_error(&err);
    }

    #[test]
    fn total_pixels_builder_and_has_any() {
        let limits = ResourceLimits::none().with_max_total_pixels(100_000_000);
        assert!(limits.has_any());
        assert_eq!(limits.max_total_pixels, Some(100_000_000));
    }

    #[test]
    fn check_total_pixels_pass_and_fail() {
        let limits = ResourceLimits::none().with_max_total_pixels(50_000_000);
        assert!(limits.check_total_pixels(50_000_000).is_ok());
        let err = limits.check_total_pixels(50_000_001).unwrap_err();
        assert_eq!(
            err,
            LimitExceeded::TotalPixels {
                actual: 50_000_001,
                max: 50_000_000
            }
        );
    }

    #[test]
    fn check_total_pixels_no_limit() {
        let limits = ResourceLimits::none();
        assert!(limits.check_total_pixels(u64::MAX).is_ok());
    }

    #[test]
    fn check_image_info_total_pixels() {
        use crate::{ImageFormat, ImageInfo};
        // 1000×1000 × 200 frames = 200M pixels, limit 100M
        let limits = ResourceLimits::none().with_max_total_pixels(100_000_000);
        let info = ImageInfo::new(1000, 1000, ImageFormat::Gif).with_sequence(
            crate::ImageSequence::Animation {
                frame_count: Some(200),
                loop_count: None,
                random_access: false,
            },
        );
        let err = limits.check_image_info(&info).unwrap_err();
        assert_eq!(
            err,
            LimitExceeded::TotalPixels {
                actual: 200_000_000,
                max: 100_000_000
            }
        );
    }

    #[test]
    fn check_image_info_total_pixels_pass() {
        use crate::{ImageFormat, ImageInfo};
        let limits = ResourceLimits::none().with_max_total_pixels(100_000_000);
        let info = ImageInfo::new(1000, 1000, ImageFormat::Gif).with_sequence(
            crate::ImageSequence::Animation {
                frame_count: Some(100),
                loop_count: None,
                random_access: false,
            },
        );
        assert!(limits.check_image_info(&info).is_ok());
    }

    #[test]
    fn check_image_info_total_pixels_still_image() {
        use crate::{ImageFormat, ImageInfo};
        // Still image: frame_count() returns Some(1), so total = w*h*1
        let limits = ResourceLimits::none().with_max_total_pixels(1_000_000);
        let info = ImageInfo::new(1000, 1000, ImageFormat::Jpeg);
        assert!(limits.check_image_info(&info).is_ok());

        // Just over: 1001×1000×1 = 1_001_000 > 1_000_000
        let info = ImageInfo::new(1001, 1000, ImageFormat::Jpeg);
        let err = limits.check_image_info(&info).unwrap_err();
        assert_eq!(
            err,
            LimitExceeded::TotalPixels {
                actual: 1_001_000,
                max: 1_000_000
            }
        );
    }

    #[test]
    fn total_pixels_display() {
        use alloc::format;
        let err = LimitExceeded::TotalPixels {
            actual: 200_000_000,
            max: 100_000_000,
        };
        assert_eq!(
            format!("{err}"),
            "total pixels 200000000 exceeds limit 100000000"
        );
    }
}
