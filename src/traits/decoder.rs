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

/// A byte span within an image file or stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub struct ByteRange {
    /// Starting byte offset from the beginning of the stream.
    pub offset: u64,
    /// Number of bytes in this range. Zero means "any amount from `offset`
    /// onward": a sequential decoder names its cursor this way.
    pub length: u64,
}

impl ByteRange {
    /// Create a new byte range.
    #[inline]
    pub const fn new(offset: u64, length: u64) -> Self {
        Self { offset, length }
    }

    /// A sequential request: any number of bytes from `offset` onward.
    #[inline]
    pub const fn from_offset(offset: u64) -> Self {
        Self { offset, length: 0 }
    }

    /// The ending byte offset (exclusive).
    #[inline]
    pub const fn end(&self) -> u64 {
        self.offset.saturating_add(self.length)
    }

    /// Returns `true` if this range has zero length (a sequential request,
    /// or nothing at all).
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

/// Metadata update event reported by [`Step::Metadata`].
///
/// Image containers deliver metadata across stages: the dimensions header,
/// then colour signalling, then supplementary or trailing payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum MetadataUpdate {
    /// No new metadata.
    #[default]
    None,
    /// Dimensions and format header are available in
    /// [`info()`](IncrementalDecode::info).
    HeaderReady,
    /// Colour profile or primaries (ICC / CICP) are available or refined.
    ColorReady,
    /// Supplementary or trailing metadata (EXIF, XMP, gain map) was
    /// discovered or updated.
    Updated,
}

impl MetadataUpdate {
    /// Returns `true` if any new metadata was discovered.
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

/// Metadata describing a batch of decoded scanlines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct BatchMeta {
    /// Starting row index (top row of this batch).
    pub y: u32,
    /// Progressive pass index (0 for baseline / non-progressive decode).
    pub pass: u16,
    /// Whether this batch is the final form of these scanlines.
    pub is_final: bool,
}

impl BatchMeta {
    /// Batch metadata with baseline defaults (`pass = 0`).
    #[inline]
    pub const fn new(y: u32, is_final: bool) -> Self {
        Self {
            y,
            pass: 0,
            is_final,
        }
    }

    /// Final rows starting at `y` (`pass = 0`, `is_final = true`).
    #[inline]
    pub const fn final_rows(y: u32) -> Self {
        Self::new(y, true)
    }

    /// Specify the progressive pass index.
    #[inline]
    pub const fn with_pass(mut self, pass: u16) -> Self {
        self.pass = pass;
        self
    }
}

/// What an incremental decoder did, or needs, on one [`step`](IncrementalDecode::step).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Step<'a> {
    /// The decoder needs these bytes before it can continue. Feed them with
    /// [`feed`](IncrementalDecode::feed), then step again.
    ///
    /// A sequential decoder names its cursor with a zero-length range
    /// ("any amount from here"). A range-aware decoder names exact spans;
    /// drivers may coalesce near-adjacent spans and fetch them in one round.
    Need(&'a [ByteRange]),
    /// The decoder needs the total source length. Announce it with an empty
    /// `feed(len, &[], true)`, then step again.
    NeedLen,
    /// The decoder wrote `rows` rows starting at `meta.y` into the sink.
    /// Step again for more.
    Rows {
        /// Row position, pass and finality of the rows just written.
        meta: BatchMeta,
        /// Number of rows written.
        rows: u32,
    },
    /// Metadata became available or was refined. Step again.
    Metadata(MetadataUpdate),
    /// Decoding finished; the sink has been told to `finish`.
    Done(OutputInfo),
}

impl Step<'_> {
    /// Returns `true` for [`Step::Done`].
    #[inline]
    pub const fn is_done(&self) -> bool {
        matches!(self, Self::Done(_))
    }
}

