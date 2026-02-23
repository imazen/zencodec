//! Encode and decode output types.

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::any::Any;
use imgref::ImgRef;
use rgb::alt::BGRA;
use rgb::{Gray, Rgb, Rgba};

use crate::{ImageFormat, ImageInfo, MetadataView, PixelData};

/// Output from an encode operation.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct EncodeOutput {
    data: Vec<u8>,
    format: ImageFormat,
}

impl EncodeOutput {
    /// Create a new encode output.
    pub fn new(data: Vec<u8>, format: ImageFormat) -> Self {
        Self { data, format }
    }

    /// Consume and return the encoded bytes.
    pub fn into_vec(self) -> Vec<u8> {
        self.data
    }

    /// Borrow the encoded bytes.
    pub fn bytes(&self) -> &[u8] {
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
}

impl AsRef<[u8]> for EncodeOutput {
    fn as_ref(&self) -> &[u8] {
        &self.data
    }
}

/// Output from a decode operation.
#[non_exhaustive]
pub struct DecodeOutput {
    pixels: PixelData,
    info: ImageInfo,
    extras: Option<Box<dyn Any + Send>>,
}

impl DecodeOutput {
    /// Create a new decode output.
    pub fn new(pixels: PixelData, info: ImageInfo) -> Self {
        Self {
            pixels,
            info,
            extras: None,
        }
    }

    /// Attach format-specific extras (e.g., JPEG gain maps, MPF data).
    pub fn with_extras<T: Any + Send + 'static>(mut self, extras: T) -> Self {
        self.extras = Some(Box::new(extras));
        self
    }

    /// Borrow typed extras if present and the type matches.
    pub fn extras<T: Any + Send + 'static>(&self) -> Option<&T> {
        self.extras.as_ref()?.downcast_ref()
    }

    /// Take typed extras, consuming them from this output.
    pub fn take_extras<T: Any + Send + 'static>(&mut self) -> Option<T> {
        let extras = self.extras.take()?;
        extras.downcast().ok().map(|b| *b)
    }

    /// Borrow the pixel data in its native format.
    pub fn pixels(&self) -> &PixelData {
        &self.pixels
    }

    /// Take the pixel data, consuming this output.
    pub fn into_pixels(self) -> PixelData {
        self.pixels
    }

    /// Borrow as RGB8 if that's the native format.
    pub fn as_rgb8(&self) -> Option<imgref::ImgRef<'_, Rgb<u8>>> {
        match &self.pixels {
            PixelData::Rgb8(img) => Some(img.as_ref()),
            _ => None,
        }
    }

    /// Borrow as RGBA8 if that's the native format.
    pub fn as_rgba8(&self) -> Option<imgref::ImgRef<'_, Rgba<u8>>> {
        match &self.pixels {
            PixelData::Rgba8(img) => Some(img.as_ref()),
            _ => None,
        }
    }

    /// Borrow as BGRA8 if that's the native format.
    pub fn as_bgra8(&self) -> Option<imgref::ImgRef<'_, BGRA<u8>>> {
        match &self.pixels {
            PixelData::Bgra8(img) => Some(img.as_ref()),
            _ => None,
        }
    }

    /// Borrow as Gray8 if that's the native format.
    pub fn as_gray8(&self) -> Option<imgref::ImgRef<'_, Gray<u8>>> {
        match &self.pixels {
            PixelData::Gray8(img) => Some(img.as_ref()),
            _ => None,
        }
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

    /// Pixel format descriptor with the transfer function resolved from CICP.
    ///
    /// Unlike [`PixelData::descriptor()`] which returns
    /// [`Unknown`](crate::TransferFunction::Unknown), this method uses the
    /// CICP metadata from [`ImageInfo`] to set the correct transfer function.
    /// If CICP is absent or the transfer characteristics code is not
    /// recognized, the transfer function remains `Unknown`.
    pub fn descriptor(&self) -> crate::PixelDescriptor {
        self.pixels
            .descriptor()
            .with_transfer(self.info.transfer_function())
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

    /// Borrow embedded metadata for roundtrip encode.
    pub fn metadata(&self) -> MetadataView<'_> {
        self.info.metadata()
    }

    /// Convert to RGB8, consuming this output.
    pub fn into_rgb8(self) -> imgref::ImgVec<Rgb<u8>> {
        self.pixels.into_rgb8()
    }

    /// Convert to RGBA8, consuming this output.
    pub fn into_rgba8(self) -> imgref::ImgVec<Rgba<u8>> {
        self.pixels.into_rgba8()
    }

    /// Convert to Gray8, consuming this output.
    pub fn into_gray8(self) -> imgref::ImgVec<Gray<u8>> {
        self.pixels.into_gray8()
    }

    /// Convert to BGRA8, consuming this output.
    pub fn into_bgra8(self) -> imgref::ImgVec<BGRA<u8>> {
        self.pixels.into_bgra8()
    }
}

