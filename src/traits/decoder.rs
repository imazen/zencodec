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
#[non_exhaustive]
pub enum PushStatus {
    /// The decoder needs more input data before further output scanlines can be produced.
    NeedMoreInput,
    /// The decoder requires specific non-contiguous byte ranges to continue.
    /// Query [`requested_ranges()`](IncrementalDecode::requested_ranges).
    NeedRanges,
    /// One or more batches of decoded scanlines/pixels are ready to be pulled via [`pull_batch`](IncrementalDecode::pull_batch).
    OutputReady,
    /// Backpressure: internal output or intermediate buffer has reached its limit.
    /// The caller must drain output via [`pull_batch`](IncrementalDecode::pull_batch) before pushing further bytes.
    BlockedOnOutput,
    /// Decoding of the image has completed. All scanlines have been yielded.
    Complete,
}

/// A byte span within an image file or stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub struct ByteRange {
    /// Starting byte offset from the beginning of the stream.
    pub offset: u64,
    /// Number of bytes in this range.
    pub length: u64,
}

impl ByteRange {
    /// Create a new byte range.
    #[inline]
    pub const fn new(offset: u64, length: u64) -> Self {
        Self { offset, length }
    }

    /// The ending byte offset (exclusive).
    #[inline]
    pub const fn end(&self) -> u64 {
        self.offset + self.length
    }

    /// Returns `true` if this range is empty (`length == 0`).
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.length == 0
    }

    /// Returns `true` if this range contains `target_offset`.
    #[inline]
    pub const fn contains(&self, target_offset: u64) -> bool {
        target_offset >= self.offset && target_offset < self.end()
    }
}

/// Metadata update notification returned from [`push_chunk`](IncrementalDecode::push_chunk).
///
/// Image containers may deliver metadata across multiple stages:
/// - Basic container / dimensions header
/// - Color profile (ICC, CICP)
/// - Supplementary or trailing metadata (EXIF, XMP, gain maps, text chunks)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum MetadataUpdate {
    /// No new metadata was discovered during this push.
    #[default]
    None,
    /// Image dimensions and basic format header are now available in [`info()`](IncrementalDecode::info).
    HeaderReady,
    /// Color profile or color primaries (ICC / CICP) are now available or refined.
    ColorReady,
    /// Supplementary or trailing metadata (EXIF, XMP, gain map) was discovered or updated.
    Updated,
}

impl MetadataUpdate {
    /// Returns `true` if any new metadata was discovered during this push.
    #[inline]
    pub const fn is_some(&self) -> bool {
        !matches!(self, Self::None)
    }

    /// Returns `true` if basic image dimensions and format are ready.
    #[inline]
    pub const fn is_header_ready(&self) -> bool {
        matches!(self, Self::HeaderReady | Self::ColorReady | Self::Updated)
    }
}

/// Outcome of a [`push_chunk`](IncrementalDecode::push_chunk) call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct PushOutcome {
    /// Number of bytes consumed from the provided chunk.
    pub bytes_consumed: usize,
    /// Decoder status after processing those bytes.
    pub status: PushStatus,
    /// Metadata update event discovered during this push, if any.
    pub metadata: MetadataUpdate,
}

impl PushOutcome {
    /// Create a new push outcome with `metadata = MetadataUpdate::None`.
    #[inline]
    pub const fn new(bytes_consumed: usize, status: PushStatus) -> Self {
        Self {
            bytes_consumed,
            status,
            metadata: MetadataUpdate::None,
        }
    }

    /// Sets the metadata update event.
    #[inline]
    pub const fn with_metadata(mut self, metadata: MetadataUpdate) -> Self {
        self.metadata = metadata;
        self
    }

    /// Sets whether the image header is ready (convenience helper).
    ///
    /// Setting `true` sets `metadata` to [`MetadataUpdate::HeaderReady`]
    /// unless a more specific metadata update (such as [`MetadataUpdate::ColorReady`]) is already set.
    #[inline]
    pub const fn with_header_ready(mut self, header_ready: bool) -> Self {
        if header_ready {
            if matches!(self.metadata, MetadataUpdate::None) {
                self.metadata = MetadataUpdate::HeaderReady;
            }
        } else {
            self.metadata = MetadataUpdate::None;
        }
        self
    }

