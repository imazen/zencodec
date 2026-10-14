//! Incremental (sans-I/O) decoding: the contract between a resumable
//! decoder and whatever holds the bytes.
//!
//! A decoder performs no I/O. A driver offers it bytes it already holds and
//! steps it; each step reports how many of those bytes the decoder took and
//! one [`Event`]. The driver owns the input buffer: bytes the decoder did not
//! take are still the driver's, to offer again (with more appended) or to
//! drop. Rows go straight into the driver's [`DecodeRowSink`]. The same
//! decoder runs under a synchronous file loop, an async network task, an FFI
//! host that pushes buffers, or a single-threaded WASM event loop.
//!
//! ```text
//! let mut dec = job.dyn_incremental_decoder(&[])?;
//! let mut input = Input::empty();
//! loop {
//!     let p = dec.step(input, &mut sink, &stop)?;
//!     // drop p.consumed bytes from the front of what was offered
//!     match p.event {
//!         Event::Need => { /* satisfy one of dec.requested(), build the next Input */ }
//!         Event::Done(info) => break,
//!         _ => { /* Rows, Metadata, or a newer informational event: step again */ }
//!     }
//! }
//! ```
//!
//! # The rules
//!
//! **Input.** [`Input`] is a run of bytes at an absolute `offset`, possibly
//! empty, plus the total source length once the driver knows it. A decoder
//! takes a prefix of the input and reports its length in
//! [`Progress::consumed`]: the bytes it decoded in place or copied into its
//! own state, plus any bytes before them it does not need (a gap the driver
//! read across to batch two ranges). Consumed bytes are never offered again.
//! A decoder never fails because it was offered bytes it did not ask for: it
//! takes none of them, or skips them as part of the prefix.
//!
//! **Requests.** After [`Event::Need`], [`IncrementalDecode::requested`]
//! lists what the decoder needs, never empty. The driver satisfies any one
//! request and steps again:
//! - [`Request::AtLeast`]: an input starting at `offset` holding at least
//!   `min_len` bytes, or every remaining byte with `total_len` set if the
//!   source ends first. More is welcome.
//! - [`Request::Range`]: an input holding exactly that span (or a larger
//!   input starting at its offset).
//! - [`Request::Len`]: any input (even empty) carrying `total_len`.
//!
//! A decoder never requests bytes it knows do not exist: once `total_len` is
//! known, a request that cannot be met is a [`StepError::Codec`] error
//! (truncated input), not another `Need`. A decoder makes progress whenever
//! its input satisfies one of its requests; returning `Need` with nothing
//! consumed for an input that satisfied a request is a protocol violation,
//! and drivers report it instead of looping.
//!
//! **Output.** [`begin`](DecodeRowSink::begin) comes before the first rows
//! and [`finish`](DecodeRowSink::finish) before [`Event::Done`]. Every row is
//! written once in its final form through
//! [`provide_next_buffer`](DecodeRowSink::provide_next_buffer), in
//! increasing `y`. A progressive decoder may also write provisional rows
//! (a pass a later one refines) through
//! [`provide_pass_buffer`](DecodeRowSink::provide_pass_buffer), only when
//! the sink's [`accepts_provisional`](DecodeRowSink::accepts_provisional)
//! returned `true` at `begin`. A decoder with no provisional output ignores
//! both.
//!
//! **Termination.** After [`Event::Done`], every step returns `Done` again
//! with nothing consumed. After any error, every step returns
//! [`StepError::Finished`]. `stop` is checked at least once per step; a
//! tripped stop is [`StepError::Cancelled`], and terminal. [`Fused`] enforces
//! all three for any decoder, and
//! [`erase_incremental`](crate::decode::erase_incremental) applies it.
//!
//! **Events.** [`Event`] is non-exhaustive. Variants added later are
//! informational: a driver that does not recognise one steps again. Anything
//! that needs the driver to act is expressed as `Need` plus a [`Request`].

use alloc::boxed::Box;
use core::fmt;

use enough::{Stop, StopReason};

use crate::sink::{DecodeRowSink, SinkError};
use crate::{ImageInfo, OutputInfo};

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

    /// The ending byte offset (exclusive), saturating.
    #[inline]
    pub const fn end(&self) -> u64 {
        self.offset.saturating_add(self.length)
    }

    /// Returns `true` if this range has zero length.
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