impl core::fmt::Debug for DecodeOutput {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DecodeOutput")
            .field("pixels", &self.pixels)
            .field("format", &self.info.format)
            .finish()
    }
}

/// How a frame is composited over the previous canvas state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum FrameBlend {
    /// Replace the region with this frame's pixels.
    #[default]
    Source,
    /// Alpha-blend this frame over the existing canvas.
    Over,
}

/// What happens to the canvas after displaying this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum FrameDisposal {
    /// Leave the canvas as-is after this frame.
    #[default]
    None,
    /// Restore the canvas region to the background color.
    RestoreBackground,
    /// Restore the canvas region to the state before this frame.
    RestorePrevious,
}

/// A single frame from animation decoding.
///
/// Carries container-level metadata via `Arc<ImageInfo>` so each frame
/// is self-describing without duplicating metadata per frame.
#[non_exhaustive]
pub struct DecodeFrame {
    pixels: PixelData,
    info: Arc<ImageInfo>,
    delay_ms: u32,
    index: u32,
    /// Which previous frame this frame depends on for compositing.
    /// `None` means this is a keyframe (fully independent).
    required_frame: Option<u32>,
    /// Blend mode for compositing this frame over the required frame.
    blend: FrameBlend,
    /// How to handle the canvas after this frame is displayed.
    disposal: FrameDisposal,
    /// Region of the canvas this frame updates (None = full canvas).
    /// Format: `[x, y, width, height]`.
    frame_rect: Option<[u32; 4]>,
}

impl DecodeFrame {
    /// Create a new decode frame with default compositing (keyframe, source blend, no disposal).
    ///
    /// The `info` parameter carries container-level metadata (format, color space,
    /// ICC/EXIF/XMP, orientation) shared across all frames via `Arc`.
    pub fn new(pixels: PixelData, info: Arc<ImageInfo>, delay_ms: u32, index: u32) -> Self {
        Self {
            pixels,
            info,
            delay_ms,
            index,
            required_frame: None,
            blend: FrameBlend::Source,
            disposal: FrameDisposal::None,
            frame_rect: None,
        }
    }

    /// Set the required prior frame for compositing.
    pub fn with_required_frame(mut self, frame: u32) -> Self {
        self.required_frame = Some(frame);
        self
    }

    /// Set the blend mode.
    pub fn with_blend(mut self, blend: FrameBlend) -> Self {
        self.blend = blend;
        self
    }

    /// Set the disposal method.
    pub fn with_disposal(mut self, disposal: FrameDisposal) -> Self {
        self.disposal = disposal;
        self
    }

    /// Set the frame rectangle (region this frame updates).
    /// Format: `[x, y, width, height]`.
    pub fn with_frame_rect(mut self, rect: [u32; 4]) -> Self {
        self.frame_rect = Some(rect);
        self
    }

    /// Borrow the pixel data.
    pub fn pixels(&self) -> &PixelData {
        &self.pixels
    }

    /// Take the pixel data, consuming this frame.
    pub fn into_pixels(self) -> PixelData {
        self.pixels
    }

