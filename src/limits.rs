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
/// Thread count is controlled externally via `rayon::ThreadPool::install()`:
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
    /// Equivalent to [`Sequential`](Self::Sequential).
    #[deprecated(since = "0.1.19", note = "use ThreadingPolicy::Sequential")]
    SingleThread,

    /// Deprecated. Rayon codecs cannot reliably cap thread count from inside —
    /// use `Sequential` or `Parallel` and control threads via `pool.install()`.
    #[deprecated(since = "0.1.19", note = "use Sequential or Parallel")]
    LimitOrSingle {
        /// Maximum thread count before falling back to single-threaded.
        max_threads: u16,
    },

    /// Deprecated. See [`LimitOrSingle`](Self::LimitOrSingle).
    #[deprecated(since = "0.1.19", note = "use Sequential or Parallel")]
    LimitOrAny {
        /// Preferred maximum thread count (advisory, not enforced).
        preferred_max_threads: u16,
    },

    /// Equivalent to [`Parallel`](Self::Parallel).
    #[deprecated(since = "0.1.19", note = "use ThreadingPolicy::Parallel")]
    Balanced,

    /// Equivalent to [`Parallel`](Self::Parallel).
    #[deprecated(since = "0.1.19", note = "use ThreadingPolicy::Parallel")]
    Unlimited,

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

    /// Resolve this policy to a concrete thread count for native-threaded
    /// codecs (rav1e/ravif, dav1d/rav1d, libwebp, etc.).
    ///
    /// Returns:
    /// - `1` for [`Sequential`](Self::Sequential) (and deprecated
    ///   [`SingleThread`](Self::SingleThread), `LimitOrSingle { max_threads: 1 }`).
    /// - `0` for [`Parallel`](Self::Parallel) and every other variant, meaning
    ///   "auto — let the library pick based on available parallelism." This is
    ///   the safe default for the `#[non_exhaustive]` deprecated arms; the
    ///   construction sites emit warnings and should be fixed there.
    ///
    /// This is the cross-codec shared helper for translating a
    /// [`ThreadingPolicy`] to the integer thread count that native-threaded
    /// encoder libraries accept. Rayon-based codecs (zenjpeg, jxl-encoder,
    /// zenjxl-decoder, zenpng) should use [`is_parallel()`](Self::is_parallel)
    /// instead — they control thread count via the ambient rayon pool.
    ///
    /// # Example
    ///
    /// ```
    /// use zencodec::ThreadingPolicy;
    /// assert_eq!(ThreadingPolicy::Sequential.resolve_thread_count(), 1);
    /// assert_eq!(ThreadingPolicy::Parallel.resolve_thread_count(), 0);
    /// ```
    pub fn resolve_thread_count(self) -> u32 {
        match self {
            Self::Sequential | Self::SingleThread => 1,
            Self::LimitOrSingle { max_threads } if max_threads <= 1 => 1,
            _ => 0,
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
///     .with_max_pixels(120_000_000) // admits ~108 MP photos
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
    /// Maximum peak working/output memory in bytes.
    ///
    /// **Enforcement strength varies by codec.** The
    /// [`enforces_max_memory`](crate::decode::DecodeCapabilities::enforces_max_memory)
    /// capability reports whether a codec enforces this limit *at all* — not how
    /// precisely. Two models exist across the zen codecs:
    ///
    /// - **Live cumulative tracking** — every significant allocation is charged
    ///   against a running budget (and released on drop), so the codec rejects
    ///   the instant the *actual* in-flight total would cross the cap, catching
    ///   memory that accumulates across passes and intermediate buffers.
    ///   Byte-accurate, but only a minority of codecs implement it.
    /// - **Pre-flight estimate** — the codec predicts peak memory from the image
    ///   header and rejects *before* allocating. There is no running total, so it
    ///   is only as accurate as that estimate: an under-counted working set can
    ///   still over-allocate, and transient peaks are not observed. This is what
    ///   most codecs do.
    ///
    /// A codec whose `enforces_max_memory` capability is `false` does **not**
    /// guard this limit at all — bound such inputs with
    /// [`max_pixels`](Self::max_pixels), [`max_width`](Self::max_width),
    /// [`max_height`](Self::max_height), or
    /// [`max_input_bytes`](Self::max_input_bytes), which are checked from the
    /// header before any allocation.
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
    /// Caller preference for allocation fallibility on buffers sized from
    /// untrusted input — see [`AllocPreference`].
    ///
    /// Defaults to [`AllocPreference::CodecDefault`] (each codec chooses:
    /// decoders favour `Fallible` on untrusted input, encoders/trusted paths
    /// favour the faster infallible `vec!`). Override with
    /// [`with_prefer_fallible_allocations`](Self::with_prefer_fallible_allocations).
    pub prefer_fallible_allocations: AllocPreference,
}

// All primitives, no pointers — but Option<u64> niche optimization and
// enum discriminant alignment can differ between 32-bit and 64-bit.
#[cfg(target_pointer_width = "64")]
const _: () = assert!(core::mem::size_of::<ResourceLimits>() == 128);

/// Caller preference for how a codec sizes buffers from untrusted input: the
/// fallible `try_reserve` path (graceful out-of-memory error) or the faster
/// infallible `vec!` / `Vec::with_capacity` path (aborts on OOM).
///
/// `try_reserve` is slower — LLVM lowers `vec![0u8; n]` to a single `calloc`,
/// so the infallible path is the fast default for trusted sizes. This is a
/// *preference*: a codec honours it where it controls the allocation and falls
/// back where it physically cannot (e.g. a transitive allocation it does not
/// own). Carried on [`ResourceLimits::prefer_fallible_allocations`] so the
/// policy travels with the rest of the resource governance the codec already
/// threads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum AllocPreference {
    /// Let the codec decide. Decoders favour [`Fallible`](Self::Fallible) for
    /// untrusted input; encoders and trusted paths favour
    /// [`Infallible`](Self::Infallible). Default — preserves existing behaviour.
    #[default]
    CodecDefault,
    /// Force the fallible path: `try_reserve`, returning a graceful
    /// out-of-memory error instead of aborting. Prefer for untrusted input.
    Fallible,
    /// Force the infallible path: `vec!` / `Vec::with_capacity` (faster — a
    /// single `calloc` for the zeroed case) at the cost of aborting on OOM.
    /// Prefer for trusted sizes and benchmarks.
    Infallible,
}

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
            prefer_fallible_allocations: AllocPreference::CodecDefault,
        }
    }
}