/// What an incremental decoder needs before it can continue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Request {
    /// Bytes from `offset` on: an input starting there with at least
    /// `min_len` bytes, or every remaining byte (with `total_len` set) when
    /// the source ends first. More is welcome. A sequential decoder asks
    /// this way; `min_len` is the contiguous span it needs to make progress
    /// without copying (one worst-case row, one marker segment).
    AtLeast {
        /// Absolute offset of the first byte needed.
        offset: u64,
        /// Minimum contiguous bytes needed in one input.
        min_len: usize,
    },
    /// Exactly this span.
    Range(ByteRange),
    /// The total source length, supplied as [`Input::total_len`].
    Len,
}

impl Request {
    /// A request for at least `min_len` bytes from `offset`.
    #[inline]
    pub const fn at_least(offset: u64, min_len: usize) -> Self {
        Self::AtLeast { offset, min_len }
    }

    /// A request for exactly `length` bytes at `offset`.
    #[inline]
    pub const fn range(offset: u64, length: u64) -> Self {
        Self::Range(ByteRange::new(offset, length))
    }

    /// The offset this request starts at, if it asks for bytes.
    #[inline]
    pub const fn offset(&self) -> Option<u64> {
        match self {
            Self::AtLeast { offset, .. } => Some(*offset),
            Self::Range(r) => Some(r.offset),
            Self::Len => None,
        }
    }

    /// Whether `input` satisfies this request.
    pub fn is_satisfied_by(&self, input: &Input<'_>) -> bool {
        match *self {
            Self::AtLeast { offset, min_len } => {
                input.offset == offset
                    && (input.bytes.len() >= min_len || input.total_len == Some(input.end()))
            }
            Self::Range(r) => {
                input.offset == r.offset
                    && (input.bytes.len() as u64 >= r.length
                        || input.total_len == Some(input.end()))
            }
            Self::Len => input.total_len.is_some(),
        }
    }
}

/// Bytes offered to one [`step`](IncrementalDecode::step).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Input<'a> {
    /// Absolute offset of `bytes[0]` in the source.
    pub offset: u64,
    /// The bytes. May be empty.
    pub bytes: &'a [u8],
    /// Total source length, once the driver knows it.
    pub total_len: Option<u64>,
}

impl<'a> Input<'a> {
    /// `bytes` located at `offset`, total length unknown.
    #[inline]
    pub const fn new(offset: u64, bytes: &'a [u8]) -> Self {
        Self {
            offset,
            bytes,
            total_len: None,
        }
    }

    /// No bytes: step without new input.
    #[inline]
    pub const fn empty() -> Self {
        Self {
            offset: 0,
            bytes: &[],
            total_len: None,
        }
    }

    /// The same input, carrying the total source length.
    #[inline]
    pub const fn with_total_len(mut self, total_len: u64) -> Self {
        self.total_len = Some(total_len);
        self
    }

    /// The same input, carrying the total source length if known.
    #[inline]
    pub const fn with_total_len_opt(mut self, total_len: Option<u64>) -> Self {
        self.total_len = total_len;
        self
    }

    /// Absolute offset one past the last byte.
    #[inline]
    pub const fn end(&self) -> u64 {
        self.offset.saturating_add(self.bytes.len() as u64)
    }

    /// Whether these bytes run to the end of the source.
    #[inline]
    pub fn reaches_end(&self) -> bool {
        self.total_len == Some(self.end())
    }
}

/// Metadata update reported by [`Event::Metadata`].
///
/// Image containers deliver metadata across stages: the dimensions header,
/// then colour signalling, then supplementary or trailing payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MetadataUpdate {
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

/// What one [`step`](IncrementalDecode::step) produced.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Event {
    /// The decoder needs bytes or the source length; see
    /// [`requested`](IncrementalDecode::requested).
    Need,
    /// `rows` rows starting at `meta.y` were written into the sink.
    Rows {
        /// Row position, pass and finality of the rows just written.
        meta: BatchMeta,
        /// Number of rows written.
        rows: u32,
    },
    /// Metadata became available or was refined.
    Metadata(MetadataUpdate),
    /// Decoding finished; the sink has been told to `finish`.
    Done(OutputInfo),
}

impl Event {
    /// Returns `true` for [`Event::Done`].
    #[inline]
    pub const fn is_done(&self) -> bool {
        matches!(self, Self::Done(_))
    }
}

/// The result of one [`step`](IncrementalDecode::step).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Progress {
    /// Bytes taken from the front of the offered input.
    pub consumed: usize,
    /// What happened.
    pub event: Event,
}

