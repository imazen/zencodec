//! Codec capability descriptors.
//!
//! Each codec returns a static [`CodecCapabilities`] describing what it
//! supports. This lets callers discover behavior before calling methods
//! that might be no-ops or expensive.
//!
//! [`UnsupportedOperation`] provides a standard enum for codecs to report
//! which operations they don't support. [`HasUnsupportedOperation`] lets
//! generic callers check for unsupported operations without downcasting
//! codec-specific error types.

use core::fmt;

/// Identifies an operation that a codec does not support.
///
/// Codecs include this in their error types (e.g. as a variant payload)
/// so callers can generically detect "this codec doesn't support this
/// operation" without downcasting. See [`HasUnsupportedOperation`].
///
/// # Example
///
/// ```
/// use zencodec_types::UnsupportedOperation;
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

/// Trait for codec errors that can report unsupported operations.
///
/// Implement this on your codec's error type so generic callers can
/// check `err.unsupported_operation()` without downcasting.
///
/// This is opt-in — not a trait bound on the associated `Error` type.
///
/// # Example
///
/// ```
/// use zencodec_types::{HasUnsupportedOperation, UnsupportedOperation};
///
/// #[derive(Debug)]
/// enum MyCodecError {
///     Unsupported(UnsupportedOperation),
///     Other,
/// }
///
/// impl HasUnsupportedOperation for MyCodecError {
///     fn unsupported_operation(&self) -> Option<UnsupportedOperation> {
///         match self {
///             MyCodecError::Unsupported(op) => Some(*op),
///             _ => None,
///         }
///     }
/// }
///
/// let err = MyCodecError::Unsupported(UnsupportedOperation::DecodeInto);
/// assert_eq!(err.unsupported_operation(), Some(UnsupportedOperation::DecodeInto));
///
/// let err2 = MyCodecError::Other;
/// assert_eq!(err2.unsupported_operation(), None);
/// ```
pub trait HasUnsupportedOperation {
    /// Returns the [`UnsupportedOperation`] if this error represents one,
    /// or `None` for other error kinds.
    fn unsupported_operation(&self) -> Option<UnsupportedOperation>;
}

/// Describes what a codec supports.
///
/// Returned by `EncoderConfig::capabilities()` and
/// `DecoderConfig::capabilities()` as a `&'static`
/// reference. The struct uses getter methods so fields can be added over time
/// without breaking changes.
///
/// # Example
///
/// ```
/// use zencodec_types::CodecCapabilities;
///
/// static CAPS: CodecCapabilities = CodecCapabilities::new()
///     .with_encode_icc(true)
///     .with_encode_exif(true)
///     .with_encode_xmp(true)
///     .with_encode_cancel(true)
///     .with_decode_cancel(true)
///     .with_native_gray(true)
///     .with_cheap_probe(true);
///
/// assert!(CAPS.encode_icc());
/// assert!(CAPS.cheap_probe());
/// ```
#[non_exhaustive]
pub struct CodecCapabilities {
    encode_icc: bool,
    encode_exif: bool,
    encode_xmp: bool,
    decode_icc: bool,
    decode_exif: bool,
    decode_xmp: bool,
    encode_cancel: bool,
    decode_cancel: bool,
    native_gray: bool,
    cheap_probe: bool,
    encode_animation: bool,
    decode_animation: bool,
    native_16bit: bool,
    lossless: bool,
    hdr: bool,
    encode_cicp: bool,
    decode_cicp: bool,
    enforces_max_pixels: bool,
    enforces_max_memory: bool,
    enforces_max_file_size: bool,
    lossy: bool,
    native_f32: bool,
    native_alpha: bool,
    decode_into: bool,
    row_level_encode: bool,
    pull_encode: bool,
    row_level_decode: bool,
    row_level_frame_encode: bool,
    pull_frame_encode: bool,
    frame_decode_into: bool,
    row_level_frame_decode: bool,
    /// Meaningful effort range `[min, max]`. `None` = no effort tuning.
    effort_range: Option<[i32; 2]>,
    /// Meaningful quality range `[min, max]` on the calibrated 0–100 scale.
    /// `None` = lossless-only codec (no quality parameter).
    quality_range: Option<[f32; 2]>,
}

