//! Codec capability descriptors.
//!
//! Encoders return [`EncodeCapabilities`] and decoders return
//! [`DecodeCapabilities`] describing what they support. This lets callers
//! discover behavior before calling methods that might be no-ops or expensive.
//!
//! [`UnsupportedOperation`] provides a standard enum for codecs to report
//! which operations they don't support. Use [`CodecErrorExt`](crate::CodecErrorExt)
//! to find these in any error's source chain.

use core::fmt;

/// Identifies an operation that a codec does not support.
///
/// Codecs include this in their error types (e.g. as a variant payload)
/// so callers can generically detect "this codec doesn't support this
/// operation" without downcasting. See [`CodecErrorExt`](crate::CodecErrorExt).
///
/// # Example
///
/// ```
/// use zc::UnsupportedOperation;
///
/// let op = UnsupportedOperation::DecodeInto;
/// assert_eq!(format!("{op}"), "unsupported operation: decode_into");
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum UnsupportedOperation {
    /// `Encoder::push_rows()` + `finish()` (row-level encode).
    RowLevelEncode,
    /// `Encoder::encode_from()` (pull-from-source encode).
    PullEncode,
    /// All `FrameEncoder` methods (animation encoding).
    AnimationEncode,
    /// `FrameEncoder::begin_frame` / `push_rows` / `end_frame` (row-level frame encode).
    RowLevelFrameEncode,
    /// `FrameEncoder::pull_frame()` (pull-from-source frame encode).
    PullFrameEncode,
    /// `Decoder::decode_into()` (decode into caller buffer).
    DecodeInto,
    /// `Decoder::decode_rows()` (row-level decode).
    RowLevelDecode,
    /// All `FrameDecoder` methods (animation decoding).
    AnimationDecode,
    /// `FrameDecoder::next_frame_into()` (frame decode into caller buffer).
    FrameDecodeInto,
    /// `FrameDecoder::next_frame_rows()` (row-level frame decode).
    RowLevelFrameDecode,
    /// A specific pixel format is not supported.
    PixelFormat,
}

impl UnsupportedOperation {
    /// Short name for the operation (suitable for error messages).
    pub const fn name(self) -> &'static str {
        match self {
            Self::RowLevelEncode => "row_level_encode",
            Self::PullEncode => "pull_encode",
            Self::AnimationEncode => "animation_encode",
            Self::RowLevelFrameEncode => "row_level_frame_encode",
            Self::PullFrameEncode => "pull_frame_encode",
            Self::DecodeInto => "decode_into",
            Self::RowLevelDecode => "row_level_decode",
            Self::AnimationDecode => "animation_decode",
            Self::FrameDecodeInto => "frame_decode_into",
            Self::RowLevelFrameDecode => "row_level_frame_decode",
            Self::PixelFormat => "pixel_format",
        }
    }
}

impl fmt::Display for UnsupportedOperation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unsupported operation: {}", self.name())
    }
}

impl core::error::Error for UnsupportedOperation {}


// ===========================================================================
// EncodeCapabilities
// ===========================================================================

/// Describes what an encoder supports.
///
/// Returned by [`EncoderConfig::capabilities()`](crate::encode::EncoderConfig::capabilities)
/// as a `&'static` reference. Uses getter methods so fields can be added
/// without breaking changes.
///
/// # Example
///
/// ```
/// use zc::encode::EncodeCapabilities;
///
/// static CAPS: EncodeCapabilities = EncodeCapabilities::new()
///     .with_icc(true)
///     .with_exif(true)
///     .with_xmp(true)
///     .with_cancel(true)
///     .with_native_gray(true);
///
/// assert!(CAPS.icc());
/// assert!(CAPS.native_gray());
/// ```
#[non_exhaustive]
pub struct EncodeCapabilities {
    // Metadata embedding
    icc: bool,
    exif: bool,
    xmp: bool,
    cicp: bool,
    // Operation support
    cancel: bool,
    animation: bool,
    row_level: bool,
    pull: bool,
    row_level_frame: bool,
    pull_frame: bool,
    // Format capabilities
    lossy: bool,
    lossless: bool,
    hdr: bool,
    native_gray: bool,
    native_16bit: bool,
    native_f32: bool,
    native_alpha: bool,
    // Limit enforcement
    enforces_max_pixels: bool,
    enforces_max_memory: bool,
    // Tuning ranges
    effort_range: Option<[i32; 2]>,
    quality_range: Option<[f32; 2]>,
    // Threading
    threads_supported_range: (u16, u16),
}

