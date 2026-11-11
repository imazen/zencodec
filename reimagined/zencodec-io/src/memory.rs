//! Drivers for bytes and pixels that are already in memory.

use crate::collect::{Collect, Frame};
use crate::encoder_sink::EncoderSink;
use std::sync::Arc;
use zencodec::decode::{DecodeCodec, DecodeSpec, Decoder, Sink};
use zencodec::encode::{
    EncodeCodec, EncodeEvent, EncodeInput, EncodeSpec, Encoder, FrameMeta, RowSupply,
};
use zencodec::{ErrorKind, FrameHeader, ImageInfo, Result, error, error_msg};
use zenpixels::PixelSlice;

/// A decoded image.
#[derive(Debug)]
pub struct Decoded {
    /// What the file said about itself, complete.
    pub info: ImageInfo,
    /// The frames asked for.
    pub frames: Vec<Frame>,
}

/// Decode a complete input into owned frames.
pub fn decode(codec: &dyn DecodeCodec, bytes: &[u8], spec: &DecodeSpec) -> Result<Decoded> {
    let mut collect = Collect::new();
    let decoder = decode_slice(codec.decoder(spec)?, bytes, &mut collect)?;
    let info = decoder
        .info()
        .cloned()
        .ok_or_else(|| error_msg(ErrorKind::Protocol, "decode finished without info"))?;
    Ok(Decoded {
        info,
        frames: collect.into_frames(),
    })
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
) -> Result<Box<dyn Decoder>> {
    use zencodec::decode::DecodeEvent;
    use zencodec::{Input, InputRequest};

    let view = |request: Option<&InputRequest>| -> Input<'_> {
        let len = bytes.len() as u64;
        let at = |offset: u64| offset.min(len) as usize;
        match request {
            Some(InputRequest::AtLeast { offset, .. }) => {
                Input::new(*offset, &bytes[at(*offset)..]).with_total_len(Some(len))
            }
            Some(InputRequest::Range(range)) => {
                Input::new(range.offset, &bytes[at(range.offset)..at(range.end())])
                    .with_total_len(Some(len))
            }
            _ => Input::whole(bytes),
        }
    };
    let mut stalled = None;
    loop {
        // The window that answers the decoder's first request, every step.
        let input = view(decoder.requested().first());
        let lent = (input.offset, input.bytes.len());
        if stalled == Some(lent) {
            return Err(error_msg(
                ErrorKind::Protocol,
                "decoder asked again for input it already holds",
            ));
        }
        let step = decoder.step(input, sink)?;
        if step.consumed > lent.1 {
            return Err(error_msg(
                ErrorKind::Protocol,
                "decoder consumed more than it was lent",
            ));
        }
        stalled = (step.event == DecodeEvent::NeedInput && step.consumed == 0).then_some(lent);
        if step.event == DecodeEvent::Done {
            return Ok(decoder);
        }
    }
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
) -> Result<EncodeEvent> {
    loop {
        let EncodeEvent::NeedRows { y, .. } = event else {
            return Ok(event);
        };
        if y > frame.rows() {
            return Err(error_msg(ErrorKind::Protocol, "encoder asked past the frame"));
        }
        let rest = frame.sub_rows(y, frame.rows() - y);
        let step = encoder.step(EncodeInput::Rows(rest))?;
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
pub fn encode(codec: &dyn EncodeCodec, spec: EncodeSpec, pixels: PixelSlice<'_>) -> Result<Vec<u8>> {
    let mut encoder = codec.encoder(&spec.with_supply(RowSupply::Whole))?;
    let first = encoder.step(EncodeInput::None)?.event;
    match feed_frame(&mut *encoder, &pixels, first)? {
        EncodeEvent::Done => Ok(encoder.output().take()),
        _ => Err(error_msg(ErrorKind::Protocol, "encoder wants more than one frame")),
    }
}

/// Encode a sequence of frames held in memory.
pub fn encode_sequence<'a>(
    codec: &dyn EncodeCodec,
    spec: EncodeSpec,
    frames: impl IntoIterator<Item = (PixelSlice<'a>, FrameMeta)>,
) -> Result<Vec<u8>> {
    if spec.sequence.is_none() {
        return Err(error_msg(ErrorKind::Protocol, "the spec does not describe a sequence"));
    }
    let mut encoder = codec.encoder(&spec.with_supply(RowSupply::Whole))?;
    let mut event = encoder.step(EncodeInput::None)?.event;
    for (pixels, meta) in frames {
        if event != EncodeEvent::NeedFrame {
            return Err(error_msg(ErrorKind::Protocol, "the encoder is not between frames"));
        }
        event = encoder.step(EncodeInput::Frame(meta))?.event;
        event = feed_frame(&mut *encoder, &pixels, event)?;
    }
    if event != EncodeEvent::NeedFrame {
        return Err(error_msg(ErrorKind::Protocol, "the encoder is not between frames"));
    }
    match encoder.step(EncodeInput::End)?.event {
        EncodeEvent::Done => Ok(encoder.output().take()),
        _ => Err(error(ErrorKind::Protocol)),
    }
}

/// Decode `bytes` with one codec and encode with another, row by row.
///
/// `spec` builds the encode request from the first decoded frame's header.
pub fn transcode(
    decoder: Box<dyn Decoder>,
    bytes: &[u8],
    codec: Arc<dyn EncodeCodec>,
    spec: impl FnMut(&FrameHeader) -> EncodeSpec + Send + 'static,
) -> Result<Vec<u8>> {
    let mut sink = EncoderSink::new(codec, spec);
    decode_slice(decoder, bytes, &mut sink)?;
    sink.finish()?;
    Ok(sink.take_output())
}