impl ResourceLimits {
    /// No limits (all fields `None`), parallel threading (ambient pool).
    pub fn none() -> Self {
        Self::default()
    }

    /// Safer default caps for processing **untrusted input**.
    ///
    /// [`ResourceLimits::default()`] has every field `None` (no limits) for
    /// backwards compatibility — that is fine for trusted, controlled input
    /// but is **resource-management DoS by default** when feeding bytes
    /// from the network or end users. Prefer this helper when limits are
    /// not explicitly tuned by the caller.
    ///
    /// Caps applied (chosen conservatively for typical web image workloads):
    /// - `max_pixels`: 120 MP per frame (admits ~108 MP photos, e.g. 12000 × 9000)
    /// - `max_total_pixels`: 200 MP across all frames of an animation
    /// - `max_width` / `max_height`: 16384 each (typical decoder hardware ceiling)
    /// - `max_memory_bytes`: 1 GiB
    /// - `max_input_bytes`: 256 MiB
    /// - `max_frames`: 65 536
    /// - `max_animation_ms`: 1 hour
    ///
    /// Threading is left at the default ([`ThreadingPolicy::Parallel`]).
    ///
    /// These are intentionally **generous** — large enough that legitimate
    /// inputs are not rejected, small enough that an adversarial input
    /// cannot consume the whole machine. Tighten further for your specific
    /// workload (e.g. a thumbnail server may want `max_pixels = 4_000_000`).
    ///
    /// # Example
    ///
    /// ```
    /// use zencodec::ResourceLimits;
    ///
    /// // Recommended starting point for a public image-decode service.
    /// let limits = ResourceLimits::for_untrusted_input();
    /// assert!(limits.max_pixels.is_some());
    /// assert!(limits.max_input_bytes.is_some());
    /// ```
    pub fn for_untrusted_input() -> Self {
        Self {
            max_pixels: Some(120_000_000),
            max_total_pixels: Some(200_000_000),
            max_width: Some(16384),
            max_height: Some(16384),
            max_memory_bytes: Some(1024 * 1024 * 1024),
            max_input_bytes: Some(256 * 1024 * 1024),
            max_output_bytes: None,
            max_frames: Some(65_536),
            max_animation_ms: Some(60 * 60 * 1000),
            threading: ThreadingPolicy::Parallel,
            prefer_fallible_allocations: AllocPreference::Fallible,
        }
    }