impl Default for EncodeCapabilities {
    fn default() -> Self {
        Self::new()
    }
}

impl EncodeCapabilities {
    /// Empty capabilities (everything disabled, single-threaded).
    pub const EMPTY: Self = Self::new();

    /// Create capabilities with everything disabled.
    pub const fn new() -> Self {
        Self {
            icc: false,
            exif: false,
            xmp: false,
            cicp: false,
            cancel: false,
            animation: false,
            row_level: false,
            pull: false,
            row_level_frame: false,
            pull_frame: false,
            lossy: false,
            lossless: false,
            hdr: false,
            native_gray: false,
            native_16bit: false,
            native_f32: false,
            native_alpha: false,
            enforces_max_pixels: false,
            enforces_max_memory: false,
            effort_range: None,
            quality_range: None,
            threads_supported_range: (1, 1),
        }
    }

    // --- Getters ---

    /// Whether the encoder embeds ICC color profiles from `with_metadata`.
    pub const fn icc(&self) -> bool {
        self.icc
    }
    /// Whether the encoder embeds EXIF data from `with_metadata`.
    pub const fn exif(&self) -> bool {
        self.exif
    }
    /// Whether the encoder embeds XMP data from `with_metadata`.
    pub const fn xmp(&self) -> bool {
        self.xmp
    }
    /// Whether the encoder embeds CICP color description from `with_metadata`.
    pub const fn cicp(&self) -> bool {
        self.cicp
    }
    /// Whether `with_stop` on encode jobs is respected (not a no-op).
    pub const fn cancel(&self) -> bool {
        self.cancel
    }
    /// Whether the codec supports encoding animation (multiple frames).
    pub const fn animation(&self) -> bool {
        self.animation
    }
    /// Whether `push_rows()` / `finish()` work (row-level encode).
    pub const fn row_level(&self) -> bool {
        self.row_level
    }
    /// Whether `encode_from()` works (pull-from-source encode).
    pub const fn pull(&self) -> bool {
        self.pull
    }
    /// Whether `FrameEncoder::begin_frame` / `push_rows` / `end_frame` work.
    pub const fn row_level_frame(&self) -> bool {
        self.row_level_frame
    }
    /// Whether `pull_frame()` works (pull-from-source frame encode).
    pub const fn pull_frame(&self) -> bool {
        self.pull_frame
    }
    /// Whether the codec supports lossy encoding.
    pub const fn lossy(&self) -> bool {
        self.lossy
    }
    /// Whether the codec supports mathematically lossless encoding.
    pub const fn lossless(&self) -> bool {
        self.lossless
    }
    /// Whether the codec supports HDR content.
    pub const fn hdr(&self) -> bool {
        self.hdr
    }
    /// Whether the codec supports grayscale natively.
    pub const fn native_gray(&self) -> bool {
        self.native_gray
    }
    /// Whether the codec supports 16-bit per channel natively.
    pub const fn native_16bit(&self) -> bool {
        self.native_16bit
    }
    /// Whether the codec handles f32 pixel data natively.
    pub const fn native_f32(&self) -> bool {
        self.native_f32
    }
    /// Whether the codec handles alpha channel natively.
    pub const fn native_alpha(&self) -> bool {
        self.native_alpha
    }
    /// Whether the codec enforces `max_pixels` limits.
    pub const fn enforces_max_pixels(&self) -> bool {
        self.enforces_max_pixels
    }
    /// Whether the codec enforces `max_memory_bytes` limits.
    pub const fn enforces_max_memory(&self) -> bool {
        self.enforces_max_memory
    }

