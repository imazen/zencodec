//! Transcode without an intermediate image: a sink that hands the decoder
//! its own strip buffer and pushes each finished strip into an encoder.

use alloc::vec::Vec;
use core::fmt;

use zencodec::decode::{DecodeRowSink, SinkError};
use zencodec::encode::{EncodeOutput, Encoder};
use zenpixels::{PixelDescriptor, PixelSlice, PixelSliceMut};

/// Failure of [`EncoderSink::finish`].
#[derive(Debug)]
pub enum EncoderSinkError<E> {
    /// The sink failed before the encoder could finish.
    Sink(SinkError),
    /// The encoder failed.
    Encoder(E),
}

impl<E: fmt::Display> fmt::Display for EncoderSinkError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sink(e) => write!(f, "encoder sink: {e}"),
            Self::Encoder(e) => write!(f, "encoder: {e}"),
        }
    }
}

impl<E: core::error::Error + 'static> core::error::Error for EncoderSinkError<E> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Sink(e) => Some(e.as_ref()),
            Self::Encoder(e) => Some(e),
        }
    }
}

/// [`DecodeRowSink`] that feeds an [`Encoder`]'s `push_rows`.
///
/// `provide_next_buffer` returns the sink's own strip buffer, so the
/// decoder writes straight into the bytes the encoder will read; the
/// strip is pushed on the next `provide_*` call or on `finish`. No pixel
/// is copied between decoder and encoder.
pub struct EncoderSink<E> {
    encoder: Option<E>,
    strip: Vec<u8>,
    pending: Option<(u32, u32, PixelDescriptor)>,
    rows_pushed: u64,
    error: Option<SinkError>,
}

impl<E: Encoder> EncoderSink<E> {
    /// Wrap `encoder`; rows go to its `push_rows`.
    pub fn new(encoder: E) -> Self {
        Self {
            encoder: Some(encoder),
            strip: Vec::new(),
            pending: None,
            rows_pushed: 0,
            error: None,
        }
    }

    /// Rows pushed into the encoder so far.
    pub fn rows_pushed(&self) -> u64 {
        self.rows_pushed
    }

    fn push_pending(&mut self) -> Result<(), SinkError> {
        let Some((rows, width, desc)) = self.pending.take() else {
            return Ok(());
        };
        let stride = width as usize * desc.bytes_per_pixel();
        let slice = PixelSlice::new(
            &self.strip[..stride * rows as usize],
            width,
            rows,
            stride,
            desc,
        )
        .map_err(|e| SinkError::from(alloc::format!("EncoderSink: {e}")))?;
        let enc = self
            .encoder
            .as_mut()
            .ok_or_else(|| SinkError::from("EncoderSink: encoder already finished"))?;
        enc.push_rows(slice)
            .map_err(|e| SinkError::from(alloc::format!("encoder: {e}")))?;
        self.rows_pushed += rows as u64;
        Ok(())
    }

    /// Push any pending strip and finish the encoder.
    ///
    /// Call after the decode returned `Done` (the decoder called
    /// `finish` on this sink, which pushed the last strip).
    pub fn finish(mut self) -> Result<EncodeOutput, EncoderSinkError<E::Error>> {
        if let Some(e) = self.error.take() {
            return Err(EncoderSinkError::Sink(e));
        }
        self.push_pending().map_err(EncoderSinkError::Sink)?;
        let enc = self.encoder.take().expect("encoder present until finish");
        enc.finish().map_err(EncoderSinkError::Encoder)
    }
}

impl<E: Encoder> fmt::Debug for EncoderSink<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EncoderSink")
            .field("rows_pushed", &self.rows_pushed)
            .field("pending", &self.pending.is_some())
            .finish_non_exhaustive()
    }
}

impl<E: Encoder> DecodeRowSink for EncoderSink<E> {
    fn provide_next_buffer(
        &mut self,
        _y: u32,
        height: u32,
        width: u32,
        descriptor: PixelDescriptor,
    ) -> Result<PixelSliceMut<'_>, SinkError> {
        self.push_pending()?;
        let stride = width as usize * descriptor.bytes_per_pixel();
        let len = stride
            .checked_mul(height as usize)
            .ok_or_else(|| SinkError::from("EncoderSink: strip too large"))?;
        if self.strip.len() < len {
            self.strip.resize(len, 0);
        }
        self.pending = Some((height, width, descriptor));
        PixelSliceMut::new(&mut self.strip[..len], width, height, stride, descriptor)
            .map_err(|e| alloc::format!("EncoderSink: {e}").into())
    }

    fn finish(&mut self) -> Result<(), SinkError> {
        self.push_pending()
    }
}
