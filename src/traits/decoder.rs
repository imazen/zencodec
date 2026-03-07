//! Decode execution traits: one-shot, streaming, and animation.

use crate::{DecodeFrame, DecodeOutput, ImageInfo, OutputInfo};
use zenpixels::PixelSlice;

/// Single-image decode. Returns owned pixels.
///
/// Created by [`DecodeJob::decoder()`](super::DecodeJob::decoder) with input
/// data and format preferences already bound.
pub trait Decode: Sized {
    /// The codec-specific error type.
    type Error: core::error::Error + Send + Sync + 'static;

    /// Decode to owned pixels.
    ///
    /// Input data and format preferences were bound when the decoder
    /// was created via [`DecodeJob::decoder()`](super::DecodeJob::decoder).
    fn decode(self) -> Result<DecodeOutput, Self::Error>;
}

/// Streaming scanline-batch decode.
///
/// The decoder yields strips of scanlines at whatever height it prefers:
/// MCU height for JPEG, full image for simple formats, single scanline
/// for PNG, etc. The caller pulls batches until `None` is returned.
///
/// Created by [`DecodeJob::streaming_decoder()`](super::DecodeJob::streaming_decoder)
/// with input data and format preferences already bound.
///
/// # Usage
///
/// ```text
/// let job = config.job();
/// let info = job.output_info(data)?;
/// let mut dec = job.streaming_decoder(&[], data)?;
/// while let Some((y, strip)) = dec.next_batch()? {
///     // process strip.rows() scanlines starting at row y
/// }
/// ```
pub trait StreamingDecode {
    /// The codec-specific error type.
    type Error: core::error::Error + Send + Sync + 'static;

    /// Pull the next batch of scanlines.
    ///
    /// Returns `Ok(Some((y, strip)))` with the row offset and pixel data,
    /// or `Ok(None)` when the image is fully decoded.
    ///
    /// Format preferences were bound at construction. The format remains
    /// consistent across all batches.
    fn next_batch(&mut self) -> Result<Option<(u32, PixelSlice<'_>)>, Self::Error>;

    /// Image metadata, available after construction.
    fn info(&self) -> &ImageInfo;
}

/// Trivial rejection impl — codecs that don't support streaming set
/// `type StreamDec = ()` and `streaming_decoder()` returns an error.
impl StreamingDecode for () {
    type Error = crate::UnsupportedOperation;

    fn next_batch(&mut self) -> Result<Option<(u32, PixelSlice<'_>)>, Self::Error> {
        Err(crate::UnsupportedOperation::RowLevelDecode)
    }

    fn info(&self) -> &ImageInfo {
        // This is unreachable — streaming_decoder() returns Err before
        // the caller can call info(). But we need the impl for Sized.
        panic!("StreamingDecode not supported");
    }
}

/// Animation decode. Returns owned frames.
///
/// Created by [`DecodeJob::frame_decoder()`](super::DecodeJob::frame_decoder)
/// with input data and format preferences already bound.
pub trait FrameDecode: Sized {
    /// The codec-specific error type.
    type Error: core::error::Error + Send + Sync + 'static;

    /// Number of frames, if known without decoding.
    fn frame_count(&self) -> Option<u32> {
        None
    }

    /// Animation loop count from the container.
    ///
    /// - `Some(0)` = loop forever
    /// - `Some(n)` = loop `n` times
    /// - `None` = unknown or not specified
    fn loop_count(&self) -> Option<u32> {
        None
    }

    /// Pull next frame. Returns `None` when all frames consumed.
    ///
    /// Format preferences were bound at construction.
    fn next_frame(&mut self) -> Result<Option<DecodeFrame>, Self::Error>;

    /// Decode next frame directly into a caller-owned sink (push model).
    ///
    /// Returns `Ok(Some(info))` with frame metadata, or `Ok(None)` when
    /// all frames are consumed.
    ///
    /// Default implementation calls [`next_frame()`](FrameDecode::next_frame)
    /// and copies the result into the sink. Codecs with native row streaming
    /// should override for zero-copy.
    fn next_frame_to_sink(
        &mut self,
        sink: &mut dyn crate::DecodeRowSink,
    ) -> Result<Option<OutputInfo>, Self::Error> {
        let frame = match self.next_frame()? {
            Some(f) => f,
            None => return Ok(None),
        };
        let ps = frame.pixels();
        let desc = ps.descriptor();
        let w = ps.width();
        let h = ps.rows();

        let mut dst = sink.demand(0, h, w, desc);
        for row in 0..h {
            dst.row_mut(row).copy_from_slice(ps.row(row));
        }

        let info = frame.info();
        Ok(Some(OutputInfo::full_decode(info.width, info.height, desc)))
    }
}

/// Trivial rejection impl — codecs that don't support animation set
/// `type FrameDec = ()` and `frame_decoder()` returns an error.
impl FrameDecode for () {
    type Error = crate::UnsupportedOperation;

    fn next_frame(&mut self) -> Result<Option<DecodeFrame>, Self::Error> {
        Err(crate::UnsupportedOperation::AnimationDecode)
    }
}