    /// Borrow as RGB8 if that's the native format.
    pub fn as_rgb8(&self) -> Option<imgref::ImgRef<'_, Rgb<u8>>> {
        match &self.pixels {
            PixelData::Rgb8(img) => Some(img.as_ref()),
            _ => None,
        }
    }

    /// Borrow as RGBA8 if that's the native format.
    pub fn as_rgba8(&self) -> Option<imgref::ImgRef<'_, Rgba<u8>>> {
        match &self.pixels {
            PixelData::Rgba8(img) => Some(img.as_ref()),
            _ => None,
        }
    }

    /// Borrow as BGRA8 if that's the native format.
    pub fn as_bgra8(&self) -> Option<imgref::ImgRef<'_, BGRA<u8>>> {
        match &self.pixels {
            PixelData::Bgra8(img) => Some(img.as_ref()),
            _ => None,
        }
    }

    /// Borrow as Gray8 if that's the native format.
    pub fn as_gray8(&self) -> Option<imgref::ImgRef<'_, Gray<u8>>> {
        match &self.pixels {
            PixelData::Gray8(img) => Some(img.as_ref()),
            _ => None,
        }
    }

    /// Frame delay in milliseconds.
    pub fn delay_ms(&self) -> u32 {
        self.delay_ms
    }

    /// Frame index (0-based).
    pub fn index(&self) -> u32 {
        self.index
    }

    /// Which previous frame this frame depends on for compositing.
    /// `None` means this is a keyframe (fully independent).
    pub fn required_frame(&self) -> Option<u32> {
        self.required_frame
    }

    /// Blend mode for compositing this frame over the required frame.
    pub fn blend(&self) -> FrameBlend {
        self.blend
    }

    /// How to handle the canvas after this frame is displayed.
    pub fn disposal(&self) -> FrameDisposal {
        self.disposal
    }

    /// Region of the canvas this frame updates, or `None` for full canvas.
    /// Format: `[x, y, width, height]`.
    pub fn frame_rect(&self) -> Option<[u32; 4]> {
        self.frame_rect
    }

    /// Container-level image info (format, color space, metadata).
    ///
    /// Shared across all frames via `Arc` — cloning is cheap.
    pub fn info(&self) -> &ImageInfo {
        &self.info
    }

    /// Clone the `Arc<ImageInfo>` for sharing with other frames or consumers.
    pub fn info_arc(&self) -> Arc<ImageInfo> {
        Arc::clone(&self.info)
    }

    /// Borrow embedded metadata for roundtrip encode.
    ///
    /// Convenience for `self.info().metadata()`.
    pub fn metadata(&self) -> MetadataView<'_> {
        self.info.metadata()
    }

    /// Detected format (from container-level info).
    pub fn format(&self) -> ImageFormat {
        self.info.format
    }

    /// Frame width.
    pub fn width(&self) -> u32 {
        self.pixels.width()
    }

    /// Frame height.
    pub fn height(&self) -> u32 {
        self.pixels.height()
    }

    /// Whether this frame has an alpha channel.
    pub fn has_alpha(&self) -> bool {
        self.pixels.has_alpha()
    }

    /// Convert to RGB8, consuming this frame.
    pub fn into_rgb8(self) -> imgref::ImgVec<Rgb<u8>> {
        self.pixels.into_rgb8()
    }

    /// Convert to RGBA8, consuming this frame.
    pub fn into_rgba8(self) -> imgref::ImgVec<Rgba<u8>> {
        self.pixels.into_rgba8()
    }

    /// Convert to Gray8, consuming this frame.
    pub fn into_gray8(self) -> imgref::ImgVec<Gray<u8>> {
        self.pixels.into_gray8()
    }

    /// Convert to BGRA8, consuming this frame.
    pub fn into_bgra8(self) -> imgref::ImgVec<BGRA<u8>> {
        self.pixels.into_bgra8()
    }
}

impl core::fmt::Debug for DecodeFrame {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut s = f.debug_struct("DecodeFrame");
        s.field("pixels", &self.pixels)
            .field("format", &self.info.format)
            .field("delay_ms", &self.delay_ms)
            .field("index", &self.index);
        if let Some(req) = self.required_frame {
            s.field("required_frame", &req);
        }
        if self.blend != FrameBlend::Source {
            s.field("blend", &self.blend);
        }
        if self.disposal != FrameDisposal::None {
            s.field("disposal", &self.disposal);
        }
        if let Some(rect) = &self.frame_rect {
            s.field("frame_rect", rect);
        }
        s.finish()
    }
}