impl Default for CodecCapabilities {
    fn default() -> Self {
        Self::new()
    }
}

impl CodecCapabilities {
    /// Create capabilities with everything disabled.
    pub const fn new() -> Self {
        Self {
            encode_icc: false,
            encode_exif: false,
            encode_xmp: false,
            decode_icc: false,
            decode_exif: false,
            decode_xmp: false,
            encode_cancel: false,
            decode_cancel: false,
            native_gray: false,
            cheap_probe: false,
            encode_animation: false,
            decode_animation: false,
            native_16bit: false,
            lossless: false,
            hdr: false,
            encode_cicp: false,
            decode_cicp: false,
            enforces_max_pixels: false,
            enforces_max_memory: false,
            enforces_max_file_size: false,
            lossy: false,
            native_f32: false,
            native_alpha: false,
            decode_into: false,
            row_level_encode: false,
            pull_encode: false,
            row_level_decode: false,
            row_level_frame_encode: false,
            pull_frame_encode: false,
            frame_decode_into: false,
            row_level_frame_decode: false,
            effort_range: None,
            quality_range: None,
        }
    }

    /// Whether the encoder embeds ICC color profiles from `with_metadata`.
    pub const fn encode_icc(&self) -> bool {
        self.encode_icc
    }

    /// Whether the encoder embeds EXIF data from `with_metadata`.
    pub const fn encode_exif(&self) -> bool {
        self.encode_exif
    }

    /// Whether the encoder embeds XMP data from `with_metadata`.
    pub const fn encode_xmp(&self) -> bool {
        self.encode_xmp
    }

    /// Whether the decoder extracts ICC color profiles into `ImageInfo`.
    pub const fn decode_icc(&self) -> bool {
        self.decode_icc
    }

    /// Whether the decoder extracts EXIF data into `ImageInfo`.
    pub const fn decode_exif(&self) -> bool {
        self.decode_exif
    }

    /// Whether the decoder extracts XMP data into `ImageInfo`.
    pub const fn decode_xmp(&self) -> bool {
        self.decode_xmp
    }

    /// Whether `with_stop` on encode jobs is respected (not a no-op).
    pub const fn encode_cancel(&self) -> bool {
        self.encode_cancel
    }

    /// Whether `with_stop` on decode jobs is respected (not a no-op).
    pub const fn decode_cancel(&self) -> bool {
        self.decode_cancel
    }

    /// Whether the codec supports grayscale natively (without expanding to RGB).
    pub const fn native_gray(&self) -> bool {
        self.native_gray
    }

    /// Whether `probe_header` is cheap (header parse only, not a full decode).
    pub const fn cheap_probe(&self) -> bool {
        self.cheap_probe
    }

    /// Whether the codec supports encoding animation (multiple frames).
    pub const fn encode_animation(&self) -> bool {
        self.encode_animation
    }

    /// Whether the codec supports decoding animation (multiple frames).
    pub const fn decode_animation(&self) -> bool {
        self.decode_animation
    }

    /// Whether the codec supports 16-bit per channel natively (without
    /// dithering/truncating to 8-bit internally).
    pub const fn native_16bit(&self) -> bool {
        self.native_16bit
    }

    /// Whether the codec supports mathematically lossless encoding.
    pub const fn lossless(&self) -> bool {
        self.lossless
    }

    /// Whether the codec supports HDR content (wide gamut, high bit depth,
    /// PQ/HLG transfer functions, HDR metadata).
    pub const fn hdr(&self) -> bool {
        self.hdr
    }

