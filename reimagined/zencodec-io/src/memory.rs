//! Drivers for bytes and pixels that are already in memory.

use crate::collect::{Collect, Frame, Pixels};
use crate::encoder_sink::EncoderSink;
use std::sync::Arc;
use zencodec::decode::{DecodeCodec, DecodeEvent, DecodeSpec, Decoder, Sink};
use zencodec::encode::{
    EncodeCodec, EncodeEvent, EncodeInput, EncodeSpec, EncodeStep, Encoder, FrameMeta, RowSupply,
};
use zencodec::{
    ErrorKind, FrameHeader, ImageInfo, Input, InputRequest, NoPulse, Pass, Pulse, Result, check,
    error, error_msg,
};
use zenpixels::{PixelBuffer, PixelSlice, PixelSliceMut};

/// A decoded image.
#[derive(Debug)]
pub struct Decoded {
    /// What the file said about itself, complete.
    pub info: ImageInfo,
    /// The frames asked for.
    pub frames: Vec<Frame>,
}

impl Decoded {
    /// The image as one packed buffer: the first frame, with the color its
    /// header declared. `None` when nothing was decoded or the frame is planar.
    pub fn into_still(self) -> Option<(ImageInfo, PixelBuffer)> {
        let frame = self.frames.into_iter().next()?;
        let Pixels::Packed(pixels) = frame.pixels else {
            return None;
        };
        let pixels = match (pixels.color_context(), frame.header.raster.color) {
            (None, Some(color)) => pixels.with_color_context(color),
            _ => pixels,
        };
        Some((self.info, pixels))
    }
}

/// Decode a complete input into owned frames.
pub fn decode(
    codec: &dyn DecodeCodec,
    bytes: &[u8],
    spec: &DecodeSpec,
    pulse: &dyn Pulse,
) -> Result<Decoded> {
    let mut collect = Collect::new();
    let decoder = decode_slice(codec.decoder(spec)?, bytes, &mut collect, pulse)?;
    let info = decoder
        .info()
        .cloned()
        .ok_or_else(|| error_msg(ErrorKind::Protocol, "decode finished without info"))?;
    Ok(Decoded {
        info,
        frames: collect.into_frames(),
    })
}

/// A sink for a header read: nothing may reach it.
struct NoRows;

impl Sink for NoRows {
    fn begin(&mut self, _: &FrameHeader) -> Result<()> {
        Err(error_msg(ErrorKind::Protocol, "a frame began before Info"))
    }
    fn lend(&mut self, _: u32, _: u32, _: Pass) -> Result<PixelSliceMut<'_>> {
        Err(error_msg(ErrorKind::Protocol, "rows arrived before Info"))
    }
}

/// Read what a file says about itself from its first bytes, decoding no
/// pixels.
///
/// `prefix` is the start of the file, of any length. `Ok(None)` means the
/// header, or metadata the spec asked to have settled first, reaches past it.
pub fn probe(
    codec: &dyn DecodeCodec,
    prefix: &[u8],
    spec: &DecodeSpec,
) -> Result<Option<ImageInfo>> {
    let mut decoder = codec.decoder(spec)?;
    let len = prefix.len() as u64;
    let mut stalled = None;
    loop {
        let input = match decoder.requested().first() {
            Some(InputRequest::AtLeast { offset, .. }) if *offset <= len => {
                Input::new(*offset, &prefix[*offset as usize..])
            }
            Some(InputRequest::Range(range)) if range.end() <= len => Input::new(
                range.offset,
                &prefix[range.offset as usize..range.end() as usize],
            ),
            // Bytes past the prefix, the whole file, or its length.
            _ => return Ok(None),
        };
        let lent = (input.offset, input.bytes.len());
        if stalled == Some(lent) {
            return Ok(None);
        }
        let step = decoder.step(input, &mut NoRows, &NoPulse)?;
        match step.event {
            DecodeEvent::Info => return Ok(decoder.info().cloned()),
            DecodeEvent::NeedInput => stalled = (step.consumed == 0).then_some(lent),
            _ => {
                return Err(error_msg(
                    ErrorKind::Protocol,
                    "decoder went past the header without Info",
                ));
            }
        }
    }
}

/// A decode over bytes that are all in memory, resumable between steps.
///
/// Every request is answered with a sub-slice of the caller's bytes: nothing
/// is copied or buffered on the way in.
pub(crate) struct SliceRun {
    /// The window last lent to a step that took nothing and asked for more.
    stalled: Option<(u64, usize)>,
}

impl SliceRun {
    pub(crate) fn new() -> Self {
        Self { stalled: None }
    }

