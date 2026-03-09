//! Resource limits for codec operations.
//!
//! [`ResourceLimits`] defines caps on resource usage. [`LimitExceeded`]
//! is returned when a check fails. Use the `check_*` methods for
//! parse-time rejection (fastest — reject before any pixel work).

/// Threading policy for codec operations.
///
/// Controls how many threads a codec may use. Codecs report their
/// supported range via
/// [`EncodeCapabilities::threads_supported_range()`](crate::EncodeCapabilities::threads_supported_range)
/// and [`DecodeCapabilities::threads_supported_range()`](crate::DecodeCapabilities::threads_supported_range).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum ThreadingPolicy {
    /// Force single-threaded operation.
    ///
    /// Useful for deterministic output or constrained environments.
    SingleThread,

    /// Use at most `max_threads` threads. If the codec would need more,
    /// fall back to single-threaded.
    LimitOrSingle {
        /// Maximum thread count before falling back to single-threaded.
        max_threads: u16,
    },

    /// Prefer at most `preferred_max_threads` threads, but the codec
    /// may use more if it needs to.
    LimitOrAny {
        /// Preferred maximum thread count (advisory, not enforced).
        preferred_max_threads: u16,
    },

    /// Let the codec pick a reasonable thread count based on available
    /// parallelism (typically half of available cores or similar).
    Balanced,

    /// No thread limit. Use as many threads as the codec wants.
    #[default]
    Unlimited,
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
/// use zc::ResourceLimits;
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
///
/// // Cost-aware rejection (after computing decode cost)
/// let cost = job.estimated_cost(data)?;
/// limits.check_decode_cost(&cost)?;
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ResourceLimits {
    /// Maximum total pixels (width × height).
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
    /// Threading policy for the codec.
    ///
    /// Defaults to [`ThreadingPolicy::Unlimited`].
    pub threading: ThreadingPolicy,
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
            threading: ThreadingPolicy::Unlimited,
        }
    }
}

impl ResourceLimits {
    /// No limits (all fields `None`), unlimited threading.
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
            || self.threading != ThreadingPolicy::Unlimited
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

    /// Check [`ImageInfo`](crate::ImageInfo) from `probe_header()` against all
    /// applicable limits. This is the fastest rejection point — call it
    /// immediately after probing, before any pixel work.
    ///
    /// Checks: `max_width`, `max_height`, `max_pixels`, `max_frames`.
    pub fn check_image_info(&self, info: &crate::ImageInfo) -> Result<(), LimitExceeded> {
        self.check_dimensions(info.width, info.height)?;
        if let Some(max) = self.max_frames
            && let Some(count) = info.frame_count
            && count > max
        {
            return Err(LimitExceeded::Frames { actual: count, max });
        }
        Ok(())
    }

    /// Check [`OutputInfo`](crate::decode::OutputInfo) against dimension limits.
    ///
    /// Checks: `max_width`, `max_height`, `max_pixels`.
    pub fn check_output_info(&self, info: &crate::OutputInfo) -> Result<(), LimitExceeded> {
        self.check_dimensions(info.width, info.height)
    }

    /// Check [`DecodeCost`](crate::decode::DecodeCost) against all applicable limits.
    ///
    /// Checks: `max_pixels` against `pixel_count`, `max_memory_bytes` against
    /// `peak_memory` (falls back to `output_bytes` if peak is unknown).
    pub fn check_decode_cost(&self, cost: &crate::DecodeCost) -> Result<(), LimitExceeded> {
        if let Some(max) = self.max_pixels
            && cost.pixel_count > max
        {
            return Err(LimitExceeded::Pixels {
                actual: cost.pixel_count,
                max,
            });
        }
        if let Some(max) = self.max_memory_bytes {
            let memory = cost.peak_memory.unwrap_or(cost.output_bytes);
            if memory > max {
                return Err(LimitExceeded::Memory {
                    actual: memory,
                    max,
                });
            }
        }
        Ok(())
    }

    /// Check [`EncodeCost`](crate::encode::EncodeCost) against all applicable limits.
    ///
    /// Checks: `max_pixels` against `pixel_count`, `max_memory_bytes` against
    /// `peak_memory` (falls back to `input_bytes` if peak is unknown).
    pub fn check_encode_cost(&self, cost: &crate::EncodeCost) -> Result<(), LimitExceeded> {
        if let Some(max) = self.max_pixels
            && cost.pixel_count > max
        {
            return Err(LimitExceeded::Pixels {
                actual: cost.pixel_count,
                max,
            });
        }
        if let Some(max) = self.max_memory_bytes {
            let memory = cost.peak_memory.unwrap_or(cost.input_bytes);
            if memory > max {
                return Err(LimitExceeded::Memory {
                    actual: memory,
                    max,
                });
            }
        }
        Ok(())
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
        assert_eq!(limits.threading(), ThreadingPolicy::Unlimited);
        assert!(!limits.has_any());
    }