    /// Whether the encoder embeds CICP color description from `with_metadata`.
    pub const fn encode_cicp(&self) -> bool {
        self.encode_cicp
    }

    /// Whether the decoder extracts CICP color description into `ImageInfo`.
    pub const fn decode_cicp(&self) -> bool {
        self.decode_cicp
    }

    /// Whether the codec enforces [`ResourceLimits::max_pixels`](crate::ResourceLimits::max_pixels).
    pub const fn enforces_max_pixels(&self) -> bool {
        self.enforces_max_pixels
    }

    /// Whether the codec enforces [`ResourceLimits::max_memory_bytes`](crate::ResourceLimits::max_memory_bytes).
    pub const fn enforces_max_memory(&self) -> bool {
        self.enforces_max_memory
    }

    /// Whether the codec enforces [`ResourceLimits::max_file_size`](crate::ResourceLimits::max_file_size).
    pub const fn enforces_max_file_size(&self) -> bool {
        self.enforces_max_file_size
    }

    /// Meaningful effort range `[min, max]`.
    ///
    /// `None` means the codec has no effort tuning —
    /// [`EncoderConfig::with_generic_effort()`](crate::EncoderConfig::with_generic_effort) is a no-op.
    pub const fn effort_range(&self) -> Option<[i32; 2]> {
        self.effort_range
    }

    /// Meaningful quality range `[min, max]` on the calibrated 0.0–100.0 scale.
    ///
    /// `None` means the codec is lossless-only —
    /// [`EncoderConfig::with_generic_quality()`](crate::EncoderConfig::with_generic_quality) is a no-op.
    /// Most lossy codecs return `Some([0.0, 100.0])`.
    pub const fn quality_range(&self) -> Option<[f32; 2]> {
        self.quality_range
    }

    /// Whether the codec supports lossy encoding.
    ///
    /// Complement to [`lossless()`](CodecCapabilities::lossless) — a codec
    /// can support both (e.g. WebP, JXL).
    pub const fn lossy(&self) -> bool {
        self.lossy
    }

    /// Whether the codec handles f32 pixel data natively (without
    /// converting to u8/u16 internally).
    pub const fn native_f32(&self) -> bool {
        self.native_f32
    }

    /// Whether the codec handles alpha channel natively (not JPEG).
    pub const fn native_alpha(&self) -> bool {
        self.native_alpha
    }

    /// Whether `decode_into()` is
    /// implemented (not just a stub that returns an error).
    pub const fn decode_into(&self) -> bool {
        self.decode_into
    }

    /// Whether `push_rows()` / `finish()` actually work
    /// (row-level encode).
    pub const fn row_level_encode(&self) -> bool {
        self.row_level_encode
    }

    /// Whether `encode_from()` works (pull-from-source encode).
    pub const fn pull_encode(&self) -> bool {
        self.pull_encode
    }

    /// Whether `decode_rows()` pushes real streaming rows
    /// (not a single full-frame callback).
    pub const fn row_level_decode(&self) -> bool {
        self.row_level_decode
    }

    /// Whether `FrameEncoder::begin_frame` / `push_rows` / `end_frame` work.
    pub const fn row_level_frame_encode(&self) -> bool {
        self.row_level_frame_encode
    }

    /// Whether `pull_frame()` works (pull-from-source frame encode).
    pub const fn pull_frame_encode(&self) -> bool {
        self.pull_frame_encode
    }

    /// Whether `next_frame_into()` works (frame decode into caller buffer).
    pub const fn frame_decode_into(&self) -> bool {
        self.frame_decode_into
    }

    /// Whether `next_frame_rows()` works (row-level frame decode).
    pub const fn row_level_frame_decode(&self) -> bool {
        self.row_level_frame_decode
    }

    // --- const builder methods for static construction ---

    /// Set ICC embed support on encode.
    pub const fn with_encode_icc(mut self, v: bool) -> Self {
        self.encode_icc = v;
        self
    }

