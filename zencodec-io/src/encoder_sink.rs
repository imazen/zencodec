//! Transcode without an intermediate image: a sink that hands the decoder
//! its own strip buffer and pushes each finished strip into an encoder.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::fmt;

use zencodec::decode::{
    BatchMeta, DecodeRowSink, RowMetadata, RowStrip, SinkError, StripDimensions,
};
use zencodec::encode::{EncodeOutput, Encoder, OutputTiming};
use zenpixels::{
    ByteStorage, ByteStorageMut, PixelBuf, PixelFormat, PixelStorage, PixelView, PixelViewMut,
};

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
    strip: Option<PixelStorage>,
    pending: Option<(u32, RowMetadata)>,
    declaration: Option<(u32, u32, PixelFormat)>,
    row_limit: core::num::NonZeroU32,
    finished: bool,
    failed: bool,
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
            strip: None,
            pending: None,
            declaration: None,
            row_limit: core::num::NonZeroU32::new(64).unwrap(),
            finished: false,
            failed: false,
            rows_pushed: 0,
            timing: None,
        }
    }

    /// Set the largest strip that the producer may request.
    pub fn with_max_buffer_rows(mut self, limit: core::num::NonZeroU32) -> Self {
        self.row_limit = limit;
        self
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
        let Some((rows, metadata)) = self.pending.take() else {
            return Ok(());
        };
        let (width, _, desc) = self
            .declaration
            .as_ref()
            .expect("pending strip was declared");
        let stride = *width as usize * desc.layout.bytes_per_pixel();
        let slice = PixelView::new(
            self.strip
                .as_ref()
                .expect("pending strip has storage")
                .as_bytes(),
            *width,
            rows,
            stride,
            desc,
        )?;
        let enc = self
            .encoder
            .as_mut()
            .ok_or_else(|| SinkError::from("EncoderSink: encoder already finished"))?;
        if let Err(error) = enc.push_rows(metadata.attach(slice)) {
            self.failed = true;
            return Err(Box::new(error));
        }
        self.rows_pushed += rows as u64;
        Ok(())
    }

    #[cfg(feature = "tokio")]
    pub(crate) fn validate_completion(
        &self,
        info: &zencodec::decode::OutputInfo,
    ) -> Result<(), SinkError> {
        if self.finished
            && !self.failed
            && self
                .declaration
                .as_ref()
                .is_some_and(|(width, height, spec)| {
                    (*width, *height, spec) == (info.width, info.height, &info.native_format)
                })
        {
            Ok(())
        } else {
            Err("EncoderSink: decoder completion disagrees with finished rows".into())
        }
    }

    /// Push any pending strip and finish the encoder. Returns the bytes not
    /// already taken through [`consume_output`](Self::consume_output).
    pub fn finish(mut self) -> Result<EncodeOutput, EncoderSinkError<E::Error>> {
        if !self.finished {
            DecodeRowSink::finish(&mut self).map_err(EncoderSinkError::Sink)?;
        }
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
    fn begin(&mut self, width: u32, height: u32, descriptor: PixelFormat) -> Result<(), SinkError> {
        if self.declaration.is_some() || self.failed {
            return Err("EncoderSink: begin called twice or after failure".into());
        }
        descriptor.validate()?;
        let enc = self.encoder.as_mut().expect("encoder present until finish");
        let timing = enc
            .begin(width, height, descriptor.clone())
            .map_err(|e| Box::new(e) as SinkError)?;
        self.declaration = Some((width, height, descriptor));
        self.timing = Some(timing);
        Ok(())
    }

    fn max_buffer_rows(&self) -> core::num::NonZeroU32 {
        self.row_limit
    }

    fn provide_next_buffer(
        &mut self,
        strip: RowStrip,
        descriptor: PixelFormat,
    ) -> Result<PixelViewMut<'_>, SinkError> {
        self.provide_buffer_with_metadata(
            BatchMeta::final_rows(strip.y),
            StripDimensions::new(strip.width, strip.height),
            descriptor,
            &RowMetadata::default(),
        )
    }

    fn provide_buffer_with_metadata(
        &mut self,
        meta: BatchMeta,
        size: StripDimensions,
        descriptor: PixelFormat,
        metadata: &RowMetadata,
    ) -> Result<PixelViewMut<'_>, SinkError> {
        let StripDimensions { width, height } = size;
        let Some((expected_width, total_rows, spec)) = &self.declaration else {
            return Err("EncoderSink: rows before begin".into());
        };
        if self.finished || self.failed {
            return Err("EncoderSink: rows after termination".into());
        }
        let next = self.rows_pushed + self.pending.as_ref().map_or(0, |(n, _)| u64::from(*n));
        if !meta.is_final
            || u64::from(meta.y) != next
            || width != *expected_width
            || descriptor != *spec
            || height == 0
            || height > self.row_limit.get()
            || next + u64::from(height) > u64::from(*total_rows)
        {
            return Err(
                "EncoderSink: strip disagrees with the declared raster order or spec".into(),
            );
        }
        self.push_pending()?;
        let stride = (width as usize)
            .checked_mul(descriptor.layout.bytes_per_pixel())
            .ok_or("EncoderSink: row too large")?;
        let len = stride
            .checked_mul(height as usize)
            .ok_or("EncoderSink: strip too large")?;
        let alignment = descriptor.layout.min_alignment();
        if self.strip.as_ref().is_none_or(|storage| {
            storage.byte_len() < len || storage.allocation_alignment() < alignment
        }) {
            self.strip = Some(
                PixelBuf::new(width, height, descriptor.clone())?
                    .into_parts()
                    .storage,
            );
        }
        self.pending = Some((height, metadata.clone()));
        let (_, _, declared_spec) = self
            .declaration
            .as_ref()
            .expect("declaration verified above");
        let storage = self.strip.as_mut().expect("strip storage installed");
        let view = PixelViewMut::new(
            &mut storage.as_bytes_mut()[..len],
            width,
            height,
            stride,
            declared_spec,
        )?;
        Ok(view.with_hdr(metadata.hdr))
    }

    fn finish(&mut self) -> Result<(), SinkError> {
        let Some((_, height, _)) = &self.declaration else {
            return Err("EncoderSink: finish before begin".into());
        };
        if self.finished || self.failed {
            return Err("EncoderSink: finish after termination".into());
        }
        let supplied = self.rows_pushed + self.pending.as_ref().map_or(0, |(n, _)| u64::from(*n));
        if supplied != u64::from(*height) {
            return Err("EncoderSink: finish before complete row coverage".into());
        }
        self.push_pending()?;
        self.finished = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zencodec::{ImageFormat, UnsupportedOperation};
    use zenpixels::Primaries;

    struct Rows {
        bytes: Vec<u8>,
    }
    impl Encoder for Rows {
        type Error = UnsupportedOperation;
        fn reject(error: UnsupportedOperation) -> Self::Error {
            error
        }
        fn begin(
            &mut self,
            _width: u32,
            _height: u32,
            _spec: PixelFormat,
        ) -> Result<OutputTiming, Self::Error> {
            self.bytes.push(0xCC);
            Ok(OutputTiming::streaming(0))
        }
        fn encode(self, _pixels: PixelView<'_>) -> Result<EncodeOutput, Self::Error> {
            unreachable!()
        }
        fn push_rows(&mut self, rows: PixelView<'_>) -> Result<(), Self::Error> {
            if rows.row(0)[0] == 0xFF {
                return Err(UnsupportedOperation::PixelFormat);
            }
            self.bytes.extend_from_slice(rows.row(0));
            Ok(())
        }
        fn pending_output(&self) -> &[u8] {
            &self.bytes
        }
        fn finish(self) -> Result<EncodeOutput, Self::Error> {
            Ok(EncodeOutput::new(self.bytes, ImageFormat::Png))
        }
    }

    #[test]
    fn checked_strip_order_mismatch_and_terminal_calls_preserve_accepted_bytes() {
        let spec = PixelFormat::RGB8_SRGB;
        let alternate = spec.clone().with_primaries(Primaries::DisplayP3);
        let mut sink = EncoderSink::new(Rows { bytes: Vec::new() })
            .with_max_buffer_rows(core::num::NonZeroU32::MIN);
        assert!(
            sink.provide_next_buffer(RowStrip::new(0, 1, 1), spec.clone())
                .is_err()
        );
        DecodeRowSink::begin(&mut sink, 1, 2, spec.clone()).unwrap();
        assert!(DecodeRowSink::begin(&mut sink, 1, 2, spec.clone()).is_err());
        assert!(
            sink.provide_next_buffer(RowStrip::new(1, 1, 1), spec.clone())
                .is_err()
        );
        assert!(
            sink.provide_next_buffer(RowStrip::new(0, 1, 2), spec.clone())
                .is_err()
        );
        sink.provide_next_buffer(RowStrip::new(0, 1, 1), spec.clone())
            .unwrap()
            .row_mut(0)
            .copy_from_slice(&[1, 2, 3]);
        assert!(
            sink.provide_next_buffer(RowStrip::new(0, 1, 1), spec.clone())
                .is_err()
        );
        assert!(
            sink.provide_next_buffer(RowStrip::new(1, 1, 1), alternate)
                .is_err()
        );
        assert!(DecodeRowSink::finish(&mut sink).is_err());
        assert_eq!(sink.pending_output(), &[0xCC]);
        sink.provide_next_buffer(RowStrip::new(1, 1, 1), spec.clone())
            .unwrap()
            .row_mut(0)
            .copy_from_slice(&[4, 5, 6]);
        assert_eq!(sink.rows_pushed(), 1);
        DecodeRowSink::finish(&mut sink).unwrap();
        assert!(DecodeRowSink::finish(&mut sink).is_err());
        assert!(
            sink.provide_next_buffer(RowStrip::new(2, 1, 1), spec)
                .is_err()
        );
        assert_eq!(sink.finish().unwrap().data(), &[0xCC, 1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn native_row_refusal_remains_primary_and_keeps_confirmed_prefix() {
        let spec = PixelFormat::RGB8_SRGB;
        let mut sink = EncoderSink::new(Rows { bytes: Vec::new() });
        sink.begin(1, 2, spec.clone()).unwrap();
        sink.provide_next_buffer(RowStrip::new(0, 1, 1), spec.clone())
            .unwrap()
            .row_mut(0)
            .copy_from_slice(&[1, 2, 3]);
        sink.provide_next_buffer(RowStrip::new(1, 1, 1), spec)
            .unwrap()
            .row_mut(0)
            .fill(0xFF);
        let error = DecodeRowSink::finish(&mut sink).unwrap_err();
        assert_eq!(
            zencodec::find_cause::<UnsupportedOperation>(error.as_ref()),
            Some(&UnsupportedOperation::PixelFormat)
        );
        assert_eq!(sink.pending_output(), &[0xCC, 1, 2, 3]);
        assert_eq!(sink.rows_pushed(), 1);
    }
}
