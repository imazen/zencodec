//! One bound for every driver: a concrete [`IncrementalDecode`] or an
//! erased [`DynIncrementalDecoder`] behind [`Dyn`].

use alloc::boxed::Box;
use core::fmt;
use core::ops::DerefMut;

use enough::Stop;
use zencodec::ImageInfo;
use zencodec::decode::{BoxedError, DecodeRowSink, DynIncrementalDecoder, IncrementalDecode, Step};
use zencodec::source::AccessPattern;

/// What a driver needs from a decoder. Implemented for every
/// [`IncrementalDecode`] and, through [`Dyn`], for erased decoders.
pub trait Drivable {
    /// The decoder's error type.
    type Error: core::error::Error + Send + Sync + 'static;
    /// See [`IncrementalDecode::feed`].
    fn feed(&mut self, offset: u64, bytes: &[u8], eof: bool) -> Result<usize, Self::Error>;
    /// See [`IncrementalDecode::step`].
    fn step(
        &mut self,
        sink: &mut dyn DecodeRowSink,
        stop: Option<&dyn Stop>,
    ) -> Result<Step<'_>, Self::Error>;
    /// See [`IncrementalDecode::info`].
    fn info(&self) -> Option<&ImageInfo>;
    /// See [`IncrementalDecode::access_pattern`].
    fn access_pattern(&self) -> AccessPattern;
}

impl<D: IncrementalDecode + ?Sized> Drivable for D {
    type Error = D::Error;
    fn feed(&mut self, offset: u64, bytes: &[u8], eof: bool) -> Result<usize, D::Error> {
        IncrementalDecode::feed(self, offset, bytes, eof)
    }
    fn step(
        &mut self,
        sink: &mut dyn DecodeRowSink,
        stop: Option<&dyn Stop>,
    ) -> Result<Step<'_>, D::Error> {
        IncrementalDecode::step(self, sink, stop)
    }
    fn info(&self) -> Option<&ImageInfo> {
        IncrementalDecode::info(self)
    }
    fn access_pattern(&self) -> AccessPattern {
        IncrementalDecode::access_pattern(self)
    }
}

/// An erased decoder's boxed error, wrapped so it implements `Error`.
#[derive(Debug)]
pub struct DynCodecError(pub BoxedError);

impl fmt::Display for DynCodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl core::error::Error for DynCodecError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        Some(self.0.as_ref())
    }
}

/// Adapter making an erased decoder [`Drivable`]: wrap a
/// `Box<dyn DynIncrementalDecoder>` or a `&mut dyn DynIncrementalDecoder`.
pub struct Dyn<B>(pub B);

impl<B: DerefMut<Target = dyn DynIncrementalDecoder>> Drivable for Dyn<B> {
    type Error = DynCodecError;
    fn feed(&mut self, offset: u64, bytes: &[u8], eof: bool) -> Result<usize, DynCodecError> {
        self.0.feed(offset, bytes, eof).map_err(DynCodecError)
    }
    fn step(
        &mut self,
        sink: &mut dyn DecodeRowSink,
        stop: Option<&dyn Stop>,
    ) -> Result<Step<'_>, DynCodecError> {
        self.0.step(sink, stop).map_err(DynCodecError)
    }
    fn info(&self) -> Option<&ImageInfo> {
        self.0.info()
    }
    fn access_pattern(&self) -> AccessPattern {
        self.0.access_pattern()
    }
}

impl Dyn<Box<dyn DynIncrementalDecoder>> {
    /// Take the erased decoder back out.
    pub fn into_inner(self) -> Box<dyn DynIncrementalDecoder> {
        self.0
    }
}
