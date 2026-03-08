//! Output prediction and resource cost estimation.
//!
//! [`OutputInfo`] describes what a decode will produce given current hints.
//! [`DecodeCost`] and [`EncodeCost`] estimate resource usage for budget checks.

use crate::Orientation;
use zenpixels::PixelDescriptor;

/// Predicted output from a decode operation.
///
/// Returned by [`DecodeJob::output_info()`](crate::decode::DecodeJob::output_info).
/// Describes what `decode()` or `decode_into()` will produce given the
/// current decode hints (crop, scale, orientation).
///
/// Use this to allocate destination buffers — the `width` and `height`
/// are what the decoder will actually write.
///
/// # Natural info vs output info
///
/// [`ImageInfo`](crate::ImageInfo) from `probe_header()` describes the file as stored:
/// original dimensions, original orientation, embedded metadata.
///
/// `OutputInfo` describes the decoder's output: post-crop, post-scale,
/// post-orientation dimensions and pixel format. This is what your
/// buffer must match.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct OutputInfo {
    /// Width of the decoded output in pixels.
    pub width: u32,
    /// Height of the decoded output in pixels.
    pub height: u32,
    /// Pixel format the decoder will produce natively (for `decode()`).
    ///
    /// For `decode_into()`, use any format from
    /// [`supported_descriptors()`](crate::decode::DecoderConfig::supported_descriptors) —
    /// this field tells you what the codec would pick if you let it choose.
    pub native_format: PixelDescriptor,
    /// Whether the output has an alpha channel.
    pub has_alpha: bool,
    /// Orientation the decoder will apply internally.
    ///
    /// [`Normal`](Orientation::Normal) means the decoder will NOT handle
    /// orientation — the caller must apply it. Any other value means the
    /// decoder will rotate/flip the pixels, and the output `width`/`height`
    /// already reflect the rotated dimensions.
    ///
    /// Remaining orientation for the caller:
    /// `natural.orientation - orientation_applied` (via D4 group composition).
    pub orientation_applied: Orientation,
    /// Crop the decoder will actually apply (`[x, y, width, height]` in
    /// source coordinates).
    ///
    /// May differ from the crop hint due to block alignment (JPEG MCU
    /// boundaries, AV1 superblock alignment, etc.). `None` if no crop.
    pub crop_applied: Option<[u32; 4]>,
}

impl OutputInfo {
    /// Create an `OutputInfo` for a simple full-frame decode (no hints applied).
    pub fn full_decode(width: u32, height: u32, native_format: PixelDescriptor) -> Self {
        Self {
            width,
            height,
            native_format,
            has_alpha: native_format.has_alpha(),
            orientation_applied: Orientation::Normal,
            crop_applied: None,
        }
    }

    /// Set whether the output has alpha.
    pub fn with_alpha(mut self, has_alpha: bool) -> Self {
        self.has_alpha = has_alpha;
        self
    }

    /// Set the orientation the decoder will apply.
    pub fn with_orientation_applied(mut self, o: Orientation) -> Self {
        self.orientation_applied = o;
        self
    }

    /// Set the crop the decoder will apply.
    pub fn with_crop_applied(mut self, rect: [u32; 4]) -> Self {
        self.crop_applied = Some(rect);
        self
    }

    /// Minimum buffer size in bytes for the native format (no padding).
    ///
    /// This is `width * height * bytes_per_pixel`. For aligned/strided
    /// buffers, use [`PixelDescriptor::aligned_stride()`] instead.
    pub fn buffer_size(&self) -> u64 {
        self.width as u64 * self.height as u64 * self.native_format.bytes_per_pixel() as u64
    }

    /// Pixel count (`width * height`).
    pub fn pixel_count(&self) -> u64 {
        self.width as u64 * self.height as u64
    }
}