    #[test]
    fn threading_policy_single_thread() {
        let limits = ResourceLimits::none().with_threading(ThreadingPolicy::SingleThread);
        assert!(limits.has_any());
        assert_eq!(limits.threading(), ThreadingPolicy::SingleThread);
    }

    #[test]
    fn threading_policy_limit_or_single() {
        let limits = ResourceLimits::none()
            .with_threading(ThreadingPolicy::LimitOrSingle { max_threads: 4 });
        assert!(limits.has_any());
        assert_eq!(
            limits.threading(),
            ThreadingPolicy::LimitOrSingle { max_threads: 4 }
        );
    }

    #[test]
    fn threading_policy_balanced() {
        let limits = ResourceLimits::none().with_threading(ThreadingPolicy::Balanced);
        assert!(limits.has_any());
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

        let info = ImageInfo::new(3840, 2160, ImageFormat::Avif).with_frame_count(50);
        assert!(limits.check_image_info(&info).is_ok());

        let big = ImageInfo::new(5000, 4000, ImageFormat::Jpeg);
        let err = limits.check_image_info(&big).unwrap_err();
        assert!(matches!(err, LimitExceeded::Width { .. }));

        let many_frames = ImageInfo::new(100, 100, ImageFormat::Gif).with_frame_count(200);
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
    fn check_decode_cost_with_peak_memory() {
        use crate::DecodeCost;
        let limits = ResourceLimits::none()
            .with_max_pixels(10_000_000)
            .with_max_memory(100 * 1024 * 1024);

        let cost = DecodeCost {
            output_bytes: 30 * 1024 * 1024,
            pixel_count: 8_000_000,
            peak_memory: Some(90 * 1024 * 1024),
        };
        assert!(limits.check_decode_cost(&cost).is_ok());

        let expensive = DecodeCost {
            output_bytes: 30 * 1024 * 1024,
            pixel_count: 8_000_000,
            peak_memory: Some(200 * 1024 * 1024),
        };
        let err = limits.check_decode_cost(&expensive).unwrap_err();
        assert!(matches!(err, LimitExceeded::Memory { .. }));
    }

    #[test]
    fn check_decode_cost_falls_back_to_output_bytes() {
        use crate::DecodeCost;
        let limits = ResourceLimits::none().with_max_memory(50 * 1024 * 1024);

        let cost = DecodeCost {
            output_bytes: 100 * 1024 * 1024,
            pixel_count: 25_000_000,
            peak_memory: None,
        };
        let err = limits.check_decode_cost(&cost).unwrap_err();
        assert!(matches!(err, LimitExceeded::Memory { actual, .. } if actual == 100 * 1024 * 1024));
    }

    #[test]
    fn check_encode_cost_with_peak_memory() {
        use crate::EncodeCost;
        let limits = ResourceLimits::none().with_max_memory(256 * 1024 * 1024);

        let cost = EncodeCost {
            input_bytes: 30 * 1024 * 1024,
            pixel_count: 8_000_000,
            peak_memory: Some(200 * 1024 * 1024),
        };
        assert!(limits.check_encode_cost(&cost).is_ok());

        let expensive = EncodeCost {
            input_bytes: 30 * 1024 * 1024,
            pixel_count: 8_000_000,
            peak_memory: Some(500 * 1024 * 1024),
        };
        let err = limits.check_encode_cost(&expensive).unwrap_err();
        assert!(matches!(err, LimitExceeded::Memory { .. }));
    }

    #[test]
    fn check_encode_cost_falls_back_to_input_bytes() {
        use crate::EncodeCost;
        let limits = ResourceLimits::none().with_max_memory(10 * 1024 * 1024);

        let cost = EncodeCost {
            input_bytes: 30 * 1024 * 1024,
            pixel_count: 8_000_000,
            peak_memory: None,
        };
        let err = limits.check_encode_cost(&cost).unwrap_err();
        assert!(matches!(err, LimitExceeded::Memory { actual, .. } if actual == 30 * 1024 * 1024));
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
}