    /// Meaningful effort range `[min, max]`.
    ///
    /// `None` means the codec has no effort tuning.
    pub const fn effort_range(&self) -> Option<[i32; 2]> {
        self.effort_range
    }

    /// Meaningful quality range `[min, max]` on the calibrated 0.0–100.0 scale.
    ///
    /// `None` means the codec is lossless-only.
    pub const fn quality_range(&self) -> Option<[f32; 2]> {
        self.quality_range
    }

    /// Supported thread count range `(min, max)`.
    ///
    /// `(1, 1)` means single-threaded only.
    /// `(1, 16)` means the encoder can use 1 to 16 threads.
    pub const fn threads_supported_range(&self) -> (u16, u16) {
        self.threads_supported_range
    }

    // --- Const builder methods ---

    pub const fn with_icc(mut self, v: bool) -> Self {
        self.icc = v;
        self
    }
    pub const fn with_exif(mut self, v: bool) -> Self {
        self.exif = v;
        self
    }
    pub const fn with_xmp(mut self, v: bool) -> Self {
        self.xmp = v;
        self
    }
    pub const fn with_cicp(mut self, v: bool) -> Self {
        self.cicp = v;
        self
    }
    pub const fn with_cancel(mut self, v: bool) -> Self {
        self.cancel = v;
        self
    }
    pub const fn with_animation(mut self, v: bool) -> Self {
        self.animation = v;
        self
    }
    pub const fn with_row_level(mut self, v: bool) -> Self {
        self.row_level = v;
        self
    }
    pub const fn with_pull(mut self, v: bool) -> Self {
        self.pull = v;
        self
    }
    pub const fn with_row_level_frame(mut self, v: bool) -> Self {
        self.row_level_frame = v;
        self
    }
    pub const fn with_pull_frame(mut self, v: bool) -> Self {
        self.pull_frame = v;
        self
    }
    pub const fn with_lossy(mut self, v: bool) -> Self {
        self.lossy = v;
        self
    }
    pub const fn with_lossless(mut self, v: bool) -> Self {
        self.lossless = v;
        self
    }
    pub const fn with_hdr(mut self, v: bool) -> Self {
        self.hdr = v;
        self
    }
    pub const fn with_native_gray(mut self, v: bool) -> Self {
        self.native_gray = v;
        self
    }
    pub const fn with_native_16bit(mut self, v: bool) -> Self {
        self.native_16bit = v;
        self
    }
    pub const fn with_native_f32(mut self, v: bool) -> Self {
        self.native_f32 = v;
        self
    }
    pub const fn with_native_alpha(mut self, v: bool) -> Self {
        self.native_alpha = v;
        self
    }
    pub const fn with_enforces_max_pixels(mut self, v: bool) -> Self {
        self.enforces_max_pixels = v;
        self
    }
    pub const fn with_enforces_max_memory(mut self, v: bool) -> Self {
        self.enforces_max_memory = v;
        self
    }

    /// Set the meaningful effort range `[min, max]`.
    pub const fn with_effort_range(mut self, min: i32, max: i32) -> Self {
        self.effort_range = Some([min, max]);
        self
    }

    /// Set the meaningful quality range `[min, max]`.
    pub const fn with_quality_range(mut self, min: f32, max: f32) -> Self {
        self.quality_range = Some([min, max]);
        self
    }

    /// Set supported thread count range.
    pub const fn with_threads_supported_range(mut self, min: u16, max: u16) -> Self {
        self.threads_supported_range = (min, max);
        self
    }
}

impl fmt::Debug for EncodeCapabilities {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = f.debug_struct("EncodeCapabilities");
        s.field("icc", &self.icc)
            .field("exif", &self.exif)
            .field("xmp", &self.xmp)
            .field("cicp", &self.cicp)
            .field("cancel", &self.cancel)
            .field("animation", &self.animation)
            .field("lossy", &self.lossy)
            .field("lossless", &self.lossless)
            .field("hdr", &self.hdr)
            .field("native_gray", &self.native_gray)
            .field("native_16bit", &self.native_16bit)
            .field("native_f32", &self.native_f32)
            .field("native_alpha", &self.native_alpha)
            .field("row_level", &self.row_level)
            .field("pull", &self.pull)
            .field("row_level_frame", &self.row_level_frame)
            .field("pull_frame", &self.pull_frame)
            .field("enforces_max_pixels", &self.enforces_max_pixels)
            .field("enforces_max_memory", &self.enforces_max_memory)
            .field("threads_supported_range", &self.threads_supported_range);
        if let Some(range) = &self.effort_range {
            s.field("effort_range", range);
        }
        if let Some(range) = &self.quality_range {
            s.field("quality_range", range);
        }
        s.finish()
    }
}

