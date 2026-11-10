//! Caller-facing examples, compiled and exercised by zencodec-testkit.
use zencodec::{CodecSet, CodecSetError, ImageFormat, encode::EncodeOutput};

pub fn convert_pixels(
    codecs: &CodecSet,
    bytes: &[u8],
    format: ImageFormat,
) -> Result<EncodeOutput, CodecSetError> {
    // EXAMPLE: simple_convert
    // ANCHOR: simple_convert
    let image = codecs.decode(bytes)?;
    let output = codecs.encode(format, image.pixels())?;
    // END: simple_convert
    // END EXAMPLE: simple_convert
    Ok(output)
}