impl Progress {
    /// A step that took `consumed` bytes and produced `event`.
    #[inline]
    pub const fn new(consumed: usize, event: Event) -> Self {
        Self { consumed, event }
    }

    /// A step that needs more input and took `consumed` bytes.
    #[inline]
    pub const fn need(consumed: usize) -> Self {
        Self::new(consumed, Event::Need)
    }
}

/// Why a [`step`](IncrementalDecode::step) failed. Every error is terminal.
#[derive(Debug)]
#[non_exhaustive]
pub enum StepError<E> {
    /// The decoder rejected the input (malformed, truncated, unsupported,
    /// over a limit).
    Codec(E),
    /// The sink refused rows or failed.
    Sink(SinkError),
    /// The stop token tripped.
    Cancelled(StopReason),
    /// The decoder was stepped again after an error.
    Finished,
}

impl<E> StepError<E> {
    /// Map the codec error, keeping the other variants.
    pub fn map_codec<F>(self, f: impl FnOnce(E) -> F) -> StepError<F> {
        match self {
            Self::Codec(e) => StepError::Codec(f(e)),
            Self::Sink(e) => StepError::Sink(e),
            Self::Cancelled(r) => StepError::Cancelled(r),
            Self::Finished => StepError::Finished,
        }
    }
}

impl<E> From<StopReason> for StepError<E> {
    fn from(r: StopReason) -> Self {
        Self::Cancelled(r)
    }
}

impl<E: fmt::Display> fmt::Display for StepError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Codec(e) => write!(f, "{e}"),
            Self::Sink(e) => write!(f, "row sink: {e}"),
            Self::Cancelled(r) => write!(f, "decode cancelled: {r:?}"),
            Self::Finished => write!(f, "decoder stepped after it failed"),
        }
    }
}

impl<E: core::error::Error + 'static> core::error::Error for StepError<E> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Codec(e) => Some(e),
            Self::Sink(e) => Some(e.as_ref()),
            _ => None,
        }
    }
}

/// A resumable decoder driven by whoever holds the bytes. See the
/// [module documentation](self) for the rules.
///
/// The trait is object-safe: [`DynIncrementalDecoder`] is
/// `dyn IncrementalDecode<Error = ErasedError> + Send`.
pub trait IncrementalDecode {
    /// The codec-specific error type.
    type Error: core::error::Error + Send + Sync + 'static;

    /// Take bytes from the front of `input` and advance by one event.
    fn step(
        &mut self,
        input: Input<'_>,
        sink: &mut dyn DecodeRowSink,
        stop: &dyn Stop,
    ) -> Result<Progress, StepError<Self::Error>>;

    /// What the decoder needs, valid after a step returned [`Event::Need`]
    /// (never empty then). Empty otherwise.
    fn requested(&self) -> &[Request];

    /// Image metadata, once the header has been parsed.
    fn info(&self) -> Option<&ImageInfo>;

    /// Output pixel descriptor, once the header has been parsed.
    fn output_descriptor(&self) -> Option<zenpixels::PixelDescriptor> {
        None
    }

    /// How this decoder reads its input, so a driver can choose a buffering
    /// strategy. May change once the format is known.
    fn access_pattern(&self) -> crate::source::AccessPattern {
        crate::source::AccessPattern::Sequential
    }
}

/// A codec error with its type erased: the error type of
/// [`DynIncrementalDecoder`].
///
/// `Box<dyn Error>` does not itself implement `Error`, so the erased
/// decoder needs this wrapper. It displays as the inner error and reports it
/// as its [`source`](core::error::Error::source), so cause-chain lookups
/// (such as [`CodecErrorExt`](crate::CodecErrorExt)) reach the original.
pub struct ErasedError(crate::traits::BoxedError);

impl ErasedError {
    /// Wrap a boxed error.
    pub fn new(err: crate::traits::BoxedError) -> Self {
        Self(err)
    }

    /// The boxed error.
    pub fn into_inner(self) -> crate::traits::BoxedError {
        self.0
    }

    /// The error, borrowed.
    pub fn get_ref(&self) -> &(dyn core::error::Error + Send + Sync + 'static) {
        &*self.0
    }
}

impl fmt::Debug for ErasedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

impl fmt::Display for ErasedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl core::error::Error for ErasedError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        Some(&*self.0)
    }
}