/// A single typed frame for animation encoding.
///
/// Pairs typed pixel data with a frame duration. Used by
/// typed convenience methods on [`EncoderConfig`](crate::EncoderConfig).
#[derive(Clone, Copy)]
#[non_exhaustive]
/// A typed frame for animation encoding convenience methods.
///
/// Like [`EncodeFrame`] but with a concrete pixel type for use with
/// the typed convenience methods on [`EncoderConfig`](crate::EncoderConfig)
/// (e.g. `encode_animation_rgb8`).
pub struct TypedEncodeFrame<'a, Pixel> {
    /// The pixel data for this frame.
    pub image: ImgRef<'a, Pixel>,
    /// Frame duration in milliseconds.
    pub duration_ms: u32,
    /// Canvas region `[x, y, w, h]` this frame occupies.
    ///
    /// `None` means the frame covers the full canvas.
    pub frame_rect: Option<[u32; 4]>,
    /// How to composite this frame onto the canvas.
    pub blend: FrameBlend,
    /// What happens to the canvas region after this frame is displayed.
    pub disposal: FrameDisposal,
}

impl<'a, Pixel> TypedEncodeFrame<'a, Pixel> {
    /// Create a full-canvas typed encode frame with default compositing.
    pub fn new(image: ImgRef<'a, Pixel>, duration_ms: u32) -> Self {
        Self {
            image,
            duration_ms,
            frame_rect: None,
            blend: FrameBlend::Source,
            disposal: FrameDisposal::None,
        }
    }

    /// Set the canvas region this frame occupies.
    pub fn with_frame_rect(mut self, rect: [u32; 4]) -> Self {
        self.frame_rect = Some(rect);
        self
    }

    /// Set the blend mode for compositing.
    pub fn with_blend(mut self, blend: FrameBlend) -> Self {
        self.blend = blend;
        self
    }

    /// Set the disposal method after this frame is displayed.
    pub fn with_disposal(mut self, disposal: FrameDisposal) -> Self {
        self.disposal = disposal;
        self
    }
}

impl<Pixel> core::fmt::Debug for TypedEncodeFrame<'_, Pixel> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut s = f.debug_struct("TypedEncodeFrame");
        s.field("width", &self.image.width())
            .field("height", &self.image.height())
            .field("duration_ms", &self.duration_ms);
        if let Some(rect) = &self.frame_rect {
            s.field("frame_rect", rect);
        }
        if self.blend != FrameBlend::Source {
            s.field("blend", &self.blend);
        }
        if self.disposal != FrameDisposal::None {
            s.field("disposal", &self.disposal);
        }
        s.finish()
    }
}

#[non_exhaustive]
/// A single frame for animation encoding.
///
/// For full-canvas frames, use [`EncodeFrame::new()`] — `frame_rect`
/// defaults to `None` (full canvas), blend to [`FrameBlend::Source`],
/// disposal to [`FrameDisposal::None`].
///
/// For sub-canvas frames (GIF, APNG, WebP, JXL), set `frame_rect` to
/// the region this frame occupies within the canvas, and set `blend`
/// and `disposal` to control compositing. Use
/// [`EncodeJob::with_canvas_size()`](crate::EncodeJob::with_canvas_size)
/// to set the canvas dimensions before pushing sub-canvas frames.
///
/// # Example
///
/// ```
/// use zencodec_types::{EncodeFrame, FrameBlend, FrameDisposal, PixelSlice};
///
/// # fn example(full_canvas: PixelSlice<'_>, sub_region: PixelSlice<'_>) {
/// // Full-canvas frame (simple case)
/// let frame = EncodeFrame::new(full_canvas, 100);
///
/// // Sub-canvas frame at (10, 20) with alpha blending
/// let frame = EncodeFrame::new(sub_region, 100)
///     .with_frame_rect([10, 20, 64, 48])
///     .with_blend(FrameBlend::Over)
///     .with_disposal(FrameDisposal::RestoreBackground);
/// # }
/// ```
pub struct EncodeFrame<'a> {
    /// The pixel data for this frame.
    pub pixels: crate::PixelSlice<'a>,
    /// Frame duration in milliseconds.
    pub duration_ms: u32,
    /// Canvas region `[x, y, w, h]` this frame occupies.
    ///
    /// `None` means the frame covers the full canvas (pixels dimensions
    /// must match canvas dimensions). When `Some`, the pixels dimensions
    /// must match `w` and `h`.
    pub frame_rect: Option<[u32; 4]>,
    /// How to composite this frame onto the canvas.
    pub blend: FrameBlend,
    /// What happens to the canvas region after this frame is displayed.
    pub disposal: FrameDisposal,
}