// ===========================================================================
// DecodeCapabilities
// ===========================================================================

/// Describes what a decoder supports.
///
/// Returned by [`DecoderConfig::capabilities()`](crate::decode::DecoderConfig::capabilities)
/// as a `&'static` reference. Uses getter methods so fields can be added
/// without breaking changes.
///
/// # Example
///
/// ```
/// use zc::decode::DecodeCapabilities;
///
/// static CAPS: DecodeCapabilities = DecodeCapabilities::new()
///     .with_icc(true)
///     .with_exif(true)
///     .with_cancel(true)
///     .with_cheap_probe(true);
///
/// assert!(CAPS.icc());
/// assert!(CAPS.cheap_probe());
/// ```
#[non_exhaustive]
pub struct DecodeCapabilities {
    // Metadata extraction
    icc: bool,
    exif: bool,
    xmp: bool,
    cicp: bool,
    // Operation support
    cancel: bool,
    animation: bool,
    cheap_probe: bool,
    decode_into: bool,
    row_level: bool,
    frame_decode_into: bool,
    row_level_frame: bool,
    // Format capabilities
    hdr: bool,
    native_gray: bool,
    native_16bit: bool,
    native_f32: bool,
    native_alpha: bool,
    // Limit enforcement
    enforces_max_pixels: bool,
    enforces_max_memory: bool,
    enforces_max_input_bytes: bool,
    // Threading
    threads_supported_range: (u16, u16),
}

impl Default for DecodeCapabilities {
    fn default() -> Self {
        Self::new()
    }
}

impl DecodeCapabilities {
    /// Empty capabilities (everything disabled, single-threaded).
    pub const EMPTY: Self = Self::new();

    /// Create capabilities with everything disabled.
    pub const fn new() -> Self {
        Self {
            icc: false,
            exif: false,
            xmp: false,
            cicp: false,
            cancel: false,
            animation: false,
            cheap_probe: false,
            decode_into: false,
            row_level: false,
            frame_decode_into: false,
            row_level_frame: false,
            hdr: false,
            native_gray: false,
            native_16bit: false,
            native_f32: false,
            native_alpha: false,
            enforces_max_pixels: false,
            enforces_max_memory: false,
            enforces_max_input_bytes: false,
            threads_supported_range: (1, 1),
        }
    }

    // --- Getters ---