    /// Alias for [`for_untrusted_input`](Self::for_untrusted_input).
    ///
    /// Provided for callers who prefer the `safe_default` naming convention
    /// (mirrors the pattern used in some other crates).
    pub fn safe_default() -> Self {
        Self::for_untrusted_input()
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

    /// Set the allocation-fallibility preference (see [`AllocPreference`]).
    ///
    /// `CodecDefault` lets each codec choose; `Fallible` forces `try_reserve`
    /// (graceful OOM on untrusted input); `Infallible` forces the faster
    /// `vec!` path. Default is [`AllocPreference::CodecDefault`].
    pub fn with_prefer_fallible_allocations(mut self, pref: AllocPreference) -> Self {
        self.prefer_fallible_allocations = pref;
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
    fn alloc_preference_defaults_and_builder() {
        // Default / none() preserves existing behaviour: let the codec choose.
        assert_eq!(AllocPreference::default(), AllocPreference::CodecDefault);
        assert_eq!(
            ResourceLimits::default().prefer_fallible_allocations,
            AllocPreference::CodecDefault
        );
        // The untrusted preset opts into graceful (fallible) allocation.
        assert_eq!(
            ResourceLimits::for_untrusted_input().prefer_fallible_allocations,
            AllocPreference::Fallible
        );
        // Builder overrides either way.
        let l =
            ResourceLimits::none().with_prefer_fallible_allocations(AllocPreference::Infallible);
        assert_eq!(l.prefer_fallible_allocations, AllocPreference::Infallible);
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

    #[test]
    fn resolve_thread_count_sequential_is_1() {
        assert_eq!(ThreadingPolicy::Sequential.resolve_thread_count(), 1);
    }

    #[test]
    fn resolve_thread_count_parallel_is_0() {
        assert_eq!(ThreadingPolicy::Parallel.resolve_thread_count(), 0);
    }

    #[allow(deprecated)]
    #[test]
    fn resolve_thread_count_legacy_arms_default_auto() {
        // SingleThread → 1 (alias for Sequential)
        assert_eq!(ThreadingPolicy::SingleThread.resolve_thread_count(), 1);
        // LimitOrSingle { 1 } → 1 (effective single-thread)
        assert_eq!(
            ThreadingPolicy::LimitOrSingle { max_threads: 1 }.resolve_thread_count(),
            1
        );
        // LimitOrSingle { >1 } → 0 (auto)
        assert_eq!(
            ThreadingPolicy::LimitOrSingle { max_threads: 8 }.resolve_thread_count(),
            0
        );
        // LimitOrAny → 0 (auto)
        assert_eq!(
            ThreadingPolicy::LimitOrAny {
                preferred_max_threads: 16
            }
            .resolve_thread_count(),
            0
        );
        // Balanced / Unlimited → 0 (auto)
        assert_eq!(ThreadingPolicy::Balanced.resolve_thread_count(), 0);
        assert_eq!(ThreadingPolicy::Unlimited.resolve_thread_count(), 0);
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
    fn for_untrusted_input_has_caps() {
        let limits = ResourceLimits::for_untrusted_input();
        assert!(limits.has_any());
        assert!(limits.max_pixels.is_some());
        assert!(limits.max_total_pixels.is_some());
        assert!(limits.max_width.is_some());
        assert!(limits.max_height.is_some());
        assert!(limits.max_memory_bytes.is_some());
        assert!(limits.max_input_bytes.is_some());
        assert!(limits.max_frames.is_some());
        assert!(limits.max_animation_ms.is_some());
    }

    #[test]
    fn for_untrusted_input_rejects_oversized_image() {
        use crate::{ImageFormat, ImageInfo};
        let limits = ResourceLimits::for_untrusted_input();
        // 30000×30000 = 900 MP, far above the 100 MP per-frame cap.
        let info = ImageInfo::new(30000, 30000, ImageFormat::Jpeg);
        let err = limits.check_image_info(&info).unwrap_err();
        // Width is the first cap we trip (16384 < 30000).
        assert!(matches!(err, LimitExceeded::Width { .. }));

        // Smaller width but still huge pixel count (130 MP > the 120 MP cap).
        let info = ImageInfo::new(10000, 13000, ImageFormat::Jpeg);
        let err = limits.check_image_info(&info).unwrap_err();
        assert!(matches!(err, LimitExceeded::Pixels { .. }));
    }

    #[test]
    fn for_untrusted_input_accepts_typical_image() {
        use crate::{ImageFormat, ImageInfo};
        let limits = ResourceLimits::for_untrusted_input();
        // 4K image — should pass.
        let info = ImageInfo::new(3840, 2160, ImageFormat::Jpeg);
        assert!(limits.check_image_info(&info).is_ok());
        // 12 MP photo — should pass.
        let info = ImageInfo::new(4000, 3000, ImageFormat::Jpeg);
        assert!(limits.check_image_info(&info).is_ok());
    }

    #[test]
    fn for_untrusted_input_rejects_oversized_input() {
        let limits = ResourceLimits::for_untrusted_input();
        // 1 GiB input is definitely too big.
        assert!(limits.check_input_size(1024 * 1024 * 1024).is_err());
        // 16 MiB input is fine.
        assert!(limits.check_input_size(16 * 1024 * 1024).is_ok());
    }

    #[test]
    fn safe_default_alias_matches_for_untrusted_input() {
        assert_eq!(
            ResourceLimits::safe_default(),
            ResourceLimits::for_untrusted_input()
        );
    }

    #[test]
    fn default_remains_no_limits_for_backwards_compat() {
        // Per the crate's stability guarantee, ResourceLimits::default()
        // continues to mean "no limits" — switching to safer caps is
        // opt-in via for_untrusted_input().
        let limits = ResourceLimits::default();
        assert!(!limits.has_any());
        assert!(limits.max_pixels.is_none());
        assert!(limits.max_input_bytes.is_none());
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
