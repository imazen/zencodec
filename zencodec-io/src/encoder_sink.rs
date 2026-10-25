//! Transcode without an intermediate image: a sink that hands the decoder
//! its own strip buffer and pushes each finished strip into an encoder.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::fmt;

use zencodec::decode::{DecodeRowSink, SinkError};
use zencodec::encode::{EncodeOutput, Encoder, OutputTiming};
use zenpixels::{PixelDescriptor, PixelSlice, PixelSliceMut};

/// Failure of [`EncoderSink::finish`].
#[derive(Debug)]
#[non_exhaustive]
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

/// [`DecodeRowSink`] that feeds an [`Encoder`].
///
/// `begin` declares the image to the encoder ([`Encoder::begin`]); an
/// encoder that answers with a streaming [`OutputTiming`] writes its header
/// at once and makes bytes available as rows arrive.
/// [`output_timing`](Self::output_timing) reports that answer.
/// `provide_next_buffer` returns the sink's own strip buffer, so the
/// decoder writes straight into the bytes the encoder reads; the strip is
/// pushed on the next `provide_next_buffer` or on `finish`. No pixel is
/// copied by this handoff; the encoder may copy or retain its own input/state.
///
/// Drain encoded bytes between decode steps with
/// [`pending_output`](Self::pending_output) and
/// [`consume_output`](Self::consume_output) to keep output memory bounded.
/// That bounds queued encoded bytes only. Awaiting a slow writer can keep
/// much larger raw/intermediate allocations alive; output timing alone does
/// not establish when those allocations are released.
/// Encoder errors reach the decoder's caller as the sink error, typed, so
/// they can be told apart from malformed input.
pub struct EncoderSink<E> {
    encoder: Option<E>,
    strip: Vec<u8>,
    pending: Option<(u32, u32, PixelDescriptor)>,
    rows_pushed: u64,
    timing: Option<OutputTiming>,
}

impl<E: Encoder> EncoderSink<E>
where
    E::Error: Send + Sync,
{
    /// Wrap `encoder`; rows go to its `push_rows`.
    pub fn new(encoder: E) -> Self {
        Self {
            encoder: Some(encoder),
            strip: Vec::new(),
            pending: None,
            rows_pushed: 0,
            timing: None,
        }
    }

    /// What the encoder's [`begin`](Encoder::begin) reported: whether its
    /// bytes stream and how many rows it holds back. `None` until the
    /// decoder has declared the image.
    ///
    /// The sink pushes each strip when the decoder asks for the next one,
    /// so one strip more than the encoder's
    /// [`lag_rows`](OutputTiming::lag_rows) can wait here.
    pub fn output_timing(&self) -> Option<OutputTiming> {
        self.timing
    }

    /// Rows pushed into the encoder so far.
    pub fn rows_pushed(&self) -> u64 {
        self.rows_pushed
    }

    /// The encoder.
    pub fn encoder(&self) -> &E {
        self.encoder.as_ref().expect("present until finish")
    }

    /// Encoded bytes ready to send; see [`Encoder::pending_output`].
    pub fn pending_output(&self) -> &[u8] {
        self.encoder().pending_output()
    }

    /// Mark the first `n` pending bytes as sent.
    pub fn consume_output(&mut self, n: usize) {
        if let Some(enc) = self.encoder.as_mut() {
            enc.consume_output(n);
        }
    }

    /// Append every pending byte to `into` and consume it; see
    /// [`Encoder::take_output`].
    pub fn take_output(&mut self, into: &mut Vec<u8>) {
        if let Some(enc) = self.encoder.as_mut() {
            enc.take_output(into);
        }
    }

    /// Write every pending byte to `w`. Returns how many were written.
    #[cfg(feature = "std")]
    pub fn write_pending(&mut self, w: &mut impl std::io::Write) -> std::io::Result<usize> {
        let n = self.pending_output().len();
        if n > 0 {
            w.write_all(self.pending_output())?;
            self.consume_output(n);
        }
        Ok(n)
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
        enc.push_rows(slice).map_err(|e| Box::new(e) as SinkError)?;
        self.rows_pushed += rows as u64;
        Ok(())
    }

    /// Push any pending strip and finish the encoder. Returns the bytes not
    /// already taken through [`consume_output`](Self::consume_output).
    pub fn finish(mut self) -> Result<EncodeOutput, EncoderSinkError<E::Error>> {
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

impl<E: Encoder> DecodeRowSink for EncoderSink<E>
where
    E::Error: Send + Sync,
{
    fn begin(
        &mut self,
        width: u32,
        height: u32,
        descriptor: PixelDescriptor,
    ) -> Result<(), SinkError> {
        let enc = self
            .encoder
            .as_mut()
            .ok_or_else(|| SinkError::from("EncoderSink: encoder already finished"))?;
        let timing = enc
            .begin(width, height, descriptor)
            .map_err(|e| Box::new(e) as SinkError)?;
        self.timing = Some(timing);
        Ok(())
    }

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
