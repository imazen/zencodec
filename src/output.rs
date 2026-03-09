//! Encode and decode output types.

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::any::Any;

use crate::detect::SourceEncodingDetails;
use crate::{ImageFormat, ImageInfo, Metadata};
use zenpixels::{PixelBuffer, PixelDescriptor, PixelSlice};

/// Output from an encode operation.
///
/// Carries the encoded bytes, the format enum, and the actual MIME type and
/// file extension of the output. The MIME type and extension default to
/// [`ImageFormat::mime_type()`] / [`ImageFormat::extension()`] but can be
/// overridden with [`with_mime_type()`](EncodeOutput::with_mime_type) /
/// [`with_extension()`](EncodeOutput::with_extension) for cases where the
/// output differs from the base format (e.g. `image/apng` vs `image/png`).
#[non_exhaustive]
pub struct EncodeOutput {
    data: Vec<u8>,
    format: ImageFormat,
    mime_type: &'static str,
    extension: &'static str,
    extras: Option<Arc<dyn Any + Send + Sync>>,
}

impl EncodeOutput {
    /// Create a new encode output.
    ///
    /// MIME type and extension default to the format's primary values.
    /// Use [`with_mime_type()`](EncodeOutput::with_mime_type) /
    /// [`with_extension()`](EncodeOutput::with_extension) to override
    /// (e.g. for animated PNG → `"image/apng"` / `"apng"`).
    pub fn new(data: Vec<u8>, format: ImageFormat) -> Self {
        Self {
            data,
            mime_type: format.mime_type(),
            extension: format.extension(),
            format,
            extras: None,
        }
    }

    /// Override the MIME type for the encoded output.
    ///
    /// Use when the actual output differs from the base format's default,
    /// e.g. `"image/apng"` for animated PNG.
    pub fn with_mime_type(mut self, mime_type: &'static str) -> Self {
        self.mime_type = mime_type;
        self
    }

    /// Override the file extension for the encoded output.
    ///
    /// Use when the actual output differs from the base format's default,
    /// e.g. `"apng"` for animated PNG.
    pub fn with_extension(mut self, extension: &'static str) -> Self {
        self.extension = extension;
        self
    }

    /// Consume and return the encoded bytes.
    pub fn into_vec(self) -> Vec<u8> {
        self.data
    }

    /// Borrow the encoded bytes.
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Encoded byte count.
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Whether the output is empty.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// The format that was used for encoding.
    pub fn format(&self) -> ImageFormat {
        self.format
    }

    /// MIME type of the encoded output (e.g. `"image/png"` or `"image/apng"`).
    ///
    /// Defaults to [`ImageFormat::mime_type()`] unless overridden by the
    /// encoder via [`with_mime_type()`](EncodeOutput::with_mime_type).
    pub fn mime_type(&self) -> &'static str {
        self.mime_type
    }

    /// Suggested file extension for the encoded output (e.g. `"png"` or `"apng"`).
    ///
    /// Defaults to [`ImageFormat::extension()`] unless overridden by the
    /// encoder via [`with_extension()`](EncodeOutput::with_extension).
    pub fn extension(&self) -> &'static str {
        self.extension
    }

    /// Attach format-specific extras (e.g., encoding statistics, codec-specific metadata).
    pub fn with_extras<T: Any + Send + Sync + 'static>(mut self, extras: T) -> Self {
        self.extras = Some(Arc::new(extras));
        self
    }

    /// Borrow typed extras if present and the type matches.
    pub fn extras<T: Any + Send + Sync + 'static>(&self) -> Option<&T> {
        self.extras.as_ref()?.downcast_ref()
    }

    /// Take typed extras, consuming them from this output.
    ///
    /// Returns `Some(T)` only when this is the sole reference to the extras
    /// (i.e. no clones are holding an `Arc` to the same value). Returns
    /// `None` if the type doesn't match or other references exist.
    pub fn take_extras<T: Any + Send + Sync + 'static>(&mut self) -> Option<T> {
        let arc = self.extras.take()?;
        let arc_t: Arc<T> = arc.downcast().ok()?;
        Arc::try_unwrap(arc_t).ok()
    }
}