impl StepError<ErasedError> {
    /// Flatten into a boxed error: a codec error unwraps to the original,
    /// anything else is boxed as is.
    pub fn into_boxed(self) -> crate::traits::BoxedError {
        match self {
            StepError::Codec(c) => c.0,
            other => Box::new(other),
        }
    }
}

/// The object-safe form every erased incremental decoder takes.
pub type DynIncrementalDecoder = dyn IncrementalDecode<Error = ErasedError> + Send;

impl<D: IncrementalDecode + ?Sized> IncrementalDecode for Box<D> {
    type Error = D::Error;

    fn step(
        &mut self,
        input: Input<'_>,
        sink: &mut dyn DecodeRowSink,
        stop: &dyn Stop,
    ) -> Result<Progress, StepError<D::Error>> {
        (**self).step(input, sink, stop)
    }

    fn requested(&self) -> &[Request] {
        (**self).requested()
    }

    fn info(&self) -> Option<&ImageInfo> {
        (**self).info()
    }

    fn output_descriptor(&self) -> Option<zenpixels::PixelDescriptor> {
        (**self).output_descriptor()
    }

    fn access_pattern(&self) -> crate::source::AccessPattern {
        (**self).access_pattern()
    }
}

impl<D: IncrementalDecode + ?Sized> IncrementalDecode for &mut D {
    type Error = D::Error;

    fn step(
        &mut self,
        input: Input<'_>,
        sink: &mut dyn DecodeRowSink,
        stop: &dyn Stop,
    ) -> Result<Progress, StepError<D::Error>> {
        (**self).step(input, sink, stop)
    }

    fn requested(&self) -> &[Request] {
        (**self).requested()
    }

    fn info(&self) -> Option<&ImageInfo> {
        (**self).info()
    }

    fn output_descriptor(&self) -> Option<zenpixels::PixelDescriptor> {
        (**self).output_descriptor()
    }

    fn access_pattern(&self) -> crate::source::AccessPattern {
        (**self).access_pattern()
    }
}

/// Enforces the termination rules for any decoder: steps after `Done`
/// return `Done` again, steps after an error return
/// [`StepError::Finished`], and `stop` is checked before every step, a
/// tripped stop being terminal.
///
/// Wrap a hand-written decoder in `Fused` and its own state machine need
/// not track any of this.
#[derive(Debug)]
pub struct Fused<D> {
    inner: D,
    state: FusedState,
}

#[derive(Debug)]
enum FusedState {
    Running,
    Done(OutputInfo),
    Failed,
}

impl<D> Fused<D> {
    /// Wrap `inner`.
    pub const fn new(inner: D) -> Self {
        Self {
            inner,
            state: FusedState::Running,
        }
    }

    /// The wrapped decoder.
    pub fn get_ref(&self) -> &D {
        &self.inner
    }

    /// Unwrap.
    pub fn into_inner(self) -> D {
        self.inner
    }
}

impl<D: IncrementalDecode> IncrementalDecode for Fused<D> {
    type Error = D::Error;

    fn step(
        &mut self,
        input: Input<'_>,
        sink: &mut dyn DecodeRowSink,
        stop: &dyn Stop,
    ) -> Result<Progress, StepError<D::Error>> {
        match &self.state {
            FusedState::Done(info) => return Ok(Progress::new(0, Event::Done(info.clone()))),
            FusedState::Failed => return Err(StepError::Finished),
            FusedState::Running => {}
        }
        if let Err(reason) = stop.check() {
            self.state = FusedState::Failed;
            return Err(StepError::Cancelled(reason));
        }
        match self.inner.step(input, sink, stop) {
            Ok(p) => {
                if let Event::Done(info) = &p.event {
                    self.state = FusedState::Done(info.clone());
                }
                Ok(p)
            }
            Err(e) => {
                self.state = FusedState::Failed;
                Err(e)
            }
        }
    }

    fn requested(&self) -> &[Request] {
        match self.state {
            FusedState::Running => self.inner.requested(),
            _ => &[],
        }
    }

    fn info(&self) -> Option<&ImageInfo> {
        self.inner.info()
    }

    fn output_descriptor(&self) -> Option<zenpixels::PixelDescriptor> {
        self.inner.output_descriptor()
    }

    fn access_pattern(&self) -> crate::source::AccessPattern {
        self.inner.access_pattern()
    }
}

/// Shim erasing a decoder's error type to [`ErasedError`].
pub struct IncrementalDecoderShim<D>(pub D);