/// Estimated resource cost of a decode operation.
///
/// Returned by `DecodeJob::estimated_cost()`.
/// Use this for resource management: reject oversized images, limit
/// concurrency, enforce memory budgets, or choose processing strategies
/// before committing to a decode.
///
/// `output_bytes` and `pixel_count` are always accurate (derived from
/// [`OutputInfo`]). `peak_memory` is a codec-specific estimate and may
/// be `None` if the codec can't predict it.
///
/// Use [`ResourceLimits::check_decode_cost()`](crate::ResourceLimits::check_decode_cost)
/// to validate against limits.
///
/// # Typical working memory multipliers (over output buffer size)
///
/// | Codec | Multiplier | Notes |
/// |-------|-----------|-------|
/// | JPEG | ~1-2x | DCT blocks + Huffman state |
/// | PNG | ~1-2x | Filter + zlib state |
/// | GIF | ~1-2x | LZW + frame compositing canvas |
/// | WebP lossy | ~2x | VP8 reference frames |
/// | AV1/AVIF | ~2-3x | Tile buffers + CDEF + loop restoration + reference frames |
/// | JPEG XL to u8 | ~1-2x | Native format output |
/// | JPEG XL to f32 | ~4x | Float conversion overhead |
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct DecodeCost {
    /// Output buffer size in bytes (width × height × bytes_per_pixel).
    pub output_bytes: u64,
    /// Total pixels in the output (width × height).
    pub pixel_count: u64,
    /// Estimated peak memory during decode, in bytes.
    ///
    /// Includes working buffers (YUV planes, entropy decode state, etc.)
    /// plus the output buffer. `None` if the codec can't estimate this.
    ///
    /// When `None`, callers should fall back to `output_bytes` as a
    /// lower-bound estimate for limit checks.
    pub peak_memory: Option<u64>,
}

impl DecodeCost {
    /// Create a decode cost estimate from raw values.
    ///
    /// Prefer [`from_output_info`](DecodeCost::from_output_info) — it computes
    /// `output_bytes` and `pixel_count` for you.
    #[deprecated(since = "0.2.0", note = "use DecodeCost::from_output_info() instead")]
    pub const fn new(output_bytes: u64, pixel_count: u64, peak_memory: Option<u64>) -> Self {
        Self {
            output_bytes,
            pixel_count,
            peak_memory,
        }
    }

    /// Create a decode cost estimate from [`OutputInfo`].
    ///
    /// Computes `output_bytes` and `pixel_count` from the output dimensions
    /// and pixel format. `peak_memory` defaults to `None`; chain
    /// [`with_peak_memory`](DecodeCost::with_peak_memory) to set it.
    ///
    /// ```rust,ignore
    /// let cost = DecodeCost::from_output_info(&info)
    ///     .with_peak_memory(info.buffer_size() * 2);
    /// ```
    pub fn from_output_info(info: &OutputInfo) -> Self {
        Self {
            output_bytes: info.buffer_size(),
            pixel_count: info.pixel_count(),
            peak_memory: None,
        }
    }

    /// Set estimated peak memory (builder pattern).
    pub const fn with_peak_memory(mut self, bytes: u64) -> Self {
        self.peak_memory = Some(bytes);
        self
    }
}