impl Clone for EncodeOutput {
    fn clone(&self) -> Self {
        Self {
            data: self.data.clone(),
            format: self.format,
            mime_type: self.mime_type,
            extension: self.extension,
            extras: self.extras.clone(),
        }
    }
}

impl core::fmt::Debug for EncodeOutput {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("EncodeOutput")
            .field("data_len", &self.data.len())
            .field("format", &self.format)
            .field("mime_type", &self.mime_type)
            .field("extension", &self.extension)
            .field("has_extras", &self.extras.is_some())
            .finish()
    }
}

impl PartialEq for EncodeOutput {
    fn eq(&self, other: &Self) -> bool {
        self.data == other.data
            && self.format == other.format
            && self.mime_type == other.mime_type
            && self.extension == other.extension
    }
}

impl Eq for EncodeOutput {}

impl AsRef<[u8]> for EncodeOutput {
    fn as_ref(&self) -> &[u8] {
        &self.data
    }
}

/// Output from a decode operation.
///
/// Stores pixel data as a [`PixelBuffer`] with embedded format descriptor.
/// The descriptor carries the correct transfer function, color primaries,
/// and signal range — no need to resolve from CICP separately.
#[non_exhaustive]
pub struct DecodeOutput {
    pixels: PixelBuffer,
    info: ImageInfo,
    source_encoding: Option<Box<dyn SourceEncodingDetails>>,
    extras: Option<Arc<dyn Any + Send + Sync>>,
}

impl DecodeOutput {
    /// Create a new decode output from a [`PixelBuffer`].
    ///
    /// The PixelBuffer's descriptor should already have the correct transfer
    /// function and color primaries set by the decoder.
    pub fn new(pixels: PixelBuffer, info: ImageInfo) -> Self {
        Self {
            pixels,
            info,
            source_encoding: None,
            extras: None,
        }
    }

    /// Attach source encoding details (quality estimate, codec-specific probe data).
    ///
    /// The concrete type must implement [`SourceEncodingDetails`] — typically
    /// a codec's probe type (e.g. `WebPProbe`, `JpegProbe`).
    ///
    /// Callers access the generic quality via
    /// [`source_encoding_details()`](DecodeOutput::source_encoding_details)
    /// and downcast for codec-specific fields.
    pub fn with_source_encoding_details<T: SourceEncodingDetails + 'static>(
        mut self,
        details: T,
    ) -> Self {
        self.source_encoding = Some(Box::new(details));
        self
    }

    /// Source encoding details, if available.
    ///
    /// Returns the codec's probe result with both generic quality
    /// (via [`SourceEncodingDetails::source_generic_quality()`]) and
    /// codec-specific fields (via [`codec_details()`](dyn SourceEncodingDetails::codec_details)).
    pub fn source_encoding_details(&self) -> Option<&dyn SourceEncodingDetails> {
        self.source_encoding.as_deref()
    }

    /// Take source encoding details, consuming them from this output.
    pub fn take_source_encoding_details(&mut self) -> Option<Box<dyn SourceEncodingDetails>> {
        self.source_encoding.take()
    }

    /// Attach format-specific extras (e.g., JPEG gain maps, MPF data).
    pub fn with_extras<T: Any + Send + Sync + 'static>(mut self, extras: T) -> Self {
        self.extras = Some(Arc::new(extras));
        self
    }

    /// Borrow typed extras if present and the type matches.
    pub fn extras<T: Any + Send + Sync + 'static>(&self) -> Option<&T> {
        self.extras.as_ref()?.downcast_ref()
    }

    /// Take typed extras, consuming them from this output.
    ///
    /// Returns `Some(T)` only when this is the sole reference to the extras
    /// (i.e. no clones are holding an `Arc` to the same value). Returns
    /// `None` if the type doesn't match or other references exist.
    pub fn take_extras<T: Any + Send + Sync + 'static>(&mut self) -> Option<T> {
        let arc = self.extras.take()?;
        let arc_t: Arc<T> = arc.downcast().ok()?;
        Arc::try_unwrap(arc_t).ok()
    }

    /// Borrow the pixel data as a [`PixelSlice`].
    pub fn pixels(&self) -> PixelSlice<'_> {
        self.pixels.as_slice()
    }

    /// Take the pixel buffer, consuming this output.
    pub fn into_buffer(self) -> PixelBuffer {
        self.pixels
    }

    /// Image info.
    pub fn info(&self) -> &ImageInfo {
        &self.info
    }

    /// Image width.
    pub fn width(&self) -> u32 {
        self.pixels.width()
    }

    /// Image height.
    pub fn height(&self) -> u32 {
        self.pixels.height()
    }

    /// Whether the image has an alpha channel.
    pub fn has_alpha(&self) -> bool {
        self.pixels.has_alpha()
    }

    /// Pixel format descriptor.
    pub fn descriptor(&self) -> PixelDescriptor {
        self.pixels.descriptor()
    }

    /// Detected format.
    pub fn format(&self) -> ImageFormat {
        self.info.format
    }

    /// Build a [`ColorContext`](crate::ColorContext) from the image's ICC/CICP metadata.
    ///
    /// Delegates to [`ImageInfo::color_context()`].
    pub fn color_context(&self) -> Option<alloc::sync::Arc<crate::ColorContext>> {
        self.info.color_context()
    }

    /// Get embedded metadata for roundtrip encode.
    pub fn metadata(&self) -> Metadata {
        self.info.metadata()
    }
}

