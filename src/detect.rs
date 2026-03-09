//! Source encoding detection traits.
//!
//! [`SourceEncodingDetails`] provides a codec-agnostic interface for
//! querying properties of the encoder that produced an image file.
//! Each codec's probe type (e.g. `WebPProbe`, `JpegProbe`) implements
//! this trait, providing both the generic quality number and full
//! codec-specific details via downcasting.
//!
//! # Usage
//!
//! ```rust,ignore
//! use zencodec::decode::{DecodeOutput, SourceEncodingDetails};
//!
//! let output: DecodeOutput = /* decode an image */;
//!
//! if let Some(details) = output.source_encoding_details() {
//!     // Generic quality (0-100, same scale as EncoderConfig::with_generic_quality)
//!     if let Some(q) = details.source_generic_quality() {
//!         println!("Source quality: {:.0}", q);
//!     }
//!
//!     // Codec-specific details via downcast
//!     if let Some(webp) = details.codec_details::<zenwebp::detect::WebPProbe>() {
//!         println!("VP8 quantizer: {:?}", webp.bitstream);
//!     }
//! }
//! ```

use core::any::Any;

/// Codec-agnostic interface for source encoding properties.
///
/// Implemented by each codec's probe/detect type to provide both a
/// generic quality number and codec-specific details. The generic
/// quality uses the same 0.0–100.0 scale as
/// [`EncoderConfig::with_generic_quality()`](crate::encode::EncoderConfig::with_generic_quality).
///
/// # Downcasting
///
/// Use [`codec_details()`](dyn SourceEncodingDetails::codec_details) to
/// access the concrete probe type for codec-specific fields:
///
/// ```rust,ignore
/// if let Some(jpeg) = details.codec_details::<zenjpeg::detect::JpegProbe>() {
///     println!("Encoder: {:?}", jpeg.encoder);
/// }
/// ```
pub trait SourceEncodingDetails: Any + Send + Sync {
    /// Estimated source quality on the generic 0.0–100.0 scale.
    ///
    /// Returns `None` for lossless formats (PNG, lossless WebP) or when
    /// quality cannot be determined from the bitstream headers.
    ///
    /// The value is approximate (±5) — different encoders map quality
    /// parameters differently, so the reverse-engineered value may not
    /// exactly match the original setting.
    fn source_generic_quality(&self) -> Option<f32>;

    /// Whether the source encoding is lossless.
    ///
    /// True for PNG, lossless WebP, lossless JPEG 2000, etc.
    /// When true, `source_generic_quality()` typically returns `None`.
    fn is_lossless(&self) -> bool {
        false
    }
}

// ── Design note ────────────────────────────────────────────────────────
//
// This trait intentionally has very few methods. Only properties that are
// meaningful across ALL image formats belong here (quality, lossless).
//
// Codec-specific details — color type, bit depth, palette size, chroma
// subsampling, encoder family, quantizer tables, etc. — belong as fields
// or methods on the concrete probe struct (e.g. `PngProbe`, `JpegProbe`).
// Callers access them via downcast:
//
//     if let Some(png) = details.codec_details::<PngProbe>() {
//         println!("{}bpp, palette: {:?}", png.bits_per_pixel(), png.palette_size);
//     }
//
// This keeps the trait stable and avoids a combinatorial explosion of
// optional methods that only apply to a subset of codecs.
// ───────────────────────────────────────────────────────────────────────

// Downcast helper — avoids requiring callers to import `core::any::Any`.
impl dyn SourceEncodingDetails {
    /// Downcast to a concrete codec probe type.
    ///
    /// Returns `Some(&T)` if the underlying type matches, `None` otherwise.
    ///
    /// ```rust,ignore
    /// use zenwebp::detect::WebPProbe;
    ///
    /// if let Some(webp) = details.codec_details::<WebPProbe>() {
    ///     println!("Lossy: {:?}", webp.bitstream);
    /// }
    /// ```
    pub fn codec_details<T: SourceEncodingDetails + 'static>(&self) -> Option<&T> {
        (self as &dyn Any).downcast_ref()
    }
}

impl dyn SourceEncodingDetails + Send {
    /// Downcast to a concrete codec probe type.
    pub fn codec_details<T: SourceEncodingDetails + 'static>(&self) -> Option<&T> {
        (self as &dyn Any).downcast_ref()
    }
}

impl dyn SourceEncodingDetails + Send + Sync {
    /// Downcast to a concrete codec probe type.
    pub fn codec_details<T: SourceEncodingDetails + 'static>(&self) -> Option<&T> {
        (self as &dyn Any).downcast_ref()
    }
}