    /// Whether the decoder extracts ICC color profiles into `ImageInfo`.
    pub const fn icc(&self) -> bool {
        self.icc
    }
    /// Whether the decoder extracts EXIF data into `ImageInfo`.
    pub const fn exif(&self) -> bool {
        self.exif
    }
    /// Whether the decoder extracts XMP data into `ImageInfo`.
    pub const fn xmp(&self) -> bool {
        self.xmp
    }
    /// Whether the decoder extracts CICP color description into `ImageInfo`.
    pub const fn cicp(&self) -> bool {
        self.cicp
    }
    /// Whether `with_stop` on decode jobs is respected (not a no-op).
    pub const fn cancel(&self) -> bool {
        self.cancel
    }
    /// Whether the codec supports decoding animation (multiple frames).
    pub const fn animation(&self) -> bool {
        self.animation
    }
    /// Whether `probe()` is cheap (header parse only, not a full decode).
    pub const fn cheap_probe(&self) -> bool {
        self.cheap_probe
    }
    /// Whether `decode_into()` is implemented.
    pub const fn decode_into(&self) -> bool {
        self.decode_into
    }
    /// Whether streaming row-level decode works.
    pub const fn row_level(&self) -> bool {
        self.row_level
    }
    /// Whether `next_frame_into()` works (frame decode into caller buffer).
    pub const fn frame_decode_into(&self) -> bool {
        self.frame_decode_into
    }
    /// Whether row-level frame decode works.
    pub const fn row_level_frame(&self) -> bool {
        self.row_level_frame
    }
    /// Whether the codec supports HDR content.
    pub const fn hdr(&self) -> bool {
        self.hdr
    }
    /// Whether the codec supports grayscale natively.
    pub const fn native_gray(&self) -> bool {
        self.native_gray
    }
    /// Whether the codec supports 16-bit per channel natively.
    pub const fn native_16bit(&self) -> bool {
        self.native_16bit
    }
    /// Whether the codec handles f32 pixel data natively.
    pub const fn native_f32(&self) -> bool {
        self.native_f32
    }
    /// Whether the codec handles alpha channel natively.
    pub const fn native_alpha(&self) -> bool {
        self.native_alpha
    }
    /// Whether the codec enforces `max_pixels` limits.
    pub const fn enforces_max_pixels(&self) -> bool {
        self.enforces_max_pixels
    }
    /// Whether the codec enforces `max_memory_bytes` limits.
    pub const fn enforces_max_memory(&self) -> bool {
        self.enforces_max_memory
    }
    /// Whether the codec enforces `max_input_bytes` limits.
    pub const fn enforces_max_input_bytes(&self) -> bool {
        self.enforces_max_input_bytes
    }

    /// Supported thread count range `(min, max)`.
    ///
    /// `(1, 1)` means single-threaded only.
    /// `(1, 8)` means the decoder can use 1 to 8 threads.
    pub const fn threads_supported_range(&self) -> (u16, u16) {
        self.threads_supported_range
    }

    // --- Const builder methods ---

    pub const fn with_icc(mut self, v: bool) -> Self {
        self.icc = v;
        self
    }
    pub const fn with_exif(mut self, v: bool) -> Self {
        self.exif = v;
        self
    }
    pub const fn with_xmp(mut self, v: bool) -> Self {
        self.xmp = v;
        self
    }
    pub const fn with_cicp(mut self, v: bool) -> Self {
        self.cicp = v;
        self
    }
    pub const fn with_cancel(mut self, v: bool) -> Self {
        self.cancel = v;
        self
    }
    pub const fn with_animation(mut self, v: bool) -> Self {
        self.animation = v;
        self
    }
    pub const fn with_cheap_probe(mut self, v: bool) -> Self {
        self.cheap_probe = v;
        self
    }
    pub const fn with_decode_into(mut self, v: bool) -> Self {
        self.decode_into = v;
        self
    }
    pub const fn with_row_level(mut self, v: bool) -> Self {
        self.row_level = v;
        self
    }
    pub const fn with_frame_decode_into(mut self, v: bool) -> Self {
        self.frame_decode_into = v;
        self
    }
    pub const fn with_row_level_frame(mut self, v: bool) -> Self {
        self.row_level_frame = v;
        self
    }
    pub const fn with_hdr(mut self, v: bool) -> Self {
        self.hdr = v;
        self
    }
    pub const fn with_native_gray(mut self, v: bool) -> Self {
        self.native_gray = v;
        self
    }
    pub const fn with_native_16bit(mut self, v: bool) -> Self {
        self.native_16bit = v;
        self
    }
    pub const fn with_native_f32(mut self, v: bool) -> Self {
        self.native_f32 = v;
        self
    }
    pub const fn with_native_alpha(mut self, v: bool) -> Self {
        self.native_alpha = v;
        self
    }
    pub const fn with_enforces_max_pixels(mut self, v: bool) -> Self {
        self.enforces_max_pixels = v;
        self
    }
    pub const fn with_enforces_max_memory(mut self, v: bool) -> Self {
        self.enforces_max_memory = v;
        self
    }
    pub const fn with_enforces_max_input_bytes(mut self, v: bool) -> Self {
        self.enforces_max_input_bytes = v;
        self
    }