impl core::fmt::Debug for DecodeOutput {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DecodeOutput")
            .field("pixels", &self.pixels)
            .field("format", &self.info.format)
            .field("has_source_encoding", &self.source_encoding.is_some())
            .finish()
    }
}

/// A composited full-canvas animation frame, borrowing the decoder's canvas.
///
/// Returned by [`FullFrameDecoder::render_next_frame()`](crate::decode::FullFrameDecoder::render_next_frame).
/// The pixel data borrows the decoder's internal canvas buffer — calling
/// `render_next_frame()` again invalidates this borrow.
///
/// Use [`to_owned_frame()`](FullFrame::to_owned_frame) to copy the pixel data
/// if you need to retain the frame across calls.
#[non_exhaustive]
pub struct FullFrame<'a> {
    pixels: PixelSlice<'a>,
    duration_ms: u32,
    frame_index: u32,
}

impl<'a> FullFrame<'a> {
    /// Create a full frame borrowing pixel data.
    pub fn new(pixels: PixelSlice<'a>, duration_ms: u32, frame_index: u32) -> Self {
        Self {
            pixels,
            duration_ms,
            frame_index,
        }
    }

    /// Borrow the composited pixel data.
    pub fn pixels(&self) -> &PixelSlice<'a> {
        &self.pixels
    }

    /// Frame duration in milliseconds.
    ///
    /// Zero means platform-dependent minimum display time for most formats.
    /// For JXL, zero-duration frames are compositing helpers and are never
    /// yielded by [`FullFrameDecoder`](crate::decode::FullFrameDecoder).
    pub fn duration_ms(&self) -> u32 {
        self.duration_ms
    }

    /// Displayed frame index (0-based).
    ///
    /// Counts only frames yielded by the decoder — internal compositing
    /// frames (e.g. JXL zero-duration) are not counted.
    pub fn frame_index(&self) -> u32 {
        self.frame_index
    }

    /// Copy pixel data to produce an owned frame.
    pub fn to_owned_frame(&self) -> OwnedFullFrame {
        let ps = &self.pixels;
        let w = ps.width();
        let h = ps.rows();
        let desc = ps.descriptor();
        let bpp = desc.bytes_per_pixel();
        let row_bytes = w as usize * bpp;

        // Copy rows into a tightly-packed buffer
        let mut data = alloc::vec::Vec::with_capacity(h as usize * row_bytes);
        for y in 0..h {
            data.extend_from_slice(ps.row(y));
        }

        let pixels = PixelBuffer::from_vec(data, w, h, desc)
            .expect("to_owned_frame: buffer sized correctly");

        OwnedFullFrame {
            pixels,
            duration_ms: self.duration_ms,
            frame_index: self.frame_index,
            extras: None,
        }
    }
}

