//! Decode execution traits: one-shot, streaming, and animation.

use crate::{DecodeOutput, ImageInfo, OutputInfo};
use crate::output::{FullFrame, OwnedFullFrame};
use crate::sink::SinkError;
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

/// Full-frame composited animation decode.
///
/// The decoder composites internally (handling disposal, blending,
/// sub-canvas positioning, reference slots, etc.) and yields
/// full-canvas frames ready for display.
///
/// Created by [`DecodeJob::full_frame_decoder()`](super::DecodeJob::full_frame_decoder)
/// with input data and format preferences already bound.
///
/// # Frame index
///
/// Frame indices are 0-based and count only displayed frames. Internal
/// compositing helper frames (e.g. JXL zero-duration frames) are consumed
/// internally and never yielded.
///
/// # Borrowed vs owned
///
/// [`render_next_frame()`](FullFrameDecoder::render_next_frame) returns a
/// [`FullFrame`] that borrows the decoder's internal canvas — zero-copy
/// but invalidated by the next call.
/// [`render_next_frame_owned()`](FullFrameDecoder::render_next_frame_owned)
/// copies to an [`OwnedFullFrame`] for independent ownership.
pub trait FullFrameDecoder: Sized {
    /// The codec-specific error type.
    type Error: core::error::Error + Send + Sync + 'static;

    /// Wrap a [`SinkError`] into this decoder's error type.
    ///
    /// Used by default implementations of sink-based methods. Mirrors
    /// the [`reject`](crate::encode::Encoder::reject) pattern on encoders.
    ///
    /// A typical implementation:
    ///
    /// ```rust,ignore
    /// fn wrap_sink_error(err: SinkError) -> Self::Error {
    ///     MyError::Sink(err) // or MyError::External(err.to_string())
    /// }
    /// ```
    fn wrap_sink_error(err: SinkError) -> Self::Error;

    /// Image metadata, available after construction.
    fn info(&self) -> &ImageInfo;

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

    /// Render the next composited full-canvas frame.
    ///
    /// Returns `Ok(Some(frame))` with the composited frame borrowing the
    /// decoder's internal canvas, or `Ok(None)` when all frames are consumed.
    ///
    /// The returned [`FullFrame`] borrows the decoder's canvas buffer.
    /// Calling this method again invalidates the previous frame.
    fn render_next_frame(&mut self) -> Result<Option<FullFrame<'_>>, Self::Error>;

    /// Render the next frame as an owned copy.
    ///
    /// Default implementation calls [`render_next_frame()`](FullFrameDecoder::render_next_frame)
    /// and copies the pixel data. Codecs that produce owned data natively
    /// may override for efficiency.
    fn render_next_frame_owned(&mut self) -> Result<Option<OwnedFullFrame>, Self::Error> {
        match self.render_next_frame()? {
            Some(frame) => Ok(Some(frame.to_owned_frame())),
            None => Ok(None),
        }
    }

    /// Render the next frame directly into a caller-owned sink (push model).
    ///
    /// Returns `Ok(Some(info))` with frame metadata, or `Ok(None)` when
    /// all frames are consumed.
    ///
    /// Codecs with native row streaming should write directly into the sink.
    /// Codecs that render to an internal canvas should call
    /// [`render_frame_to_sink_via_copy()`] as a fallback.
    fn render_next_frame_to_sink(
        &mut self,
        sink: &mut dyn crate::DecodeRowSink,
    ) -> Result<Option<OutputInfo>, Self::Error>;
}

/// Fallback [`render_next_frame_to_sink`](FullFrameDecoder::render_next_frame_to_sink)
/// via [`render_next_frame`](FullFrameDecoder::render_next_frame) + copy.
///
/// Renders the next frame into the decoder's internal canvas, then copies
/// row-by-row into the sink. Correct but adds one full-frame copy.
///
/// # Example
///
/// ```rust,ignore
/// fn render_next_frame_to_sink(
///     &mut self,
///     sink: &mut dyn DecodeRowSink,
/// ) -> Result<Option<OutputInfo>, Self::Error> {
///     render_frame_to_sink_via_copy(self, sink)
/// }
/// ```
pub fn render_frame_to_sink_via_copy<D: FullFrameDecoder>(
    decoder: &mut D,
    sink: &mut dyn crate::DecodeRowSink,
) -> Result<Option<OutputInfo>, D::Error> {
    let frame = match decoder.render_next_frame()? {
        Some(f) => f,
        None => return Ok(None),
    };
    let ps = frame.pixels();
    let desc = ps.descriptor();
    let w = ps.width();
    let h = ps.rows();

    sink.begin(w, h, desc).map_err(D::wrap_sink_error)?;
    let mut dst = sink
        .provide_next_buffer(0, h, w, desc)
        .map_err(D::wrap_sink_error)?;
    for row in 0..h {
        dst.row_mut(row).copy_from_slice(ps.row(row));
    }
    drop(dst);
    sink.finish().map_err(D::wrap_sink_error)?;

    Ok(Some(OutputInfo::full_decode(w, h, desc)))
}