    /// Set supported thread count range.
    pub const fn with_threads_supported_range(mut self, min: u16, max: u16) -> Self {
        self.threads_supported_range = (min, max);
        self
    }
}

impl fmt::Debug for DecodeCapabilities {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = f.debug_struct("DecodeCapabilities");
        s.field("icc", &self.icc)
            .field("exif", &self.exif)
            .field("xmp", &self.xmp)
            .field("cicp", &self.cicp)
            .field("cancel", &self.cancel)
            .field("animation", &self.animation)
            .field("cheap_probe", &self.cheap_probe)
            .field("decode_into", &self.decode_into)
            .field("row_level", &self.row_level)
            .field("frame_decode_into", &self.frame_decode_into)
            .field("row_level_frame", &self.row_level_frame)
            .field("hdr", &self.hdr)
            .field("native_gray", &self.native_gray)
            .field("native_16bit", &self.native_16bit)
            .field("native_f32", &self.native_f32)
            .field("native_alpha", &self.native_alpha)
            .field("enforces_max_pixels", &self.enforces_max_pixels)
            .field("enforces_max_memory", &self.enforces_max_memory)
            .field("enforces_max_input_bytes", &self.enforces_max_input_bytes)
            .field("threads_supported_range", &self.threads_supported_range);
        s.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_default_all_false() {
        let caps = EncodeCapabilities::new();
        assert!(!caps.icc());
        assert!(!caps.exif());
        assert!(!caps.xmp());
        assert!(!caps.cicp());
        assert!(!caps.cancel());
        assert!(!caps.animation());
        assert!(!caps.row_level());
        assert!(!caps.pull());
        assert!(!caps.row_level_frame());
        assert!(!caps.pull_frame());
        assert!(!caps.lossy());
        assert!(!caps.lossless());
        assert!(!caps.hdr());
        assert!(!caps.native_gray());
        assert!(!caps.native_16bit());
        assert!(!caps.native_f32());
        assert!(!caps.native_alpha());
        assert!(!caps.enforces_max_pixels());
        assert!(!caps.enforces_max_memory());
        assert!(caps.effort_range().is_none());
        assert!(caps.quality_range().is_none());
        assert_eq!(caps.threads_supported_range(), (1, 1));
    }

    #[test]
    fn decode_default_all_false() {
        let caps = DecodeCapabilities::new();
        assert!(!caps.icc());
        assert!(!caps.exif());
        assert!(!caps.xmp());
        assert!(!caps.cicp());
        assert!(!caps.cancel());
        assert!(!caps.animation());
        assert!(!caps.cheap_probe());
        assert!(!caps.decode_into());
        assert!(!caps.row_level());
        assert!(!caps.frame_decode_into());
        assert!(!caps.row_level_frame());
        assert!(!caps.hdr());
        assert!(!caps.native_gray());
        assert!(!caps.native_16bit());
        assert!(!caps.native_f32());
        assert!(!caps.native_alpha());
        assert!(!caps.enforces_max_pixels());
        assert!(!caps.enforces_max_memory());
        assert!(!caps.enforces_max_input_bytes());
        assert_eq!(caps.threads_supported_range(), (1, 1));
    }

    #[test]
    fn encode_builder() {
        let caps = EncodeCapabilities::new()
            .with_icc(true)
            .with_cancel(true)
            .with_native_gray(true)
            .with_animation(true)
            .with_native_16bit(true)
            .with_hdr(true)
            .with_threads_supported_range(1, 8);
        assert!(caps.icc());
        assert!(!caps.exif());
        assert!(caps.cancel());
        assert!(caps.native_gray());
        assert!(caps.animation());
        assert!(caps.native_16bit());
        assert!(!caps.lossless());
        assert!(caps.hdr());
        assert_eq!(caps.threads_supported_range(), (1, 8));
    }

    #[test]
    fn decode_builder() {
        let caps = DecodeCapabilities::new()
            .with_icc(true)
            .with_cheap_probe(true)
            .with_cancel(true)
            .with_animation(true)
            .with_enforces_max_input_bytes(true)
            .with_threads_supported_range(1, 4);
        assert!(caps.icc());
        assert!(caps.cheap_probe());
        assert!(caps.cancel());
        assert!(caps.animation());
        assert!(caps.enforces_max_input_bytes());
        assert_eq!(caps.threads_supported_range(), (1, 4));
    }

