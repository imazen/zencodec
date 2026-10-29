//! Type-erased single-image and animation encoder traits.

use crate::EncodeOutput;
use alloc::vec::Vec;

use zenpixels::{PixelDescriptor, PixelSlice, PixelSliceMut};

/// Type-erased single-image encoder.
///
/// Accepts any pixel format at runtime via [`PixelSlice`] (type-erased).
/// The encoder dispatches internally based on the pixel descriptor.
///
/// Mutually exclusive usage paths:
/// - [`encode()`](Encoder::encode) — all at once, consumes self
/// - [`push_rows()`](Encoder::push_rows) + [`finish()`](Encoder::finish), after an
///   optional [`begin()`](Encoder::begin) — caller pushes rows, and with `begin`
///   may send encoded bytes as they arrive ([`pending_output`](Encoder::pending_output))
/// - [`encode_from()`](Encoder::encode_from) — legacy one-call callback
/// - [`begin_declared()`](Encoder::begin_declared) + [`pull_rows()`](Encoder::pull_rows)
///   + [`finish_output()`](Encoder::finish_output) — declared encoder-driven passes
///
/// Once `begin` or `push_rows` has been called, `encode` and `encode_from`
/// are errors. After an error from `begin`, `push_rows` or `finish`, the
/// encode cannot complete: bytes already handed out are a prefix of a file
/// that will never be finished, so a driver that has sent any must abort its
/// transport (close the connection, abort the upload) rather than end it
/// normally. The one exception is a failed `begin`, which leaves the encoder
/// exactly as it was.
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
    /// Rules every implementation follows:
    ///
    /// - **Declaration.** `begin` answers as [`encode`](Self::encode) of a
    ///   `width` x `height` image would (a size `encode` refuses, `begin`
    ///   refuses). A second `begin` is an error. A failed `begin` leaves the
    ///   encoder unchanged.
    /// - **Rows.** After `begin`, pushing more or fewer rows than declared,
    ///   or rows of another width or descriptor, is an error at `push_rows`
    ///   or `finish`. Colour comes from the job (metadata, policy); rows whose
    ///   pixel colour context disagrees with it may be refused.
    /// - **Lag.** A streaming lag is below `height` (an encoder that would
    ///   hold every row reports `AT_FINISH`), counts pushed rows whatever
    ///   the push size, and counts a row as done once its bytes are in
    ///   `pending_output`. A coder without row-aligned output (deflate, rANS
    ///   blocks) claims a lag by flushing on a row schedule fixed at `begin`.
    /// - **Bytes.** Draining changes memory, never output: what was taken
    ///   plus `finish`'s return equals the same encode left undrained. `begin`
    ///   may choose a framing or flush schedule that makes the file differ
    ///   from `encode()`'s (one IDAT chunk per strip instead of one), never
    ///   different pixels, and never a different quality or format
    ///   configuration.
    /// - **Limits.** `max_output_bytes` bounds the whole file, bytes already
    ///   handed out included. A streaming encode nobody drains holds its
    ///   output like an `AT_FINISH` one, and the memory limit may refuse it.
    ///
    /// Calling `begin` is optional; without it, encoders behave as before
    /// and may hold all output until `finish`.
    ///
    /// Default: rejects row-level encoding with
    /// [`UnsupportedOperation::RowLevelEncode`](crate::UnsupportedOperation::RowLevelEncode),
    /// so drivers can use [`encode`](Self::encode) for one-shot-only encoders.
    fn begin(
        &mut self,
        width: u32,
        height: u32,
        descriptor: zenpixels::PixelDescriptor,
    ) -> Result<crate::encode::OutputTiming, Self::Error> {
        let _ = (width, height, descriptor);
        Err(Self::reject(crate::UnsupportedOperation::RowLevelEncode))
    }

    /// Start with the same complete declaration admitted during job planning.
    ///
    /// Validate current color before creating headers; job metadata must not
    /// override the supplied interpretation. Return the actual plan so the
    /// driver can reject drift before source reads or output publication.
    /// A failed call leaves the encoder unchanged, as with `begin`.
    ///
    /// Default: explicit refusal, including descriptor-only declarations. A
    /// legacy `begin_output` cannot establish this complete planning contract.
    fn begin_declared(
        &mut self,
        _declaration: &crate::encode::EncodeDeclaration<'_>,
    ) -> Result<crate::encode::EncodePlan, Self::Error> {
        Err(Self::reject(crate::UnsupportedOperation::EncodePlanning))
    }

    /// Advance an admitted encoder-driven row plan by at most one source fill.
    ///
    /// Use after `begin_declared` commits to `InputRequirement::Replay`.
    /// `Filled` requires exactly one successful fill; `Done` requires none and
    /// complete pass coverage. Do not mix this protocol with `push_rows` or
    /// one-shot encoding. The existing `pending_output` / `consume_output` and
    /// `finish_output` methods retain their output ownership contracts.
    ///
    /// `None` adds no per-call control; the encoder must still honor retained
    /// job cancellation, including when `Some(NoPulse)` is supplied.
    /// Check Pulse around native work; byte timing does not bound retained
    /// memory. A fill failure is terminal. Drivers retain its original cause
    /// and refuse further publication even if native code swallows the marker.
    /// This synchronous loan does not implement pending asynchronous input.
    ///
    /// Default: refuse without calling the source or changing encoder state.
    fn pull_rows(
        &mut self,
        _rows: &mut dyn crate::encode::EncodeRows,
        _pulse: Option<&dyn crate::Pulse>,
    ) -> Result<crate::encode::PullProgress, Self::Error> {
        Err(Self::reject(crate::UnsupportedOperation::PullEncode))
    }

    /// Prepare row encoding for actual destination access. The default uses
    /// the existing forward-only begin path without changing configuration.
    /// A patching implementation must refuse Forward before exposing bytes,
    /// report `OutputTiming::with_patches`, and complete with `finish_output`.
    /// Legacy `begin`/`finish` must never silently discard required patches.
    /// Access permits framing choices, never implicit quality/metadata changes.
    fn begin_output(
        &mut self,
        width: u32,
        height: u32,
        descriptor: PixelDescriptor,
        access: crate::output_patch::OutputAccess,
    ) -> Result<crate::encode::OutputTiming, Self::Error> {
        let _ = access;
        self.begin(width, height, descriptor)
    }

    /// Complete negotiated output, retaining ordered patches for the driver.
    /// The default transfers the legacy finish allocation without copying.
    /// The tail retains EncodeOutput format, MIME and extension/statistics data.
    fn finish_output(self) -> Result<crate::output_patch::OutputFinish<EncodeOutput>, Self::Error> {
        Ok(crate::output_patch::OutputFinish::new(
            self.finish()?,
            Vec::new(),
        ))
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
    /// as sent. An `n` larger than the pending length marks all of it.
    /// Default: no-op.
    fn consume_output(&mut self, n: usize) {
        let _ = n;
    }

    /// Append every pending byte to `into` and consume it: the same as
    /// copying [`pending_output`](Self::pending_output) and calling
    /// [`consume_output`](Self::consume_output) with its length.
    ///
    /// An encoder that keeps its output in an
    /// [`OutputBuffer`](crate::encode::OutputBuffer) overrides this with
    /// [`OutputBuffer::take_into`](crate::encode::OutputBuffer::take_into),
    /// so a driver that passes an empty vector receives the encoder's
    /// allocation without a copy. Default: copies.
    fn take_output(&mut self, into: &mut Vec<u8>) {
        let pending = self.pending_output();
        let n = pending.len();
        into.extend_from_slice(pending);
        self.consume_output(n);
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
/// Each method takes an `Option<&dyn crate::Pulse>` token for cooperative
/// cancellation. Because `AnimationFrameEnc: 'static`, the encoder cannot
/// borrow the job's pulse token. Instead, the caller passes a pulse
/// token per call. Codecs that also stored an owned pulse at
/// construction time can combine the two with
/// [`OrStop`](https://docs.rs/almost-enough/latest/almost_enough/struct.OrStop.html).
/// Pass `None` when cancellation is not needed.
pub trait AnimationFrameEncoder: Sized {
    /// The codec-specific error type.
    type Error: core::error::Error + Send + Sync + 'static;

    /// Convert an [`UnsupportedOperation`](crate::UnsupportedOperation) into this
    /// encoder's error type. Used by default method implementations.
    fn reject(op: crate::UnsupportedOperation) -> Self::Error;

    /// Declare the animation before the first frame: its canvas, the
    /// descriptor frames will arrive in, and how many frames will follow
    /// when the caller knows. Returns when this encoder's bytes will become
    /// available.
    ///
    /// The animation counterpart of [`Encoder::begin`], with the same rules
    /// counted in frames:
    ///
    /// - **Declaration.** A canvas the encoder would refuse, `begin`
    ///   refuses, before any frame moves. A second `begin` is an error, and a
    ///   failed `begin` leaves the encoder unchanged. With `frame_count`
    ///   given, finishing after a different number of frames is an error.
    ///   Some formats write the frame count first (APNG's `acTL`) and can
    ///   stream only when it is given.
    /// - **Lag.** A streaming lag counts pushed frames, and a frame is done
    ///   once its bytes are in [`pending_output`](Self::pending_output).
    ///   With `frame_count` given, the lag is below it.
    /// - **Bytes.** Draining changes memory, never output. `begin` may change
    ///   framing (where chunks break), never pixels, timing, or quality
    ///   configuration.
    /// - **Errors.** After an error from `begin`, a push or `finish`, the
    ///   animation cannot complete; a driver that has sent bytes aborts its
    ///   transport.
    ///
    /// Calling `begin` is optional; without it, encoders behave as before.
    ///
    /// Default: accepts the declaration and reports
    /// [`AnimationOutputTiming::AT_FINISH`](crate::encode::AnimationOutputTiming::AT_FINISH).
    fn begin(
        &mut self,
        width: u32,
        height: u32,
        descriptor: PixelDescriptor,
        frame_count: Option<u32>,
    ) -> Result<crate::encode::AnimationOutputTiming, Self::Error> {
        let _ = (width, height, descriptor, frame_count);
        Ok(crate::encode::AnimationOutputTiming::AT_FINISH)
    }

    /// Encoded bytes ready to send, not yet consumed; see
    /// [`Encoder::pending_output`]. Default: empty.
    fn pending_output(&self) -> &[u8] {
        &[]
    }

    /// Mark the first `n` bytes of [`pending_output`](Self::pending_output)
    /// as sent, clamped to its length. Default: no-op.
    fn consume_output(&mut self, n: usize) {
        let _ = n;
    }

    /// Append every pending byte to `into` and consume it; see
    /// [`Encoder::take_output`]. Default: copies.
    fn take_output(&mut self, into: &mut Vec<u8>) {
        let pending = self.pending_output();
        let n = pending.len();
        into.extend_from_slice(pending);
        self.consume_output(n);
    }

    /// Push a complete full-canvas frame.
    ///
    /// Pass `None` if cancellation is not needed.
    fn push_frame(
        &mut self,
        pixels: PixelSlice<'_>,
        duration_ms: u32,
        pulse: Option<&dyn crate::Pulse>,
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
        pulse: Option<&dyn crate::Pulse>,
    ) -> Result<crate::animation::FrameDuration, Self::Error> {
        self.push_frame_exact(pixels, duration, pulse)?;
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
        _pulse: Option<&dyn crate::Pulse>,
    ) -> Result<(), Self::Error> {
        Err(Self::reject(crate::UnsupportedOperation::AnimationTiming))
    }

    /// Finalize animation. Returns the encoded bytes not already consumed
    /// through [`pending_output`](Self::pending_output).
    ///
    /// Pass `None` if cancellation is not needed.
    fn finish(self, pulse: Option<&dyn crate::Pulse>) -> Result<EncodeOutput, Self::Error>;
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
        _: Option<&dyn crate::Pulse>,
    ) -> Result<(), Self::Error> {
        Err(crate::UnsupportedOperation::AnimationEncode)
    }

    fn finish(self, _: Option<&dyn crate::Pulse>) -> Result<EncodeOutput, Self::Error> {
        Err(crate::UnsupportedOperation::AnimationEncode)
    }
}
