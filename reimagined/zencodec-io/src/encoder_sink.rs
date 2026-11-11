//! A sink that feeds an encoder: the transcoding seam.

use crate::memory::encoder_step;
use std::sync::Arc;
use zencodec::decode::Sink;
use zencodec::encode::{
    EncodeCodec, EncodeEvent, EncodeInput, EncodePlan, EncodeSpec, Encoder, FrameMeta, Patch,
    RowSupply,
};
use zencodec::{
    ErrorKind, FrameHeader, Pass, Pulse, Raster, Result, SharedPulse, Unsupported, copy_rows,
    error, error_msg, unsupported,
};
use zenpixels::plane::PlanarSlice;
use zenpixels::{PixelBuffer, PixelSlice, PixelSliceMut};

type SpecFn = Box<dyn FnMut(&FrameHeader) -> EncodeSpec + Send>;

/// Feeds decoded rows to an encoder.
///
/// The encoder is prepared when the first frame begins, from a spec built
/// out of that frame's header, so nothing about the image has to be known
/// before the decoder has read it. Rows the decoder already holds go to the
/// encoder in place. Rows the decoder wants to write are lent from one strip
/// buffer, sized to the encoder's strip height. No other copy is made.
///
/// The encoder's steps observe the sink's own pulse `P`: a borrowed
/// `&dyn Pulse` when the transcode stays on the caller's thread, a
/// [`SharedPulse`] when the sink travels to a worker. It may be the pulse
/// the decoder observes or a different stage of the same plan.
pub struct EncoderSink<P = SharedPulse> {
    codec: Arc<dyn EncodeCodec>,
    spec: SpecFn,
    pulse: P,
    encoder: Option<Box<dyn Encoder>>,
    raster: Option<Raster>,
    sequence: bool,
    need: EncodeEvent,
    strip: Option<PixelBuffer>,
    /// Rows the strip can hold.
    capacity: u32,
    /// Rows in the strip the encoder has not consumed.
    staged: u32,
    /// The frame row of the strip's first staged row.
    y: u32,
}

impl<P: Pulse> EncoderSink<P> {
    /// Encode with `codec`, to the spec `spec` builds from the first frame's
    /// header, observed by `pulse`. The sink sets the spec's row supply
    /// itself: a decoder shows each row once.
    pub fn new(
        codec: Arc<dyn EncodeCodec>,
        spec: impl FnMut(&FrameHeader) -> EncodeSpec + Send + 'static,
        pulse: P,
    ) -> Self {
        Self {
            codec,
            spec: Box::new(spec),
            pulse,
            encoder: None,
            raster: None,
            sequence: false,
            need: EncodeEvent::Done,
            strip: None,
            capacity: 0,
            staged: 0,
            y: 0,
        }
    }

    /// The encoder's plan, once the first frame has begun.
    pub fn plan(&self) -> Option<&EncodePlan> {
        self.encoder.as_deref().map(Encoder::plan)
    }

    /// Take the bytes encoded so far.
    pub fn take_output(&mut self) -> Vec<u8> {
        self.encoder
            .as_mut()
            .map_or_else(Vec::new, |e| e.output().take())
    }

    /// Return a vector from [`take_output`](Self::take_output) for reuse.
    pub fn recycle(&mut self, spare: Vec<u8>) {
        if let Some(encoder) = self.encoder.as_mut() {
            encoder.output().recycle(spare);
        }
    }

    /// Take the patches to bytes already taken.
    pub fn take_patches(&mut self) -> Vec<Patch> {
        self.encoder
            .as_mut()
            .map_or_else(Vec::new, |e| e.output().take_patches())
    }

    /// Close the encode once the decoder is done. For a sequence this ends
    /// it; a still is already complete.
    pub fn finish(&mut self) -> Result<()> {
        let Some(encoder) = self.encoder.as_mut() else {
            return Err(error_msg(
                ErrorKind::Protocol,
                "no frame reached the encoder",
            ));
        };
        if self.need == EncodeEvent::NeedFrame {
            self.need = encoder_step(&mut **encoder, EncodeInput::End, &self.pulse)?.event;
        }
        if self.need == EncodeEvent::Done {
            Ok(())
        } else {
            Err(error_msg(
                ErrorKind::Protocol,
                "the encoder still wants rows",
            ))
        }
    }