    /// Set EXIF embed support on encode.
    pub const fn with_encode_exif(mut self, v: bool) -> Self {
        self.encode_exif = v;
        self
    }

    /// Set XMP embed support on encode.
    pub const fn with_encode_xmp(mut self, v: bool) -> Self {
        self.encode_xmp = v;
        self
    }

    /// Set ICC extraction support on decode.
    pub const fn with_decode_icc(mut self, v: bool) -> Self {
        self.decode_icc = v;
        self
    }

    /// Set EXIF extraction support on decode.
    pub const fn with_decode_exif(mut self, v: bool) -> Self {
        self.decode_exif = v;
        self
    }

    /// Set XMP extraction support on decode.
    pub const fn with_decode_xmp(mut self, v: bool) -> Self {
        self.decode_xmp = v;
        self
    }

    /// Set cooperative cancellation support on encode.
    pub const fn with_encode_cancel(mut self, v: bool) -> Self {
        self.encode_cancel = v;
        self
    }

    /// Set cooperative cancellation support on decode.
    pub const fn with_decode_cancel(mut self, v: bool) -> Self {
        self.decode_cancel = v;
        self
    }

    /// Set native grayscale support.
    pub const fn with_native_gray(mut self, v: bool) -> Self {
        self.native_gray = v;
        self
    }

    /// Set whether probe_header is cheap.
    pub const fn with_cheap_probe(mut self, v: bool) -> Self {
        self.cheap_probe = v;
        self
    }

    /// Set animation encoding support.
    pub const fn with_encode_animation(mut self, v: bool) -> Self {
        self.encode_animation = v;
        self
    }

    /// Set animation decoding support.
    pub const fn with_decode_animation(mut self, v: bool) -> Self {
        self.decode_animation = v;
        self
    }

    /// Set native 16-bit support.
    pub const fn with_native_16bit(mut self, v: bool) -> Self {
        self.native_16bit = v;
        self
    }

    /// Set lossless encoding support.
    pub const fn with_lossless(mut self, v: bool) -> Self {
        self.lossless = v;
        self
    }

    /// Set HDR support.
    pub const fn with_hdr(mut self, v: bool) -> Self {
        self.hdr = v;
        self
    }

    /// Set CICP embed support on encode.
    pub const fn with_encode_cicp(mut self, v: bool) -> Self {
        self.encode_cicp = v;
        self
    }

    /// Set CICP extraction support on decode.
    pub const fn with_decode_cicp(mut self, v: bool) -> Self {
        self.decode_cicp = v;
        self
    }

    /// Set whether the codec enforces max_pixels limits.
    pub const fn with_enforces_max_pixels(mut self, v: bool) -> Self {
        self.enforces_max_pixels = v;
        self
    }

    /// Set whether the codec enforces max_memory limits.
    pub const fn with_enforces_max_memory(mut self, v: bool) -> Self {
        self.enforces_max_memory = v;
        self
    }

    /// Set whether the codec enforces max_file_size limits.
    pub const fn with_enforces_max_file_size(mut self, v: bool) -> Self {
        self.enforces_max_file_size = v;
        self
    }

    /// Set the meaningful effort range `[min, max]`.
    ///
    /// `None` (default) means the codec has no effort tuning.
    pub const fn with_effort_range(mut self, min: i32, max: i32) -> Self {
        self.effort_range = Some([min, max]);
        self
    }

    /// Set the meaningful quality range `[min, max]` on the calibrated 0.0–100.0 scale.
    ///
    /// `None` (default) means the codec is lossless-only.
    /// Most lossy codecs: `with_quality_range(0.0, 100.0)`.
    pub const fn with_quality_range(mut self, min: f32, max: f32) -> Self {
        self.quality_range = Some([min, max]);
        self
    }

    /// Set lossy encoding support.
    pub const fn with_lossy(mut self, v: bool) -> Self {
        self.lossy = v;
        self
    }

