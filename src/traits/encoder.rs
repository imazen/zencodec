//! Type-erased single-image and animation encoder traits.

use crate::{EncodeFrame, EncodeOutput};
use zenpixels::{PixelDescriptor, PixelSlice, PixelSliceMut};

/// Type-erased single-image encoder.
///
/// Accepts any pixel format at runtime via [`PixelSlice`] (type-erased).
/// The encoder dispatches internally based on the pixel descriptor.
///
/// Three mutually exclusive usage paths:
/// - [`encode()`](Encoder::encode) — all at once, consumes self
/// - [`push_rows()`](Encoder::push_rows) + [`finish()`](Encoder::finish) — caller pushes rows
/// - [`encode_from()`](Encoder::encode_from) — encoder pulls rows from a callback
///
/// Codecs that need full-frame data (e.g. AV1) may buffer internally
/// when rows are pushed or pulled incrementally.
///
/// Codecs may implement this alongside per-format traits like [`EncodeRgb8`](super::EncodeRgb8).
pub trait Encoder: Sized {
    /// The codec-specific error type.
    ///
    /// Must implement `From<UnsupportedOperation>` so default method
    /// implementations can return proper errors for unimplemented paths.
    type Error: core::error::Error + Send + Sync + 'static + From<crate::UnsupportedOperation>;

    /// Suggested strip height for optimal row-level encoding.
    ///
    /// For JPEG, typically the MCU height (8 or 16 rows).
    /// For PNG, typically 1 (row-at-a-time filtering).
    ///
    /// Returns 0 if the codec has no preference or doesn't support
    /// row-level encoding.
    fn preferred_strip_height(&self) -> u32 {
        0
    }