impl core::fmt::Debug for FullFrame<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FullFrame")
            .field("pixels", &self.pixels)
            .field("duration_ms", &self.duration_ms)
            .field("frame_index", &self.frame_index)
            .finish()
    }
}

/// A composited full-canvas animation frame with owned pixel data.
///
/// Produced by [`FullFrame::to_owned_frame()`] or
/// [`FullFrameDecoder::render_next_frame_owned()`](crate::decode::FullFrameDecoder::render_next_frame_owned).
#[non_exhaustive]
pub struct OwnedFullFrame {
    pixels: PixelBuffer,
    duration_ms: u32,
    frame_index: u32,
    extras: Option<Arc<dyn Any + Send + Sync>>,
}

impl OwnedFullFrame {
    /// Create an owned frame from a [`PixelBuffer`].
    pub fn new(pixels: PixelBuffer, duration_ms: u32, frame_index: u32) -> Self {
        Self {
            pixels,
            duration_ms,
            frame_index,
            extras: None,
        }
    }

    /// Borrow the pixel data as a [`PixelSlice`].
    pub fn pixels(&self) -> PixelSlice<'_> {
        self.pixels.as_slice()
    }

    /// Take the pixel buffer, consuming this frame.
    pub fn into_buffer(self) -> PixelBuffer {
        self.pixels
    }

    /// Frame duration in milliseconds.
    pub fn duration_ms(&self) -> u32 {
        self.duration_ms
    }

    /// Displayed frame index (0-based).
    pub fn frame_index(&self) -> u32 {
        self.frame_index
    }

    /// Borrow as a [`FullFrame`].
    pub fn as_full_frame(&self) -> FullFrame<'_> {
        FullFrame::new(self.pixels.as_slice(), self.duration_ms, self.frame_index)
    }

    /// Attach format-specific extras (e.g., per-frame codec metadata).
    pub fn with_extras<T: Any + Send + Sync + 'static>(mut self, extras: T) -> Self {
        self.extras = Some(Arc::new(extras));
        self
    }

    /// Borrow typed extras if present and the type matches.
    pub fn extras<T: Any + Send + Sync + 'static>(&self) -> Option<&T> {
        self.extras.as_ref()?.downcast_ref()
    }

    /// Take typed extras, consuming them from this frame.
    ///
    /// Returns `Some(T)` only when this is the sole reference to the extras
    /// (i.e. no clones are holding an `Arc` to the same value). Returns
    /// `None` if the type doesn't match or other references exist.
    pub fn take_extras<T: Any + Send + Sync + 'static>(&mut self) -> Option<T> {
        let arc = self.extras.take()?;
        let arc_t: Arc<T> = arc.downcast().ok()?;
        Arc::try_unwrap(arc_t).ok()
    }
}

impl core::fmt::Debug for OwnedFullFrame {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("OwnedFullFrame")
            .field("pixels", &self.pixels)
            .field("duration_ms", &self.duration_ms)
            .field("frame_index", &self.frame_index)
            .field("has_extras", &self.extras.is_some())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use zenpixels::PixelDescriptor;

    fn make_rgb8_buffer(w: u32, h: u32) -> PixelBuffer {
        PixelBuffer::new(w, h, PixelDescriptor::RGB8_SRGB)
    }

    fn make_rgba8_buffer(w: u32, h: u32) -> PixelBuffer {
        PixelBuffer::new(w, h, PixelDescriptor::RGBA8_SRGB)
    }

    fn make_gray8_buffer(w: u32, h: u32) -> PixelBuffer {
        PixelBuffer::new(w, h, PixelDescriptor::GRAY8_SRGB)
    }

    #[test]
    fn encode_output() {
        let output = EncodeOutput::new(vec![1, 2, 3], ImageFormat::Jpeg);
        assert_eq!(output.format(), ImageFormat::Jpeg);
        assert_eq!(output.mime_type(), "image/jpeg");
        assert_eq!(output.extension(), "jpg");
        assert_eq!(output.len(), 3);
        assert_eq!(output.data(), &[1, 2, 3]);
        assert!(!output.is_empty());
        assert_eq!(output.into_vec(), vec![1, 2, 3]);
    }