impl<'a> EncodeFrame<'a> {
    /// Create a full-canvas encode frame with default compositing.
    pub fn new(pixels: crate::PixelSlice<'a>, duration_ms: u32) -> Self {
        Self {
            pixels,
            duration_ms,
            frame_rect: None,
            blend: FrameBlend::Source,
            disposal: FrameDisposal::None,
        }
    }

    /// Set the canvas region this frame occupies.
    ///
    /// `rect` is `[x, y, width, height]` in canvas coordinates.
    /// The pixel data dimensions must match `width` and `height`.
    pub fn with_frame_rect(mut self, rect: [u32; 4]) -> Self {
        self.frame_rect = Some(rect);
        self
    }

    /// Set the blend mode for compositing.
    pub fn with_blend(mut self, blend: FrameBlend) -> Self {
        self.blend = blend;
        self
    }

    /// Set the disposal method after this frame is displayed.
    pub fn with_disposal(mut self, disposal: FrameDisposal) -> Self {
        self.disposal = disposal;
        self
    }
}

impl core::fmt::Debug for EncodeFrame<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut s = f.debug_struct("EncodeFrame");
        s.field("pixels", &self.pixels)
            .field("duration_ms", &self.duration_ms);
        if let Some(rect) = &self.frame_rect {
            s.field("frame_rect", rect);
        }
        if self.blend != FrameBlend::Source {
            s.field("blend", &self.blend);
        }
        if self.disposal != FrameDisposal::None {
            s.field("disposal", &self.disposal);
        }
        s.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use imgref::ImgVec;

    fn test_info(w: u32, h: u32, fmt: ImageFormat) -> Arc<ImageInfo> {
        Arc::new(ImageInfo::new(w, h, fmt))
    }

    #[test]
    fn encode_output() {
        let output = EncodeOutput::new(vec![1, 2, 3], ImageFormat::Jpeg);
        assert_eq!(output.format(), ImageFormat::Jpeg);
        assert_eq!(output.len(), 3);
        assert_eq!(output.bytes(), &[1, 2, 3]);
        assert!(!output.is_empty());
        assert_eq!(output.into_vec(), vec![1, 2, 3]);
    }

    #[test]
    fn decode_output() {
        let img = ImgVec::new(
            vec![
                Rgb {
                    r: 10u8,
                    g: 20,
                    b: 30
                };
                4
            ],
            2,
            2,
        );
        let info = ImageInfo::new(2, 2, ImageFormat::Png).with_frame_count(1);
        let output = DecodeOutput::new(PixelData::Rgb8(img), info);
        assert_eq!(output.width(), 2);
        assert_eq!(output.height(), 2);
        assert!(!output.has_alpha());
        assert_eq!(output.format(), ImageFormat::Png);
        assert!(output.as_rgb8().is_some());
        assert!(output.as_rgba8().is_none());
    }

    #[test]
    fn decode_output_extras() {
        let img = ImgVec::new(vec![Rgb { r: 0u8, g: 0, b: 0 }; 4], 2, 2);
        let info = ImageInfo::new(2, 2, ImageFormat::Jpeg);
        let mut output = DecodeOutput::new(PixelData::Rgb8(img), info).with_extras(42u32);
        assert_eq!(output.extras::<u32>(), Some(&42u32));
        assert_eq!(output.extras::<u64>(), None); // wrong type
        let taken = output.take_extras::<u32>();
        assert_eq!(taken, Some(42u32));
        assert!(output.extras::<u32>().is_none()); // consumed
    }

    #[test]
    fn decode_frame() {
        let img = ImgVec::new(
            vec![
                Rgba {
                    r: 1u8,
                    g: 2,
                    b: 3,
                    a: 4
                };
                4
            ],
            2,
            2,
        );
        let info = test_info(2, 2, ImageFormat::Png);
        let frame = DecodeFrame::new(PixelData::Rgba8(img), info, 100, 0);
        assert_eq!(frame.delay_ms(), 100);
        assert_eq!(frame.index(), 0);
        assert_eq!(frame.width(), 2);
        assert_eq!(frame.height(), 2);
        assert!(frame.has_alpha());
        assert_eq!(frame.format(), ImageFormat::Png);
    }

    #[test]
    fn decode_frame_as_methods() {
        let img = ImgVec::new(
            vec![
                Rgb {
                    r: 10u8,
                    g: 20,
                    b: 30
                };
                4
            ],
            2,
            2,
        );
        let frame = DecodeFrame::new(
            PixelData::Rgb8(img),
            test_info(2, 2, ImageFormat::Png),
            50,
            1,
        );
        assert!(frame.as_rgb8().is_some());
        assert!(frame.as_rgba8().is_none());
        assert!(frame.as_bgra8().is_none());
        assert!(frame.as_gray8().is_none());
    }

    #[test]
    fn decode_frame_debug() {
        let img = ImgVec::new(vec![Gray::new(0u8); 4], 2, 2);
        let frame = DecodeFrame::new(
            PixelData::Gray8(img),
            test_info(2, 2, ImageFormat::Gif),
            100,
            3,
        );
        let s = alloc::format!("{:?}", frame);
        assert!(s.contains("DecodeFrame"));
        assert!(s.contains("delay_ms: 100"));
        assert!(s.contains("index: 3"));
    }

    #[test]
    fn decode_output_as_gray8() {
        let img = ImgVec::new(vec![Gray::new(42u8); 4], 2, 2);
        let info = ImageInfo::new(2, 2, ImageFormat::Png);
        let output = DecodeOutput::new(PixelData::Gray8(img), info);
        assert!(output.as_gray8().is_some());
        assert!(output.as_rgb8().is_none());
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
    fn decode_frame_info_accessor() {
        let img = ImgVec::new(vec![Rgb { r: 0u8, g: 0, b: 0 }; 4], 2, 2);
        let info =
            Arc::new(ImageInfo::new(2, 2, ImageFormat::WebP).with_icc_profile(vec![10, 20, 30]));
        let frame = DecodeFrame::new(PixelData::Rgb8(img), Arc::clone(&info), 100, 0);
        assert_eq!(frame.info().format, ImageFormat::WebP);
        assert_eq!(
            frame.info().icc_profile.as_deref(),
            Some([10, 20, 30].as_slice())
        );
        assert_eq!(frame.format(), ImageFormat::WebP);
    }

    #[test]
    fn decode_frame_metadata_accessor() {
        let img = ImgVec::new(vec![Rgb { r: 0u8, g: 0, b: 0 }; 4], 2, 2);
        let info = Arc::new(
            ImageInfo::new(2, 2, ImageFormat::Avif)
                .with_cicp(crate::info::Cicp::SRGB)
                .with_orientation(crate::Orientation::Rotate90),
        );
        let frame = DecodeFrame::new(PixelData::Rgb8(img), info, 50, 0);
        let meta = frame.metadata();
        assert_eq!(meta.cicp, Some(crate::info::Cicp::SRGB));
        assert_eq!(meta.orientation, crate::Orientation::Rotate90);
    }

    #[test]
    fn decode_frame_info_arc_shared() {
        let img = ImgVec::new(vec![Rgb { r: 0u8, g: 0, b: 0 }; 4], 2, 2);
        let info = Arc::new(ImageInfo::new(2, 2, ImageFormat::Gif));
        let frame = DecodeFrame::new(PixelData::Rgb8(img), Arc::clone(&info), 100, 0);
        let arc2 = frame.info_arc();
        assert!(Arc::ptr_eq(&info, &arc2));
    }
}