    /// Up to `quota` steps, and at least one. `Ok(true)` when the decode is
    /// complete.
    ///
    /// The pulse is checked before the first step and after every step, so a
    /// step that succeeds cannot hide a stop raised while it ran, the last
    /// one included. A decoder's own error comes first.
    pub(crate) fn run(
        &mut self,
        decoder: &mut dyn Decoder,
        bytes: &[u8],
        sink: &mut dyn Sink,
        quota: usize,
        pulse: &dyn Pulse,
    ) -> Result<bool> {
        let len = bytes.len() as u64;
        let at = |offset: u64| offset.min(len) as usize;
        check(pulse)?;
        for _ in 0..quota.max(1) {
            // The window that answers the decoder's first request, every step.
            let input = match decoder.requested().first() {
                Some(InputRequest::AtLeast { offset, .. }) => {
                    Input::new(*offset, &bytes[at(*offset)..]).with_total_len(Some(len))
                }
                Some(InputRequest::Range(range)) => {
                    Input::new(range.offset, &bytes[at(range.offset)..at(range.end())])
                        .with_total_len(Some(len))
                }
                _ => Input::whole(bytes),
            };
            let lent = (input.offset, input.bytes.len());
            if self.stalled == Some(lent) {
                return Err(error_msg(
                    ErrorKind::Protocol,
                    "decoder asked again for input it already holds",
                ));
            }
            let step = decoder.step(input, sink, pulse)?;
            check(pulse)?;
            if step.consumed > lent.1 {
                return Err(error_msg(
                    ErrorKind::Protocol,
                    "decoder consumed more than it was lent",
                ));
            }
            self.stalled =
                (step.event == DecodeEvent::NeedInput && step.consumed == 0).then_some(lent);
            if step.event == DecodeEvent::Done {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

/// Run a prepared decoder over a complete input.
///
/// Every request is answered with a sub-slice of `bytes`: nothing is copied
/// or buffered on the way in. Returns the decoder, whose
/// [`info`](Decoder::info) is now complete.
pub fn decode_slice(
    mut decoder: Box<dyn Decoder>,
    bytes: &[u8],
    sink: &mut dyn Sink,
    pulse: &dyn Pulse,
) -> Result<Box<dyn Decoder>> {
    let mut run = SliceRun::new();
    while !run.run(&mut *decoder, bytes, sink, usize::MAX, pulse)? {}
    Ok(decoder)
}

/// One encoder step. A step that succeeds cannot hide a stop raised while it
/// ran; the encoder's own error comes first.
pub(crate) fn encoder_step(
    encoder: &mut dyn Encoder,
    input: EncodeInput<'_>,
    pulse: &dyn Pulse,
) -> Result<EncodeStep> {
    let step = encoder.step(input, pulse)?;
    check(pulse)?;
    Ok(step)
}

/// Answer an encoder's requests for rows from one frame held in memory.
///
/// Returns the first event that is not a request for this frame's rows. This
/// one loop serves an encoder that wants strips, the whole frame, or the
/// frame several times over.
pub fn feed_frame(
    encoder: &mut dyn Encoder,
    frame: &PixelSlice<'_>,
    mut event: EncodeEvent,
    pulse: &dyn Pulse,
) -> Result<EncodeEvent> {
    loop {
        let EncodeEvent::NeedRows { y, .. } = event else {
            return Ok(event);
        };
        if y > frame.rows() {
            return Err(error_msg(
                ErrorKind::Protocol,
                "encoder asked past the frame",
            ));
        }
        let rest = frame.sub_rows(y, frame.rows() - y);
        let step = encoder_step(encoder, EncodeInput::Rows(rest), pulse)?;
        if step.rows == 0 && step.event == event {
            return Err(error_msg(
                ErrorKind::Protocol,
                "encoder took nothing from the rest of the frame",
            ));
        }
        event = step.event;
    }
}

/// Encode one image held in memory. The encoder's output allocation is
/// returned as is.
///
/// [`EncodeSpec::for_pixels`] builds a spec with the image's own size, format
/// and color.
pub fn encode(
    codec: &dyn EncodeCodec,
    spec: EncodeSpec,
    pixels: PixelSlice<'_>,
    pulse: &dyn Pulse,
) -> Result<Vec<u8>> {
    let mut encoder = codec.encoder(&spec.with_supply(RowSupply::Whole))?;
    let first = encoder_step(&mut *encoder, EncodeInput::None, pulse)?.event;
    match feed_frame(&mut *encoder, &pixels, first, pulse)? {
        EncodeEvent::Done => Ok(encoder.output().take()),
        _ => Err(error_msg(
            ErrorKind::Protocol,
            "encoder wants more than one frame",
        )),
    }
}

/// Encode a sequence of frames held in memory.
pub fn encode_sequence<'a>(
    codec: &dyn EncodeCodec,
    spec: EncodeSpec,
    frames: impl IntoIterator<Item = (PixelSlice<'a>, FrameMeta)>,
    pulse: &dyn Pulse,
) -> Result<Vec<u8>> {
    if spec.sequence.is_none() {
        return Err(error_msg(
            ErrorKind::Protocol,
            "the spec does not describe a sequence",
        ));
    }
    let mut encoder = codec.encoder(&spec.with_supply(RowSupply::Whole))?;
    let mut event = encoder_step(&mut *encoder, EncodeInput::None, pulse)?.event;
    for (pixels, meta) in frames {
        if event != EncodeEvent::NeedFrame {
            return Err(error_msg(
                ErrorKind::Protocol,
                "the encoder is not between frames",
            ));
        }
        event = encoder_step(&mut *encoder, EncodeInput::Frame(meta), pulse)?.event;
        event = feed_frame(&mut *encoder, &pixels, event, pulse)?;
    }
    if event != EncodeEvent::NeedFrame {
        return Err(error_msg(
            ErrorKind::Protocol,
            "the encoder is not between frames",
        ));
    }
    match encoder_step(&mut *encoder, EncodeInput::End, pulse)?.event {
        EncodeEvent::Done => Ok(encoder.output().take()),
        _ => Err(error(ErrorKind::Protocol)),
    }
}

/// Decode `bytes` with one codec and encode with another, row by row.
///
/// `spec` builds the encode request from the first decoded frame's header.
/// Both sides observe `pulse`.
pub fn transcode(
    decoder: Box<dyn Decoder>,
    bytes: &[u8],
    codec: Arc<dyn EncodeCodec>,
    spec: impl FnMut(&FrameHeader) -> EncodeSpec + Send + 'static,
    pulse: &dyn Pulse,
) -> Result<Vec<u8>> {
    let mut sink = EncoderSink::new(codec, spec, pulse);
    decode_slice(decoder, bytes, &mut sink, pulse)?;
    sink.finish()?;
    Ok(sink.take_output())
}
