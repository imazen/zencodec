//! Type-erased single-image and animation encoder traits.

use crate::EncodeOutput;
use enough::Stop;
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
/// The encoder dispatches internally based on the pixel descriptor.
pub trait Encoder: Sized {
    /// The codec-specific error type.
    type Error: core::error::Error + Send + Sync + 'static;

    /// Convert an [`UnsupportedOperation`](crate::UnsupportedOperation) into this
    /// encoder's error type.
    ///
    /// Used by default method implementations to report unsupported paths.
    /// A typical implementation:
    ///
    /// ```rust,ignore
    /// fn reject(op: UnsupportedOperation) -> Self::Error {
    ///     MyError::from(op).start_at() // or just MyError::from(op)
    /// }
    /// ```
    fn reject(op: crate::UnsupportedOperation) -> Self::Error;

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
    /// Hot-path entry point for RGBA8 data from bitmap sources (e.g.
    /// imageflow). The buffer is mutable — the encoder may modify it
    /// in-place for format adaptation (e.g. RGBA→BGRA channel reorder,
    /// alpha premultiplication, stripping alpha to RGB). Callers must
    /// not rely on the buffer contents after this call returns.
    ///
    /// The default delegates to [`encode()`](Encoder::encode) by wrapping
    /// the raw bytes in a [`PixelSlice`] with the appropriate descriptor.
    /// Codec overrides (e.g. zenjpeg) may bypass `PixelSlice` construction
    /// for zero-overhead encoding from raw buffers.
    ///
    /// # Parameters
    ///
    /// - `data`: raw pixel bytes in RGBA order, 4 bytes per pixel (may be mutated).
    ///   Length must be ≥ `stride_pixels * height * 4`.
    /// - `make_opaque`: if `true`, treat the alpha channel as padding
    ///   (enables RGB fast paths in codecs that don't support alpha).
    ///   Some codecs set all alpha bytes to 255 in-place.
    /// - `width`, `height`: image dimensions in pixels
    /// - `stride_pixels`: row stride in **pixels** (not bytes), must be ≥ `width`.
    ///   Stride in bytes is `stride_pixels * 4`. Callers creating bitmaps
    ///   for this method should allocate rows with `stride_pixels = width`
    ///   (contiguous) unless alignment requirements dictate otherwise.
    ///   Non-contiguous strides (stride > width) are supported but may
    ///   prevent zero-copy fast paths in some codecs.
    fn encode_srgba8(
        self,
        data: &mut [u8],
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
        // which is a caller bug (not untrusted input — encoders receive
        // validated data). Panic is appropriate; UnsupportedOperation::PixelFormat
        // would mask the real issue and mislead callers.
        let pixels = PixelSlice::new(data, width, height, stride_bytes, descriptor).expect(
            "encode_srgba8: data.len() does not match width * height * 4 (stride-adjusted)",
        );
        self.encode(pixels)
    }

    /// Declare the image before the first [`push_rows`](Self::push_rows):
    /// its full dimensions and the descriptor rows will arrive in. Returns
    /// when this encoder's bytes will become available.
    ///
    /// `begin` is where an encoder commits to a row-level encode. It
    /// validates the declaration against its configuration, limits and
    /// metadata, and rejects it here, before any row moves, when this
    /// configuration cannot encode from rows
    /// ([`UnsupportedOperation::RowLevelEncode`](crate::UnsupportedOperation::RowLevelEncode))
    /// or the image is out of bounds. An encoder that returns
    /// [`OutputTiming::streaming`](crate::encode::OutputTiming::streaming)
    /// writes its header now and exposes encoded bytes through
    /// [`pending_output`](Self::pending_output) as rows arrive; one that
    /// returns [`OutputTiming::AT_FINISH`](crate::encode::OutputTiming::AT_FINISH)
    /// keeps `pending_output` empty and returns everything from
    /// [`finish`](Self::finish). Only encoders whose codec declares
    /// [`streaming_output`](crate::encode::EncodeCapabilities::streaming_output)
    /// may return a streaming timing.
    ///
    /// After `begin`, pushing more or fewer rows than declared, or rows of
    /// another width or descriptor, is an error at `push_rows` or `finish`.
    /// The streamed bytes must equal those of the same encode left
    /// undrained: draining changes memory, never output.
    ///
    /// Calling `begin` is optional; without it, encoders behave as before
    /// and may hold all output until `finish`.
    ///
    /// Default: accepts the declaration and reports
    /// [`OutputTiming::AT_FINISH`](crate::encode::OutputTiming::AT_FINISH).
    fn begin(
        &mut self,
        width: u32,
        height: u32,
        descriptor: zenpixels::PixelDescriptor,
    ) -> Result<crate::encode::OutputTiming, Self::Error> {
        let _ = (width, height, descriptor);
        Ok(crate::encode::OutputTiming::AT_FINISH)
    }

    /// Encoded bytes ready to send, not yet consumed.
    ///
    /// Encoders whose [`begin`](Self::begin) reported a streaming
    /// [`OutputTiming`](crate::encode::OutputTiming) fill this as rows are
    /// pushed; a driver writes it out and calls
    /// [`consume_output`](Self::consume_output), keeping output memory
    /// bounded. [`finish`](Self::finish) returns only the bytes not
    /// consumed. Default: empty (all output arrives from `finish`).
    fn pending_output(&self) -> &[u8] {
        &[]
    }