    /// One step of the encoder, recording what it needs next.
    fn step(&mut self, input: EncodeInput<'_>) -> Result<u32> {
        let encoder = self
            .encoder
            .as_deref_mut()
            .ok_or_else(|| error_msg(ErrorKind::Protocol, "rows outside a frame"))?;
        let step = encoder_step(encoder, input, &self.pulse)?;
        self.need = step.event;
        Ok(step.rows)
    }

    /// Present the staged rows. Mid-frame the encoder may leave a remainder;
    /// at the frame's end it must take them all.
    fn flush(&mut self, end: bool) -> Result<()> {
        while self.staged > 0 {
            let strip = self.strip.as_ref().expect("rows are staged in it");
            let rows = strip.rows(0, self.staged);
            let encoder = self.encoder.as_mut().expect("prepared at begin");
            let step = encoder_step(&mut **encoder, EncodeInput::Rows(rows), &self.pulse)?;
            self.need = step.event;
            let used = step.rows.min(self.staged);
            if used == 0 {
                if end {
                    return Err(error_msg(
                        ErrorKind::Protocol,
                        "encoder left rows at the frame's end",
                    ));
                }
                break;
            }
            let left = self.staged - used;
            if left > 0 {
                // Slide the remainder to the front of the strip.
                let strip = self.strip.as_mut().expect("rows are staged in it");
                let row_bytes = strip.width() as usize * strip.descriptor().bytes_per_pixel();
                let mut view = strip.as_slice_mut();
                let stride = view.stride();
                let bytes = view.as_strided_bytes_mut();
                for r in 0..left as usize {
                    let from = (used as usize + r) * stride;
                    bytes.copy_within(from..from + row_bytes, r * stride);
                }
            }
            self.staged = left;
            self.y += used;
            if !end {
                break;
            }
        }
        Ok(())
    }

    /// Copy rows into the strip behind what is already staged.
    fn stage(&mut self, rows: &PixelSlice<'_>) -> Result<()> {
        let staged = self.staged;
        let strip = self.strip_mut()?;
        copy_rows(rows, &mut strip.rows_mut(staged, rows.rows()))?;
        self.staged += rows.rows();
        Ok(())
    }

    fn strip_mut(&mut self) -> Result<&mut PixelBuffer> {
        if self.strip.is_none() {
            let raster = self.raster.as_ref().expect("set at begin");
            let descriptor = raster
                .descriptor()
                .ok_or_else(|| error_msg(ErrorKind::Protocol, "packed rows for a planar frame"))?;
            let buffer = PixelBuffer::try_new(raster.width, self.capacity, descriptor)
                .map_err(|_| error(ErrorKind::OutOfMemory))?;
            self.strip = Some(buffer);
        }
        Ok(self.strip.as_mut().expect("allocated above"))
    }

    fn in_order(&self, y: u32, pass: Pass) -> Result<()> {
        if pass != Pass::Final {
            return Err(unsupported(Unsupported::Progressive));
        }
        if !matches!(self.need, EncodeEvent::NeedRows { .. }) {
            return Err(error_msg(
                ErrorKind::Protocol,
                "the encoder is not taking rows",
            ));
        }
        if y != self.y + self.staged {
            return Err(error_msg(ErrorKind::Protocol, "rows out of order"));
        }
        Ok(())
    }
}

