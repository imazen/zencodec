use std::borrow::Cow;
use std::error::Error;
use zenbitmaps::QoiDecoderConfig;
use zencodec::decode::{Decode, DecodeJob, DecodeOutput, DecoderConfig};
use zencodec::ResourceLimits;

pub fn decode_limited(
    bytes: &[u8],
    max_pixels: u64,
) -> Result<DecodeOutput, Box<dyn Error + Send + Sync>> {
    let config = QoiDecoderConfig::new();
    let limits = ResourceLimits::none().with_max_pixels(max_pixels);
    let image = config
        .job()
        .with_limits(limits)
        .decoder(Cow::Borrowed(bytes), &[])?
        .decode()?;
    Ok(image)
}