    #[test]
    fn encode_output_mime_extension_override() {
        let output = EncodeOutput::new(vec![], ImageFormat::Png)
            .with_mime_type("image/apng")
            .with_extension("apng");
        assert_eq!(output.format(), ImageFormat::Png);
        assert_eq!(output.mime_type(), "image/apng");
        assert_eq!(output.extension(), "apng");
    }

    #[test]
    fn encode_output_eq() {
        let a = EncodeOutput::new(vec![1, 2, 3], ImageFormat::Jpeg);
        let b = EncodeOutput::new(vec![1, 2, 3], ImageFormat::Jpeg);
        assert_eq!(a, b);

        let c = EncodeOutput::new(vec![1, 2, 3], ImageFormat::Png);
        assert_ne!(a, c);
    }

    #[test]
    fn decode_output() {
        let buf = make_rgb8_buffer(2, 2);
        let info = ImageInfo::new(2, 2, ImageFormat::Png).with_frame_count(1);
        let output = DecodeOutput::new(buf, info);
        assert_eq!(output.width(), 2);
        assert_eq!(output.height(), 2);
        assert!(!output.has_alpha());
        assert_eq!(output.format(), ImageFormat::Png);
    }

    #[test]
    fn decode_output_extras() {
        let buf = make_rgb8_buffer(2, 2);
        let info = ImageInfo::new(2, 2, ImageFormat::Jpeg);
        let mut output = DecodeOutput::new(buf, info).with_extras(42u32);
        assert_eq!(output.extras::<u32>(), Some(&42u32));
        assert_eq!(output.extras::<u64>(), None);
        let taken = output.take_extras::<u32>();
        assert_eq!(taken, Some(42u32));
        assert!(output.extras::<u32>().is_none());
    }

    #[test]
    fn full_frame_borrowed() {
        let buf = make_rgba8_buffer(4, 4);
        let ps = buf.as_slice();
        let frame = FullFrame::new(ps, 100, 0);
        assert_eq!(frame.duration_ms(), 100);
        assert_eq!(frame.frame_index(), 0);
        assert_eq!(frame.pixels().width(), 4);
        assert_eq!(frame.pixels().rows(), 4);
    }

    #[test]
    fn full_frame_to_owned() {
        let buf = make_rgb8_buffer(2, 2);
        let ps = buf.as_slice();
        let frame = FullFrame::new(ps, 50, 3);
        let owned = frame.to_owned_frame();
        assert_eq!(owned.duration_ms(), 50);
        assert_eq!(owned.frame_index(), 3);
        assert_eq!(owned.pixels().width(), 2);
        assert_eq!(owned.pixels().rows(), 2);
    }

    #[test]
    fn owned_full_frame_as_full_frame() {
        let buf = make_rgb8_buffer(2, 2);
        let owned = OwnedFullFrame::new(buf, 100, 5);
        let borrowed = owned.as_full_frame();
        assert_eq!(borrowed.duration_ms(), 100);
        assert_eq!(borrowed.frame_index(), 5);
    }

    #[test]
    fn owned_full_frame_into_buffer() {
        let buf = make_rgb8_buffer(3, 3);
        let owned = OwnedFullFrame::new(buf, 200, 0);
        let recovered = owned.into_buffer();
        assert_eq!(recovered.width(), 3);
        assert_eq!(recovered.height(), 3);
    }

    #[test]
    fn full_frame_debug() {
        let buf = make_gray8_buffer(2, 2);
        let ps = buf.as_slice();
        let frame = FullFrame::new(ps, 100, 3);
        let s = alloc::format!("{:?}", frame);
        assert!(s.contains("FullFrame"));
        assert!(s.contains("duration_ms: 100"));
        assert!(s.contains("frame_index: 3"));
    }

    #[test]
    fn owned_full_frame_debug() {
        let buf = make_rgb8_buffer(2, 2);
        let owned = OwnedFullFrame::new(buf, 50, 1);
        let s = alloc::format!("{:?}", owned);
        assert!(s.contains("OwnedFullFrame"));
        assert!(s.contains("duration_ms: 50"));
        assert!(s.contains("frame_index: 1"));
    }
}