impl<P: Pulse> Sink for EncoderSink<P> {
    fn begin(&mut self, frame: &FrameHeader) -> Result<()> {
        if self.encoder.is_none() {
            let mut spec = (self.spec)(frame);
            spec.supply = RowSupply::Once;
            self.sequence = spec.sequence.is_some();
            let mut encoder = self.codec.encoder(&spec)?;
            self.need = encoder_step(&mut *encoder, EncodeInput::None, &self.pulse)?.event;
            // A strip of at least 16 rows, in whole encoder strips, and no
            // taller than the frame.
            let unit = encoder.plan().strip_rows.max(1);
            self.capacity = (unit * 16u32.div_ceil(unit)).min(spec.raster.height.max(1));
            self.raster = Some(spec.raster);
            self.encoder = Some(encoder);
        }
        let raster = self.raster.as_ref().expect("set above");
        if frame.raster.width != raster.width
            || frame.raster.height != raster.height
            || frame.raster.layout != raster.layout
        {
            return Err(error_msg(
                ErrorKind::Protocol,
                "frame does not match the raster being encoded",
            ));
        }
        if self.sequence {
            if self.need != EncodeEvent::NeedFrame {
                return Err(error_msg(ErrorKind::Protocol, "the encoder is mid-frame"));
            }
            let meta = frame
                .duration
                .map_or_else(FrameMeta::default, FrameMeta::shown_for);
            self.step(EncodeInput::Frame(meta))?;
        } else if frame.index > 0 {
            return Err(unsupported(Unsupported::Animation));
        }
        if self.need != (EncodeEvent::NeedRows { pass: 0, y: 0 }) {
            return Err(error_msg(
                ErrorKind::Protocol,
                "the encoder is not at a frame's start",
            ));
        }
        self.staged = 0;
        self.y = 0;
        Ok(())
    }

    fn lend(&mut self, y: u32, rows: u32, pass: Pass) -> Result<PixelSliceMut<'_>> {
        self.in_order(y, pass)?;
        if self.staged == self.capacity {
            self.flush(false)?;
            if self.staged == self.capacity {
                return Err(error_msg(
                    ErrorKind::Protocol,
                    "encoder consumed nothing from a full strip",
                ));
            }
        }
        let n = rows.min(self.capacity - self.staged);
        if n == 0 {
            return Err(error_msg(ErrorKind::Protocol, "no rows asked for"));
        }
        let at = self.staged;
        // Lent rows are the decoder's to fill before it calls again.
        self.staged += n;
        Ok(self.strip_mut()?.rows_mut(at, n))
    }

    fn rows(&mut self, y: u32, rows: PixelSlice<'_>, pass: Pass) -> Result<()> {
        self.in_order(y, pass)?;
        let mut offset = 0;
        while offset < rows.rows() {
            let left = rows.rows() - offset;
            if self.staged == 0 {
                // Nothing waiting: show the encoder the decoder's rows in place.
                let used = self
                    .step(EncodeInput::Rows(rows.sub_rows(offset, left)))?
                    .min(left);
                offset += used;
                self.y += used;
                if used == 0 {
                    // Less than one encoder strip: hold it for the next rows.
                    if left > self.capacity {
                        return Err(error_msg(
                            ErrorKind::Protocol,
                            "encoder consumed nothing from whole strips",
                        ));
                    }
                    self.stage(&rows.sub_rows(offset, left))?;
                    offset += left;
                }
            } else {
                // Top up the waiting rows, then present them.
                let n = left.min(self.capacity - self.staged);
                self.stage(&rows.sub_rows(offset, n))?;
                offset += n;
                if self.staged == self.capacity {
                    self.flush(false)?;
                    if self.staged == self.capacity {
                        return Err(error_msg(
                            ErrorKind::Protocol,
                            "encoder consumed nothing from a full strip",
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    fn planes(&mut self, y: u32, strip: PlanarSlice<'_>) -> Result<()> {
        self.in_order(y, Pass::Final)?;
        let height = strip.height();
        if self.step(EncodeInput::Planes(strip))? != height {
            // Holding a planar remainder needs a mutable planar strip, which
            // zenpixels does not have yet. See the workspace README.
            return Err(error_msg(
                ErrorKind::Protocol,
                "planar strips must match the encoder's strip height",
            ));
        }
        self.y += height;
        Ok(())
    }

    fn end(&mut self) -> Result<()> {
        self.flush(true)?;
        let complete = if self.sequence {
            self.need == EncodeEvent::NeedFrame
        } else {
            self.need == EncodeEvent::Done
        };
        if complete {
            Ok(())
        } else {
            Err(error_msg(
                ErrorKind::Protocol,
                "frame ended with rows missing",
            ))
        }
    }
}