    /// Mark the first `n` bytes of [`pending_output`](Self::pending_output)
    /// as sent. `n` must not exceed its length. Default: no-op.
    fn consume_output(&mut self, n: usize) {
        let _ = n;
    }

    /// Push scanline rows incrementally.
    ///
    /// # Errors
    ///
    /// Default returns [`UnsupportedOperation::RowLevelEncode`](crate::UnsupportedOperation::RowLevelEncode).
    fn push_rows(&mut self, _rows: PixelSlice<'_>) -> Result<(), Self::Error> {
        Err(Self::reject(crate::UnsupportedOperation::RowLevelEncode))
    }

    /// Finalize after push_rows. Returns encoded output.
    ///
    /// # Errors
    ///
    /// Default returns [`UnsupportedOperation::RowLevelEncode`](crate::UnsupportedOperation::RowLevelEncode).
    fn finish(self) -> Result<EncodeOutput, Self::Error> {
        Err(Self::reject(crate::UnsupportedOperation::RowLevelEncode))
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
        Err(Self::reject(crate::UnsupportedOperation::PullEncode))
    }
}

/// Full-frame animation encoder.
///
/// Accepts composited full-canvas frames and handles format-specific
/// optimization (disposal, blending, sub-canvas extraction) internally.
///
/// The encoder accepts [`PixelSlice`] frames at runtime — the pixel
/// descriptor must be consistent across all frames.
///
/// Loop count is set on the [`EncodeJob`](super::EncodeJob) before
/// creating this encoder, because formats write the loop count before
/// frame data.
///
/// # Cooperative cancellation
///
/// Each method takes an `Option<&dyn Stop>` token for cooperative
/// cancellation. Because `AnimationFrameEnc: 'static`, the encoder cannot
/// borrow the job's stop token. Instead, the caller passes a stop
/// token per call. Codecs that also stored an owned stop at
/// construction time can combine the two with
/// [`OrStop`](https://docs.rs/almost-enough/latest/almost_enough/struct.OrStop.html).
/// Pass `None` when cancellation is not needed.
pub trait AnimationFrameEncoder: Sized {
    /// The codec-specific error type.
    type Error: core::error::Error + Send + Sync + 'static;

    /// Convert an [`UnsupportedOperation`](crate::UnsupportedOperation) into this
    /// encoder's error type. Used by default method implementations.
    fn reject(op: crate::UnsupportedOperation) -> Self::Error;

    /// Push a complete full-canvas frame.
    ///
    /// Pass `None` if cancellation is not needed.
    fn push_frame(
        &mut self,
        pixels: PixelSlice<'_>,
        duration_ms: u32,
        stop: Option<&dyn Stop>,
    ) -> Result<(), Self::Error>;

    /// Push a frame using the destination's timing resolution.
    ///
    /// Returns the actual encoded duration so rounding is observable. Codecs
    /// retain cumulative rounding error rather than rounding each frame in
    /// isolation. Zero is not clamped to a player minimum. Field overflow and
    /// unsupported zero-duration frames remain errors. A failure must not
    /// advance timing state. Use [`Self::push_frame_exact`] to forbid rounding.
    /// The compatibility default uses the exact path; adapters with a finite
    /// clock override this with destination-aware quantization.
    fn push_frame_timed(
        &mut self,
        pixels: PixelSlice<'_>,
        duration: crate::animation::FrameDuration,
        stop: Option<&dyn Stop>,
    ) -> Result<crate::animation::FrameDuration, Self::Error> {
        self.push_frame_exact(pixels, duration, stop)?;
        Ok(duration)
    }

    /// Push a frame with its exact encoded source duration.
    ///
    /// Implementations must represent this duration exactly or reject it before
    /// accepting the frame. They must not round, clamp a minimum, saturate, or
    /// wrap the format's timing field. Zero remains zero when supported by the
    /// format; JXL display helpers and codecs requiring positive durations can
    /// reject it. The default rejects even whole milliseconds because legacy
    /// `push_frame` implementations may silently quantize those values.
    fn push_frame_exact(
        &mut self,
        _pixels: PixelSlice<'_>,
        _duration: crate::animation::FrameDuration,
        _stop: Option<&dyn Stop>,
    ) -> Result<(), Self::Error> {
        Err(Self::reject(crate::UnsupportedOperation::AnimationTiming))
    }

    /// Finalize animation. Returns encoded output.
    ///
    /// Pass `None` if cancellation is not needed.
    fn finish(self, stop: Option<&dyn Stop>) -> Result<EncodeOutput, Self::Error>;
}

/// Trivial rejection impl — codecs that don't support animation set
/// `type AnimationFrameEnc = ()` and `animation_frame_encoder()` returns an error.
impl AnimationFrameEncoder for () {
    type Error = crate::UnsupportedOperation;

    fn reject(op: crate::UnsupportedOperation) -> Self::Error {
        op
    }

    fn push_frame(
        &mut self,
        _: PixelSlice<'_>,
        _: u32,
        _: Option<&dyn Stop>,
    ) -> Result<(), Self::Error> {
        Err(crate::UnsupportedOperation::AnimationEncode)
    }

    fn finish(self, _: Option<&dyn Stop>) -> Result<EncodeOutput, Self::Error> {
        Err(crate::UnsupportedOperation::AnimationEncode)
    }
}
