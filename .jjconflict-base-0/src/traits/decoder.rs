//! Decode execution traits: one-shot, streaming, and animation.

use crate::output::{AnimationFrame, OwnedAnimationFrame};
use crate::sink::SinkError;
use crate::{DecodeOutput, ImageInfo, OutputInfo};
use enough::Stop;
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

/// Status returned after pushing input bytes to an incremental decoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PushStatus {
    /// The decoder needs more input data before further output scanlines can be produced.
    NeedMoreInput,
    /// One or more batches of decoded scanlines/pixels are ready to be pulled via [`pull_batch`](IncrementalDecode::pull_batch).
    OutputReady,
    /// Decoding of the image has completed. All scanlines have been yielded.
    Complete,
}

/// Outcome of a [`push_chunk`](IncrementalDecode::push_chunk) call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PushOutcome {
    /// Number of bytes consumed from the provided chunk.
    pub bytes_consumed: usize,
    /// Decoder status after processing those bytes.
    pub status: PushStatus,
}

impl PushOutcome {
    /// Create a new push outcome.
    #[inline]
    pub const fn new(bytes_consumed: usize, status: PushStatus) -> Self {
        Self {
            bytes_consumed,
            status,
        }
    }
}

/// Incremental (Sans-I/O) scanline/batch decoder.
///
/// Accepts compressed input bytes in arbitrary chunks without performing any I/O,
/// and yields strips/scanlines as soon as sufficient data has been pushed.
///
/// # Usage
///
/// ```text
/// let mut dec = job.into_incremental_decoder(&[])?;
/// while !dec.is_complete() {
///     let chunk = read_chunk_from_network();
///     let outcome = dec.push_chunk(chunk, is_eof, stop)?;
///     while let Some((y, strip)) = dec.pull_batch()? {
///         // process scanlines starting at row y
///     }
/// }
/// ```
pub trait IncrementalDecode {
    /// The codec-specific error type.
    type Error: core::error::Error + Send + Sync + 'static;

    /// Push a chunk of compressed input bytes into the decoder.
    ///
    /// - `chunk`: slice of input bytes.
    /// - `is_eof`: true if no further bytes will follow.
    /// - `stop`: optional cancellation token.
    fn push_chunk(
        &mut self,
        chunk: &[u8],
        is_eof: bool,
        stop: Option<&dyn Stop>,
    ) -> Result<PushOutcome, Self::Error>;

    /// Pull the next batch of decoded scanlines, if available.
    ///
    /// Returns `Ok(Some((y, strip)))` with row offset and pixel slice,
    /// or `Ok(None)` if more input is needed before another batch can be produced.
    fn pull_batch(&mut self) -> Result<Option<(u32, PixelSlice<'_>)>, Self::Error>;

    /// Image metadata, available as soon as sufficient header bytes have been pushed.
    ///
    /// Returns `Some(&ImageInfo)` once known, or `None` if more input is needed.
    fn info(&self) -> Option<&ImageInfo>;

    /// Whether decoding has fully completed.
    fn is_complete(&self) -> bool;
}

/// Full-frame composited animation decode.
///
/// The decoder composites internally (handling disposal, blending,
/// sub-canvas positioning, reference slots, etc.) and yields
/// full-canvas frames ready for display.
///
/// Created by [`DecodeJob::animation_frame_decoder()`](super::DecodeJob::animation_frame_decoder)
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
/// [`render_next_frame()`](AnimationFrameDecoder::render_next_frame) returns a
/// [`AnimationFrame`] that borrows the decoder's internal canvas — zero-copy
/// but invalidated by the next call.
/// [`render_next_frame_owned()`](AnimationFrameDecoder::render_next_frame_owned)
/// copies to an [`OwnedAnimationFrame`] for independent ownership.
///
/// # Cooperative cancellation
///
/// Each render method takes an `Option<&dyn Stop>` token for cooperative
/// cancellation. The codec checks this token periodically during decode
/// and returns early with [`StopReason`](enough::StopReason) if
/// cancellation is requested.
///
/// Because `AnimationFrameDec: 'static`, the decoder cannot borrow the
/// job's stop token. Instead, the caller passes a stop token per call.
/// Codecs that also stored an owned stop at construction time (e.g. via
/// [`Stopper`](https://docs.rs/almost-enough/latest/almost_enough/struct.Stopper.html))
/// can combine the two with
/// [`OrStop`](https://docs.rs/almost-enough/latest/almost_enough/struct.OrStop.html).
/// Pass `None` when cancellation is not needed.
pub trait AnimationFrameDecoder: Sized {
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
    /// - `Some(n)` = play exactly `n` times in total, including the first play
    /// - `None` = unknown or not specified
    ///
    /// GIF's positive native repeat count must be incremented when exposed here.
    fn loop_count(&self) -> Option<u32> {
        None
    }

    /// Render the next composited full-canvas frame.
    ///
    /// Returns `Ok(Some(frame))` with the composited frame borrowing the
    /// decoder's internal canvas, or `Ok(None)` when all frames are consumed.
    ///
    /// The returned [`AnimationFrame`] borrows the decoder's canvas buffer.
    /// Calling this method again invalidates the previous frame.
    ///
    /// Pass `None` if cancellation is not needed.
    fn render_next_frame(
        &mut self,
        stop: Option<&dyn Stop>,
    ) -> Result<Option<AnimationFrame<'_>>, Self::Error>;

    /// Render the next frame as an owned copy.
    ///
    /// Default implementation calls [`render_next_frame()`](AnimationFrameDecoder::render_next_frame)
    /// and copies the pixel data. Codecs that produce owned data natively
    /// may override for efficiency.
    ///
    /// Pass `None` if cancellation is not needed.
    fn render_next_frame_owned(
        &mut self,
        stop: Option<&dyn Stop>,
    ) -> Result<Option<OwnedAnimationFrame>, Self::Error> {
        match self.render_next_frame(stop)? {
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
    /// [`zencodec::helpers::copy_frame_to_sink()`](crate::helpers::copy_frame_to_sink) as a fallback.
    ///
    /// Pass `None` if cancellation is not needed.
    fn render_next_frame_to_sink(
        &mut self,
        stop: Option<&dyn Stop>,
        sink: &mut dyn crate::DecodeRowSink,
    ) -> Result<Option<OutputInfo>, Self::Error>;
}