impl<D: IncrementalDecode> IncrementalDecode for IncrementalDecoderShim<D> {
    type Error = ErasedError;

    fn step(
        &mut self,
        input: Input<'_>,
        sink: &mut dyn DecodeRowSink,
        stop: &dyn Stop,
    ) -> Result<Progress, StepError<ErasedError>> {
        self.0
            .step(input, sink, stop)
            .map_err(|e| e.map_codec(|c| ErasedError(Box::new(c))))
    }

    fn requested(&self) -> &[Request] {
        self.0.requested()
    }

    fn info(&self) -> Option<&ImageInfo> {
        self.0.info()
    }

    fn output_descriptor(&self) -> Option<zenpixels::PixelDescriptor> {
        self.0.output_descriptor()
    }

    fn access_pattern(&self) -> crate::source::AccessPattern {
        self.0.access_pattern()
    }
}

/// Erase `decoder` for [`DecodeJob::dyn_incremental_decoder`](crate::decode::DecodeJob::dyn_incremental_decoder),
/// wrapped in [`Fused`] so the termination rules hold by construction.
pub fn erase_incremental<D>(decoder: D) -> Box<DynIncrementalDecoder>
where
    D: IncrementalDecode + Send + 'static,
{
    Box::new(IncrementalDecoderShim(Fused::new(decoder)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use zenpixels::{PixelDescriptor, PixelSliceMut};

    struct NullSink;
    impl DecodeRowSink for NullSink {
        fn provide_next_buffer(
            &mut self,
            _y: u32,
            _h: u32,
            _w: u32,
            _d: PixelDescriptor,
        ) -> Result<PixelSliceMut<'_>, SinkError> {
            Err("null sink".into())
        }
    }

    /// Fails on its second step, then would happily continue.
    struct Flaky(u32);
    impl IncrementalDecode for Flaky {
        type Error = core::fmt::Error;
        fn step(
            &mut self,
            _input: Input<'_>,
            _sink: &mut dyn DecodeRowSink,
            _stop: &dyn Stop,
        ) -> Result<Progress, StepError<core::fmt::Error>> {
            self.0 += 1;
            if self.0 == 2 {
                Err(StepError::Codec(core::fmt::Error))
            } else {
                Ok(Progress::need(0))
            }
        }
        fn requested(&self) -> &[Request] {
            &[Request::Len]
        }
        fn info(&self) -> Option<&ImageInfo> {
            None
        }
    }

    #[test]
    fn fused_makes_errors_and_cancellation_terminal() {
        let mut d = Fused::new(Flaky(0));
        let stop = enough::Unstoppable;
        assert!(d.step(Input::empty(), &mut NullSink, &stop).is_ok());
        assert!(matches!(
            d.step(Input::empty(), &mut NullSink, &stop),
            Err(StepError::Codec(_))
        ));
        assert!(matches!(
            d.step(Input::empty(), &mut NullSink, &stop),
            Err(StepError::Finished)
        ));
        assert!(d.requested().is_empty());

        struct Tripped;
        impl Stop for Tripped {
            fn check(&self) -> Result<(), StopReason> {
                Err(StopReason::Cancelled)
            }
        }
        let mut d = Fused::new(Flaky(0));
        assert!(matches!(
            d.step(Input::empty(), &mut NullSink, &Tripped),
            Err(StepError::Cancelled(StopReason::Cancelled))
        ));
        assert!(matches!(
            d.step(Input::empty(), &mut NullSink, &stop),
            Err(StepError::Finished)
        ));
    }

    #[test]
    fn requests_know_what_satisfies_them() {
        let bytes = [0u8; 10];
        let r = Request::at_least(4, 8);
        assert!(!r.is_satisfied_by(&Input::new(4, &bytes[..6])));
        assert!(r.is_satisfied_by(&Input::new(4, &bytes[..8])));
        assert!(r.is_satisfied_by(&Input::new(4, &bytes[..6]).with_total_len(10)));
        assert!(!r.is_satisfied_by(&Input::new(3, &bytes)));
        let r = Request::range(2, 3);
        assert!(r.is_satisfied_by(&Input::new(2, &bytes[..3])));
        assert!(!r.is_satisfied_by(&Input::new(2, &bytes[..2])));
        assert!(Request::Len.is_satisfied_by(&Input::empty().with_total_len(10)));
        assert!(!Request::Len.is_satisfied_by(&Input::empty()));
    }
}