    /// Set native f32 pixel data support.
    pub const fn with_native_f32(mut self, v: bool) -> Self {
        self.native_f32 = v;
        self
    }

    /// Set native alpha channel support.
    pub const fn with_native_alpha(mut self, v: bool) -> Self {
        self.native_alpha = v;
        self
    }

    /// Set `decode_into()` support.
    pub const fn with_decode_into(mut self, v: bool) -> Self {
        self.decode_into = v;
        self
    }

    /// Set row-level encode support (`push_rows()` / `finish()`).
    pub const fn with_row_level_encode(mut self, v: bool) -> Self {
        self.row_level_encode = v;
        self
    }

    /// Set pull encode support (`encode_from()`).
    pub const fn with_pull_encode(mut self, v: bool) -> Self {
        self.pull_encode = v;
        self
    }

    /// Set row-level decode support.
    pub const fn with_row_level_decode(mut self, v: bool) -> Self {
        self.row_level_decode = v;
        self
    }

    /// Set row-level frame encode support.
    pub const fn with_row_level_frame_encode(mut self, v: bool) -> Self {
        self.row_level_frame_encode = v;
        self
    }

    /// Set pull frame encode support.
    pub const fn with_pull_frame_encode(mut self, v: bool) -> Self {
        self.pull_frame_encode = v;
        self
    }

    /// Set frame decode-into support.
    pub const fn with_frame_decode_into(mut self, v: bool) -> Self {
        self.frame_decode_into = v;
        self
    }

    /// Set row-level frame decode support.
    pub const fn with_row_level_frame_decode(mut self, v: bool) -> Self {
        self.row_level_frame_decode = v;
        self
    }
}

impl core::fmt::Debug for CodecCapabilities {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut s = f.debug_struct("CodecCapabilities");
        s.field("encode_icc", &self.encode_icc)
            .field("encode_exif", &self.encode_exif)
            .field("encode_xmp", &self.encode_xmp)
            .field("decode_icc", &self.decode_icc)
            .field("decode_exif", &self.decode_exif)
            .field("decode_xmp", &self.decode_xmp)
            .field("encode_cancel", &self.encode_cancel)
            .field("decode_cancel", &self.decode_cancel)
            .field("native_gray", &self.native_gray)
            .field("cheap_probe", &self.cheap_probe)
            .field("encode_animation", &self.encode_animation)
            .field("decode_animation", &self.decode_animation)
            .field("native_16bit", &self.native_16bit)
            .field("lossless", &self.lossless)
            .field("lossy", &self.lossy)
            .field("hdr", &self.hdr)
            .field("native_f32", &self.native_f32)
            .field("native_alpha", &self.native_alpha)
            .field("decode_into", &self.decode_into)
            .field("row_level_encode", &self.row_level_encode)
            .field("pull_encode", &self.pull_encode)
            .field("row_level_decode", &self.row_level_decode)
            .field("row_level_frame_encode", &self.row_level_frame_encode)
            .field("pull_frame_encode", &self.pull_frame_encode)
            .field("frame_decode_into", &self.frame_decode_into)
            .field("row_level_frame_decode", &self.row_level_frame_decode)
            .field("encode_cicp", &self.encode_cicp)
            .field("decode_cicp", &self.decode_cicp)
            .field("enforces_max_pixels", &self.enforces_max_pixels)
            .field("enforces_max_memory", &self.enforces_max_memory)
            .field("enforces_max_file_size", &self.enforces_max_file_size);
        if let Some(range) = &self.effort_range {
            s.field("effort_range", range);
        }
        if let Some(range) = &self.quality_range {
            s.field("quality_range", range);
        }
        s.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_all_false() {
        let caps = CodecCapabilities::new();
        assert!(!caps.encode_icc());
        assert!(!caps.encode_exif());
        assert!(!caps.encode_xmp());
        assert!(!caps.decode_icc());
        assert!(!caps.decode_exif());
        assert!(!caps.decode_xmp());
        assert!(!caps.encode_cancel());
        assert!(!caps.decode_cancel());
        assert!(!caps.native_gray());
        assert!(!caps.cheap_probe());
        assert!(!caps.encode_animation());
        assert!(!caps.decode_animation());
        assert!(!caps.native_16bit());
        assert!(!caps.lossless());
        assert!(!caps.lossy());
        assert!(!caps.hdr());
        assert!(!caps.native_f32());
        assert!(!caps.native_alpha());
        assert!(!caps.decode_into());
        assert!(!caps.row_level_encode());
        assert!(!caps.pull_encode());
        assert!(!caps.row_level_decode());
        assert!(!caps.row_level_frame_encode());
        assert!(!caps.pull_frame_encode());
        assert!(!caps.frame_decode_into());
        assert!(!caps.row_level_frame_decode());
        assert!(!caps.encode_cicp());
        assert!(!caps.decode_cicp());
        assert!(!caps.enforces_max_pixels());
        assert!(!caps.enforces_max_memory());
        assert!(!caps.enforces_max_file_size());
        assert!(caps.effort_range().is_none());
        assert!(caps.quality_range().is_none());
    }