    /// Encode a complete image at once (consumes self).
    fn encode(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, Self::Error>;

    /// Encode from sRGB(A) 8-bit pixels provided as raw bytes.
    ///
    /// Universal entry point for RGBA8 data. Codecs should override this
    /// to handle pixel format conversion internally (e.g. JPEG drops
    /// the alpha channel and encodes as RGB).
    ///
    /// The default delegates to [`encode()`](Encoder::encode) by wrapping
    /// the raw bytes in a [`PixelSlice`] with the appropriate descriptor.
    ///
    /// - `data`: raw pixel bytes in RGBA order, 4 bytes per pixel
    /// - `make_opaque`: if `true`, treat the alpha channel as padding
    ///   (enables RGB fast paths in codecs that don't support alpha)
    /// - `width`, `height`: image dimensions in pixels
    /// - `stride_pixels`: row stride in pixels (≥ width)
    fn encode_srgba8(
        self,
        data: &[u8],
        make_opaque: bool,
        width: u32,
        height: u32,
        stride_pixels: u32,
    ) -> Result<EncodeOutput, Self::Error> {
        use zenpixels::AlphaMode;
        let descriptor = if make_opaque {
            PixelDescriptor::RGBA8_SRGB.with_alpha(Some(AlphaMode::Undefined))
        } else {
            PixelDescriptor::RGBA8_SRGB
        };
        let stride_bytes = stride_pixels as usize * 4;
        // PixelSlice::new only fails on dimension/stride/length mismatch,
        // which would be a caller bug. Panic is appropriate here.
        let pixels = PixelSlice::new(data, width, height, stride_bytes, descriptor)
            .expect("encode_srgba8: invalid dimensions or data length for RGBA8");
        self.encode(pixels)
    }

    /// Push scanline rows incrementally.
    ///
    /// # Errors
    ///
    /// Default returns [`UnsupportedOperation::RowLevelEncode`](crate::UnsupportedOperation::RowLevelEncode).
    fn push_rows(&mut self, _rows: PixelSlice<'_>) -> Result<(), Self::Error> {
        Err(crate::UnsupportedOperation::RowLevelEncode.into())
    }

    /// Finalize after push_rows. Returns encoded output.
    ///
    /// # Errors
    ///
    /// Default returns [`UnsupportedOperation::RowLevelEncode`](crate::UnsupportedOperation::RowLevelEncode).
    fn finish(self) -> Result<EncodeOutput, Self::Error> {
        Err(crate::UnsupportedOperation::RowLevelEncode.into())
    }

    /// Encode by pulling rows from a source callback.
    ///
    /// The encoder calls `source` repeatedly with the row index and a
    /// mutable buffer slice. The callback fills the buffer and returns
    /// the number of rows written. Returns `0` to signal end of image.
    ///
    /// # Errors
    ///
    /// Default returns [`UnsupportedOperation::PullEncode`](crate::UnsupportedOperation::PullEncode).
    fn encode_from(
        self,
        _source: &mut dyn FnMut(u32, PixelSliceMut<'_>) -> usize,
    ) -> Result<EncodeOutput, Self::Error> {
        Err(crate::UnsupportedOperation::PullEncode.into())
    }
}

/// Type-erased animation encoder.
///
/// Accepts any pixel format at runtime via [`PixelSlice`] (type-erased).
///
/// Three mutually exclusive per-frame paths:
/// - [`push_frame()`](FrameEncoder::push_frame) /
///   [`push_encode_frame()`](FrameEncoder::push_encode_frame) — complete frame at once
/// - [`begin_frame()`](FrameEncoder::begin_frame) +
///   [`push_rows()`](FrameEncoder::push_rows) +
///   [`end_frame()`](FrameEncoder::end_frame) — caller pushes rows
/// - [`pull_frame()`](FrameEncoder::pull_frame) — encoder pulls rows from a callback
///
/// Codecs may implement this alongside per-format frame traits like [`FrameEncodeRgba8`](super::FrameEncodeRgba8).
pub trait FrameEncoder: Sized {
    /// The codec-specific error type.
    ///
    /// Must implement `From<UnsupportedOperation>` so default method
    /// implementations can return proper errors for unimplemented paths.
    type Error: core::error::Error + Send + Sync + 'static + From<crate::UnsupportedOperation>;

    /// Push a complete full-canvas frame.
    fn push_frame(&mut self, pixels: PixelSlice<'_>, duration_ms: u32) -> Result<(), Self::Error>;

    /// Push a frame with sub-canvas positioning and compositing.
    ///
    /// Default: delegates to [`push_frame()`](FrameEncoder::push_frame).
    fn push_encode_frame(&mut self, frame: EncodeFrame<'_>) -> Result<(), Self::Error> {
        self.push_frame(frame.pixels, frame.duration_ms)
    }

    /// Begin a new frame (for row-level building).
    ///
    /// # Errors
    ///
    /// Default returns [`UnsupportedOperation::RowLevelFrameEncode`](crate::UnsupportedOperation::RowLevelFrameEncode).
    fn begin_frame(&mut self, _duration_ms: u32) -> Result<(), Self::Error> {
        Err(crate::UnsupportedOperation::RowLevelFrameEncode.into())
    }

    /// Push rows into the current frame (after begin_frame).
    ///
    /// # Errors
    ///
    /// Default returns [`UnsupportedOperation::RowLevelFrameEncode`](crate::UnsupportedOperation::RowLevelFrameEncode).
    fn push_rows(&mut self, _rows: PixelSlice<'_>) -> Result<(), Self::Error> {
        Err(crate::UnsupportedOperation::RowLevelFrameEncode.into())
    }

    /// End the current frame (after pushing all rows).
    ///
    /// # Errors
    ///
    /// Default returns [`UnsupportedOperation::RowLevelFrameEncode`](crate::UnsupportedOperation::RowLevelFrameEncode).
    fn end_frame(&mut self) -> Result<(), Self::Error> {
        Err(crate::UnsupportedOperation::RowLevelFrameEncode.into())
    }

    /// Encode a frame by pulling rows from a source callback.
    ///
    /// # Errors
    ///
    /// Default returns [`UnsupportedOperation::PullFrameEncode`](crate::UnsupportedOperation::PullFrameEncode).
    fn pull_frame(
        &mut self,
        _duration_ms: u32,
        _source: &mut dyn FnMut(u32, PixelSliceMut<'_>) -> usize,
    ) -> Result<(), Self::Error> {
        Err(crate::UnsupportedOperation::PullFrameEncode.into())
    }

    /// Set animation loop count.
    ///
    /// - `Some(0)` = loop forever
    /// - `Some(n)` = loop `n` times
    /// - `None` = format default
    ///
    /// Default no-op.
    fn with_loop_count(&mut self, _count: Option<u32>) {}

    /// Finalize animation. Returns encoded output.
    fn finish(self) -> Result<EncodeOutput, Self::Error>;
}

/// Trivial rejection impl — codecs that don't support animation set
/// `type FrameEnc = ()` and `frame_encoder()` returns an error.
impl FrameEncoder for () {
    type Error = crate::UnsupportedOperation;

    fn push_frame(&mut self, _: PixelSlice<'_>, _: u32) -> Result<(), Self::Error> {
        Err(crate::UnsupportedOperation::AnimationEncode)
    }

    fn finish(self) -> Result<EncodeOutput, Self::Error> {
        Err(crate::UnsupportedOperation::AnimationEncode)
    }
}