    /// Convenience check: returns `true` if this push provided (or refined) header metadata.
    #[inline]
    pub const fn header_ready(&self) -> bool {
        self.metadata.is_header_ready()
    }
}

/// Metadata describing a batch of decoded scanlines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct BatchMeta {
    /// Starting row index (top row of this batch).
    pub y: u32,
    /// Progressive pass index (0 for baseline / non-progressive decode).
    pub pass: u16,
    /// Whether this batch represents the final pass for these scanlines.
    pub is_final: bool,
}

impl BatchMeta {
    /// Create a new batch metadata with baseline defaults (`pass = 0`, `is_final = true`).
    #[inline]
    pub const fn new(y: u32, is_final: bool) -> Self {
        Self {
            y,
            pass: 0,
            is_final,
        }
    }

    /// Specify the progressive pass index.
    #[inline]
    pub const fn with_pass(mut self, pass: u16) -> Self {
        self.pass = pass;
        self
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
    /// - `stop`: optional cancellation token (`enough::Stop`). A `&dyn Pulse`
    ///   can be passed directly since `Pulse: Stop`.
    fn push_chunk(
        &mut self,
        chunk: &[u8],
        is_eof: bool,
        stop: Option<&dyn Stop>,
    ) -> Result<PushOutcome, Self::Error>;

    /// Push a chunk with an optional `&dyn Pulse` for cancellation and progress reporting.
    ///
    /// Default implementation delegates to [`push_chunk`](IncrementalDecode::push_chunk).
    fn push_chunk_pulse(
        &mut self,
        chunk: &[u8],
        is_eof: bool,
        pulse: Option<&dyn how_far::Pulse>,
    ) -> Result<PushOutcome, Self::Error> {
        self.push_chunk(chunk, is_eof, pulse.map(|p| p as &dyn Stop))
    }

    /// Returns non-contiguous byte ranges requested by the decoder to make progress.
    ///
    /// When [`push_chunk`](IncrementalDecode::push_chunk) or [`push_range`](IncrementalDecode::push_range)
    /// returns [`PushStatus::NeedRanges`], this method yields the specific spans of bytes
    /// needed from the input stream.
    ///
    /// For purely sequential decoders or when sequential input is sufficient,
    /// this returns an empty slice.
    fn requested_ranges(&self) -> &[ByteRange] {
        &[]
    }

    /// Push a chunk of input bytes tagged with an absolute stream byte offset.
    ///
    /// - `offset`: the absolute 0-based byte position of the first byte of `chunk` in the source stream.
    /// - `chunk`: slice of input bytes at `offset`.
    /// - `is_eof`: true if no further bytes will follow in the entire stream.
    /// - `stop`: optional cooperative cancellation token.
    ///
    /// The default implementation discards `offset` and delegates to [`push_chunk`](IncrementalDecode::push_chunk).
    fn push_range(
        &mut self,
        offset: u64,
        chunk: &[u8],
        is_eof: bool,
        stop: Option<&dyn Stop>,
    ) -> Result<PushOutcome, Self::Error> {
        let _ = offset;
        self.push_chunk(chunk, is_eof, stop)
    }

    /// Push a chunk of input bytes tagged with an absolute stream offset, with an optional [`Pulse`](how_far::Pulse).
    ///
    /// Default implementation delegates to [`push_range`](IncrementalDecode::push_range).
    fn push_range_pulse(
        &mut self,
        offset: u64,
        chunk: &[u8],
        is_eof: bool,
        pulse: Option<&dyn how_far::Pulse>,
    ) -> Result<PushOutcome, Self::Error> {
        self.push_range(offset, chunk, is_eof, pulse.map(|p| p as &dyn Stop))
    }

    /// Pull the next batch of decoded scanlines, if available.
    ///
    /// Returns `Ok(Some((y, strip)))` with row offset and pixel slice,
    /// or `Ok(None)` if more input is needed before another batch can be produced.
    fn pull_batch(&mut self) -> Result<Option<(u32, PixelSlice<'_>)>, Self::Error>;

    /// Pull the next batch of decoded scanlines with pass and finality metadata.
    ///
    /// Default implementation calls [`pull_batch`](IncrementalDecode::pull_batch)
    /// and tags the batch as `pass = 0`, `is_final = true`.
    fn pull_batch_meta(&mut self) -> Result<Option<(BatchMeta, PixelSlice<'_>)>, Self::Error> {
        match self.pull_batch()? {
            Some((y, slice)) => Ok(Some((BatchMeta::new(y, true), slice))),
            None => Ok(None),
        }
    }

    /// Decoupled-lifetime pull: push decoded scanlines directly into a caller-owned sink.
    ///
    /// Writes the next available batch into `sink` via [`provide_next_buffer`](DecodeRowSink::provide_next_buffer).
    /// Returns `Ok(Some(rows_written))` or `Ok(None)` if more input is needed.
    ///
    /// Default implementation pulls from [`pull_batch`](IncrementalDecode::pull_batch)
    /// and copies rows into the sink.
    fn pull_batch_into(
        &mut self,
        sink: &mut dyn crate::DecodeRowSink,
    ) -> Result<Option<u32>, Self::Error> {
        match self.pull_batch()? {
            Some((y, slice)) => {
                let h = slice.rows();
                let w = slice.width();
                let desc = slice.descriptor();
                let mut dst = sink
                    .provide_next_buffer(y, h, w, desc)
                    .map_err(Self::wrap_sink_error)?;
                for row_idx in 0..h {
                    dst.row_mut(row_idx).copy_from_slice(slice.row(row_idx));
                }
                Ok(Some(h))
            }
            None => Ok(None),
        }
    }

    /// Wrap a [`SinkError`] into this decoder's error type.
    fn wrap_sink_error(err: SinkError) -> Self::Error;

    /// Image metadata, available as soon as sufficient header bytes have been pushed.
    fn info(&self) -> Option<&ImageInfo>;

    /// Negotiated output pixel descriptor, available once the header has been parsed.
    fn output_descriptor(&self) -> Option<zenpixels::PixelDescriptor> {
        None
    }

    /// How this decoder reads its input, so a driver can choose a
    /// buffering policy when the source is an unseekable stream.
    fn access_pattern(&self) -> crate::source::AccessPattern {
        crate::source::AccessPattern::Sequential
    }

    /// Whether decoding has fully completed.
    fn is_complete(&self) -> bool;
}

/// Seekable (pull-based) decoder over a random-access source.
///
/// The codec reads exactly the ranges it needs from a
/// [`DynReadAt`](crate::source::DynReadAt) in straight-line code and writes
/// rows directly into the caller's sink. Crop or region intent is bound on
/// the job (for example `with_crop_hint`); the sink's coordinate space is
/// the output image, so no x-offset crosses this boundary.
///
/// Created by [`DecodeJob::dyn_seekable_decoder()`](super::DecodeJob::dyn_seekable_decoder).
pub trait SeekableDecode {
    /// The codec-specific error type.
    type Error: core::error::Error + Send + Sync + 'static;

    /// Decode from `source` into `sink`. Called once; the codec drives
    /// the whole decode, reading ranges as it needs them.
    fn run(
        &mut self,
        source: &mut dyn crate::source::DynReadAt,
        sink: &mut dyn crate::DecodeRowSink,
        stop: Option<&dyn Stop>,
    ) -> Result<OutputInfo, Self::Error>;

    /// Wrap a [`SinkError`] into this decoder's error type.
    fn wrap_sink_error(err: SinkError) -> Self::Error;

    /// Wrap a source (I/O, range, or buffering) failure into this
    /// decoder's error type.
    fn wrap_source_error(err: crate::traits::BoxedError) -> Self::Error;

    /// Image metadata, available after construction or after `run` has
    /// read the header.
    fn info(&self) -> Option<&ImageInfo>;

    /// Negotiated output pixel descriptor, when known before `run`.
    fn output_descriptor(&self) -> Option<zenpixels::PixelDescriptor> {
        None
    }

    /// How this decoder reads its source. Drivers use this to pick a
    /// [`BufferPolicy`](crate::source::BufferPolicy) for streams.
    fn access_pattern(&self) -> crate::source::AccessPattern {
        crate::source::AccessPattern::RandomAccess
    }
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