    #[test]
    fn builder_sets_fields() {
        let caps = CodecCapabilities::new()
            .with_encode_icc(true)
            .with_decode_cancel(true)
            .with_native_gray(true)
            .with_cheap_probe(true)
            .with_encode_animation(true)
            .with_native_16bit(true)
            .with_hdr(true);
        assert!(caps.encode_icc());
        assert!(!caps.encode_exif());
        assert!(caps.decode_cancel());
        assert!(caps.native_gray());
        assert!(caps.cheap_probe());
        assert!(caps.encode_animation());
        assert!(!caps.decode_animation());
        assert!(caps.native_16bit());
        assert!(!caps.lossless());
        assert!(caps.hdr());
    }

    #[test]
    fn static_construction() {
        static CAPS: CodecCapabilities = CodecCapabilities::new()
            .with_encode_icc(true)
            .with_encode_exif(true)
            .with_encode_xmp(true)
            .with_encode_cancel(true)
            .with_decode_cancel(true)
            .with_encode_animation(true)
            .with_decode_animation(true)
            .with_lossless(true)
            .with_encode_cicp(true)
            .with_decode_cicp(true);
        assert!(CAPS.encode_icc());
        assert!(CAPS.encode_cancel());
        assert!(!CAPS.native_gray());
        assert!(CAPS.encode_animation());
        assert!(CAPS.decode_animation());
        assert!(CAPS.lossless());
        assert!(CAPS.encode_cicp());
        assert!(CAPS.decode_cicp());
    }

    #[test]
    fn enforces_limits_flags() {
        let caps = CodecCapabilities::new()
            .with_enforces_max_pixels(true)
            .with_enforces_max_memory(true)
            .with_enforces_max_file_size(true);
        assert!(caps.enforces_max_pixels());
        assert!(caps.enforces_max_memory());
        assert!(caps.enforces_max_file_size());
    }

    #[test]
    fn enforces_limits_static() {
        static CAPS: CodecCapabilities = CodecCapabilities::new()
            .with_enforces_max_pixels(true)
            .with_enforces_max_file_size(true);
        assert!(CAPS.enforces_max_pixels());
        assert!(!CAPS.enforces_max_memory());
        assert!(CAPS.enforces_max_file_size());
    }

    #[test]
    fn effort_range_builder_and_getter() {
        let caps = CodecCapabilities::new().with_effort_range(0, 10);
        assert_eq!(caps.effort_range(), Some([0i32, 10]));
    }

    #[test]
    fn quality_range_builder_and_getter() {
        let caps = CodecCapabilities::new().with_quality_range(0.0, 100.0);
        assert_eq!(caps.quality_range(), Some([0.0, 100.0]));
    }