/// Incremental (sans-I/O) decoder: a resumable decode driven by whoever
/// holds the bytes.
///
/// The decoder performs no I/O. A driver feeds it bytes it already has and
/// steps it; each step either reports what the decoder needs next or what
/// it just produced. Rows go straight into the driver's
/// [`DecodeRowSink`](crate::DecodeRowSink), so hand-written decoders write
/// output with no intermediate copy. The same decoder runs under a
/// synchronous file loop, an async network task, an FFI host that pushes
/// buffers, or a single-threaded WASM event loop.
///
/// # Protocol
///
/// ```text
/// let mut dec = job.dyn_incremental_decoder(&[])?;
/// loop {
///     match dec.step(&mut sink, stop)? {
///         Step::Need(ranges) => { /* fetch, then dec.feed(offset, bytes, eof) */ }
///         Step::NeedLen      => { dec.feed(total_len, &[], true)?; }
///         Step::Rows { .. } | Step::Metadata(_) => continue,
///         Step::Done(info)   => break,
///     }
/// }
/// ```
///
/// - [`feed`](Self::feed) only stores bytes; it never decodes and never
///   calls the sink. It returns how many bytes it accepted. A decoder with
///   a bounded input buffer may accept fewer than offered, or none: step,
///   then re-offer the remainder.
/// - [`step`](Self::step) does a bounded amount of work and returns the
///   first event. It calls `sink.begin` before the first rows and
///   `sink.finish` before returning [`Step::Done`]. Provisional rows (a
///   progressive pass that will be refined) are written only when the sink
///   [`accepts_provisional`](crate::DecodeRowSink::accepts_provisional);
///   otherwise the decoder keeps refining internally and writes each row
///   once, in its final form.
/// - Backpressure is the driver's: do not step until the sink can take the
///   next strip. A worker thread blocks in its sink; an event loop simply
///   does not step.
/// - After an `Err`, the decoder is finished. Further steps return an error.
/// - Cancellation: `stop` is checked at least once per step. A rich
///   progress type reaches here by downgrade, since `how_far::Pulse: Stop`.
pub trait IncrementalDecode {
    /// The codec-specific error type.
    type Error: core::error::Error + Send + Sync + 'static;

    /// Offer `bytes` located at absolute `offset`. Returns the number of
    /// bytes accepted, counted from the start of `bytes`.
    ///
    /// `eof` means `offset + bytes.len()` is the total source length; an
    /// empty slice with `eof` announces the length without data. A decoder
    /// counts bytes it will never read (behind its cursor) as accepted and
    /// drops them, so a sequential driver never has to re-offer them.
    fn feed(&mut self, offset: u64, bytes: &[u8], eof: bool) -> Result<usize, Self::Error>;

    /// Advance the decode as far as fed bytes allow, writing finished rows
    /// into `sink`, and report the first event.
    fn step(
        &mut self,
        sink: &mut dyn crate::DecodeRowSink,
        stop: Option<&dyn Stop>,
    ) -> Result<Step<'_>, Self::Error>;

    /// Offer `bytes` at `offset` and advance in one call, letting the
    /// decoder read them in place for the duration of this step.
    ///
    /// Returns the bytes accepted (decoded in place, or retained) and the
    /// step event; the driver offers the next call from `offset + accepted`.
    /// A decoder with nothing buffered can decode straight from the
    /// caller's buffer here and retain only a tail that straddles the end,
    /// which removes the copy that `feed` + `step` imply. The default is
    /// exactly `feed` then `step`.
    fn feed_and_step(
        &mut self,
        offset: u64,
        bytes: &[u8],
        eof: bool,
        sink: &mut dyn crate::DecodeRowSink,
        stop: Option<&dyn Stop>,
    ) -> Result<(usize, Step<'_>), Self::Error> {
        let accepted = self.feed(offset, bytes, eof)?;
        let step = self.step(sink, stop)?;
        Ok((accepted, step))
    }

    /// Wrap a [`SinkError`] into this decoder's error type.
    fn wrap_sink_error(err: SinkError) -> Self::Error;

    /// Image metadata, available once enough header bytes have been fed
    /// and stepped.
    fn info(&self) -> Option<&ImageInfo>;

    /// Negotiated output pixel descriptor, available once the header has
    /// been parsed.
    fn output_descriptor(&self) -> Option<zenpixels::PixelDescriptor> {
        None
    }

    /// How this decoder reads its input, so a driver can size a buffer or
    /// choose a fetch strategy before the first step.
    fn access_pattern(&self) -> crate::source::AccessPattern {
        crate::source::AccessPattern::Sequential
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