    #[test]
    fn encode_static_construction() {
        static CAPS: EncodeCapabilities = EncodeCapabilities::new()
            .with_icc(true)
            .with_exif(true)
            .with_xmp(true)
            .with_cancel(true)
            .with_animation(true)
            .with_lossless(true)
            .with_cicp(true)
            .with_effort_range(0, 100)
            .with_quality_range(0.0, 100.0)
            .with_threads_supported_range(1, 16);
        assert!(CAPS.icc());
        assert!(CAPS.cancel());
        assert!(!CAPS.native_gray());
        assert!(CAPS.animation());
        assert!(CAPS.lossless());
        assert!(CAPS.cicp());
        assert_eq!(CAPS.effort_range(), Some([0, 100]));
        assert_eq!(CAPS.quality_range(), Some([0.0, 100.0]));
        assert_eq!(CAPS.threads_supported_range(), (1, 16));
    }

    #[test]
    fn decode_static_construction() {
        static CAPS: DecodeCapabilities = DecodeCapabilities::new()
            .with_icc(true)
            .with_cheap_probe(true)
            .with_cancel(true)
            .with_animation(true)
            .with_enforces_max_pixels(true)
            .with_enforces_max_input_bytes(true);
        assert!(CAPS.icc());
        assert!(CAPS.cheap_probe());
        assert!(CAPS.enforces_max_pixels());
        assert!(!CAPS.enforces_max_memory());
        assert!(CAPS.enforces_max_input_bytes());
    }

    #[test]
    fn encode_effort_quality_ranges() {
        let caps = EncodeCapabilities::new()
            .with_effort_range(0, 10)
            .with_quality_range(0.0, 100.0);
        assert_eq!(caps.effort_range(), Some([0i32, 10]));
        assert_eq!(caps.quality_range(), Some([0.0, 100.0]));
    }

    #[test]
    fn unsupported_operation_display() {
        assert_eq!(
            alloc::format!("{}", UnsupportedOperation::RowLevelEncode),
            "unsupported operation: row_level_encode"
        );
        assert_eq!(
            alloc::format!("{}", UnsupportedOperation::DecodeInto),
            "unsupported operation: decode_into"
        );
        assert_eq!(
            alloc::format!("{}", UnsupportedOperation::PixelFormat),
            "unsupported operation: pixel_format"
        );
    }

    #[test]
    fn unsupported_operation_name() {
        assert_eq!(UnsupportedOperation::PullEncode.name(), "pull_encode");
        assert_eq!(
            UnsupportedOperation::AnimationEncode.name(),
            "animation_encode"
        );
        assert_eq!(
            UnsupportedOperation::RowLevelFrameEncode.name(),
            "row_level_frame_encode"
        );
        assert_eq!(
            UnsupportedOperation::PullFrameEncode.name(),
            "pull_frame_encode"
        );
        assert_eq!(
            UnsupportedOperation::RowLevelDecode.name(),
            "row_level_decode"
        );
        assert_eq!(
            UnsupportedOperation::AnimationDecode.name(),
            "animation_decode"
        );
        assert_eq!(
            UnsupportedOperation::FrameDecodeInto.name(),
            "frame_decode_into"
        );
        assert_eq!(
            UnsupportedOperation::RowLevelFrameDecode.name(),
            "row_level_frame_decode"
        );
    }

    #[test]
    fn unsupported_operation_is_error() {
        let op = UnsupportedOperation::DecodeInto;
        let err: &dyn core::error::Error = &op;
        assert!(err.source().is_none());
    }

    #[test]
    fn unsupported_operation_eq_hash() {
        assert_eq!(
            UnsupportedOperation::DecodeInto,
            UnsupportedOperation::DecodeInto
        );
        assert_ne!(
            UnsupportedOperation::DecodeInto,
            UnsupportedOperation::PullEncode
        );
    }

}