    #[test]
    fn effort_quality_static_construction() {
        static CAPS: CodecCapabilities = CodecCapabilities::new()
            .with_lossless(true)
            .with_effort_range(0, 100)
            .with_quality_range(0.0, 100.0);
        assert!(CAPS.lossless());
        assert_eq!(CAPS.effort_range(), Some([0, 100]));
        assert_eq!(CAPS.quality_range(), Some([0.0, 100.0]));
    }

    #[test]
    fn lossless_only_codec_no_quality_range() {
        let caps = CodecCapabilities::new()
            .with_lossless(true)
            .with_effort_range(1, 9);
        assert!(caps.lossless());
        assert_eq!(caps.effort_range(), Some([1, 9]));
        assert!(caps.quality_range().is_none()); // lossless-only → no quality range
    }

    #[test]
    fn new_capability_flags() {
        let caps = CodecCapabilities::new()
            .with_lossy(true)
            .with_native_f32(true)
            .with_native_alpha(true)
            .with_decode_into(true)
            .with_row_level_encode(true)
            .with_pull_encode(true)
            .with_row_level_decode(true)
            .with_row_level_frame_encode(true)
            .with_pull_frame_encode(true)
            .with_frame_decode_into(true)
            .with_row_level_frame_decode(true);
        assert!(caps.lossy());
        assert!(caps.native_f32());
        assert!(caps.native_alpha());
        assert!(caps.decode_into());
        assert!(caps.row_level_encode());
        assert!(caps.pull_encode());
        assert!(caps.row_level_decode());
        assert!(caps.row_level_frame_encode());
        assert!(caps.pull_frame_encode());
        assert!(caps.frame_decode_into());
        assert!(caps.row_level_frame_decode());
    }

    #[test]
    fn new_capability_flags_static() {
        static CAPS: CodecCapabilities = CodecCapabilities::new()
            .with_lossy(true)
            .with_lossless(true)
            .with_native_f32(true)
            .with_native_alpha(true)
            .with_decode_into(true)
            .with_row_level_encode(true);
        assert!(CAPS.lossy());
        assert!(CAPS.lossless());
        assert!(CAPS.native_f32());
        assert!(CAPS.native_alpha());
        assert!(CAPS.decode_into());
        assert!(CAPS.row_level_encode());
        assert!(!CAPS.pull_encode());
        assert!(!CAPS.row_level_decode());
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
        // Verify it implements core::error::Error
        let op = UnsupportedOperation::DecodeInto;
        let err: &dyn core::error::Error = &op;
        assert!(err.source().is_none());
    }

    #[test]
    fn unsupported_operation_eq_hash() {
        use alloc::collections::BTreeSet;
        let mut set = BTreeSet::new();
        // Use Debug ordering via string comparison as a proxy
        assert_eq!(
            UnsupportedOperation::DecodeInto,
            UnsupportedOperation::DecodeInto
        );
        assert_ne!(
            UnsupportedOperation::DecodeInto,
            UnsupportedOperation::PullEncode
        );
        // Hash: just verify it compiles with a set-like usage
        let _ = set.insert(alloc::format!("{:?}", UnsupportedOperation::DecodeInto));
    }

    #[test]
    fn has_unsupported_operation_trait() {
        #[derive(Debug)]
        enum TestError {
            Unsupported(UnsupportedOperation),
            Other,
        }
        impl HasUnsupportedOperation for TestError {
            fn unsupported_operation(&self) -> Option<UnsupportedOperation> {
                match self {
                    TestError::Unsupported(op) => Some(*op),
                    TestError::Other => None,
                }
            }
        }
        let err = TestError::Unsupported(UnsupportedOperation::AnimationEncode);
        assert_eq!(
            err.unsupported_operation(),
            Some(UnsupportedOperation::AnimationEncode)
        );
        let err2 = TestError::Other;
        assert_eq!(err2.unsupported_operation(), None);
    }
}