/// Estimated resource cost of an encode operation.
///
/// Returned by `EncodeJob::estimated_cost()`.
/// Use this for resource management before committing to an encode.
///
/// The caller already knows the input dimensions and pixel format, so
/// `input_bytes` and `pixel_count` are provided for convenience (the
/// caller could compute these). `peak_memory` is the useful codec-specific
/// estimate.
///
/// Use [`ResourceLimits::check_encode_cost()`](crate::ResourceLimits::check_encode_cost)
/// to validate against limits.
///
/// # Typical working memory multipliers (over input buffer size)
///
/// | Codec | Multiplier | Notes |
/// |-------|-----------|-------|
/// | JPEG | ~2-3x | DCT blocks + Huffman coding |
/// | PNG | ~2x | Filter selection + zlib |
/// | GIF | ~1-2x | LZW + quantization palette |
/// | WebP lossy | ~3-4x | VP8 RDO + reference frames |
/// | AV1/AVIF | ~4-8x | Transform + RDO + reference frames |
/// | JPEG XL lossless | ~12x | Float buffers + ANS tokens |
/// | JPEG XL lossy | ~6-22x | Highly variable with effort/quality |
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct EncodeCost {
    /// Input buffer size in bytes (width × height × bytes_per_pixel).
    pub input_bytes: u64,
    /// Total pixels in the input (width × height).
    pub pixel_count: u64,
    /// Estimated peak memory during encode, in bytes.
    ///
    /// Includes input pixel data, working buffers (transform coefficients,
    /// entropy coding state, rate-distortion buffers), and output buffer.
    /// `None` if the codec can't estimate this.
    ///
    /// When `None`, callers should fall back to `input_bytes` as a
    /// lower-bound estimate for limit checks.
    pub peak_memory: Option<u64>,
}

impl EncodeCost {
    /// Create an encode cost estimate from raw values.
    ///
    /// Prefer [`for_input`](EncodeCost::for_input) — it computes
    /// `input_bytes` and `pixel_count` for you.
    #[deprecated(since = "0.2.0", note = "use EncodeCost::for_input() instead")]
    pub const fn new(input_bytes: u64, pixel_count: u64, peak_memory: Option<u64>) -> Self {
        Self {
            input_bytes,
            pixel_count,
            peak_memory,
        }
    }

    /// Create an encode cost estimate from input dimensions and pixel format.
    ///
    /// Computes `input_bytes` and `pixel_count` automatically. `peak_memory`
    /// defaults to `None`; chain [`with_peak_memory`](EncodeCost::with_peak_memory)
    /// to set it.
    ///
    /// ```rust,ignore
    /// let bpp = descriptor.bytes_per_pixel() as u64;
    /// let cost = EncodeCost::for_input(width, height, descriptor)
    ///     .with_peak_memory(width as u64 * height as u64 * bpp * 3);
    /// ```
    pub fn for_input(width: u32, height: u32, descriptor: PixelDescriptor) -> Self {
        let pixels = width as u64 * height as u64;
        Self {
            input_bytes: pixels * descriptor.bytes_per_pixel() as u64,
            pixel_count: pixels,
            peak_memory: None,
        }
    }

    /// Set estimated peak memory (builder pattern).
    pub const fn with_peak_memory(mut self, bytes: u64) -> Self {
        self.peak_memory = Some(bytes);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_cost_from_output_info() {
        let info = OutputInfo::full_decode(10, 5, PixelDescriptor::RGBA8_SRGB);
        let cost = DecodeCost::from_output_info(&info);
        assert_eq!(cost.output_bytes, 200); // 10*5*4
        assert_eq!(cost.pixel_count, 50); // 10*5
        assert_eq!(cost.peak_memory, None);
    }

    #[test]
    fn decode_cost_with_peak_memory() {
        let info = OutputInfo::full_decode(10, 5, PixelDescriptor::RGBA8_SRGB);
        let cost = DecodeCost::from_output_info(&info).with_peak_memory(400);
        assert_eq!(cost.output_bytes, 200);
        assert_eq!(cost.peak_memory, Some(400));
    }

    #[test]
    fn encode_cost_for_input() {
        let cost = EncodeCost::for_input(10, 5, PixelDescriptor::RGB8_SRGB);
        assert_eq!(cost.input_bytes, 150); // 10*5*3
        assert_eq!(cost.pixel_count, 50);
        assert_eq!(cost.peak_memory, None);
    }

    #[test]
    fn encode_cost_with_peak_memory() {
        let cost = EncodeCost::for_input(10, 5, PixelDescriptor::RGB8_SRGB).with_peak_memory(450);
        assert_eq!(cost.input_bytes, 150);
        assert_eq!(cost.peak_memory, Some(450));
    }
}
