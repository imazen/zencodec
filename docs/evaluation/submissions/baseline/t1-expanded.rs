use zenbitmaps::{BmpDecoderConfig, QoiDecoderConfig};
use zencodec::CodecSet;

pub fn decode_upload(
    bytes: &[u8],
) -> Result<(u32, u32), Box<dyn std::error::Error + Send + Sync>> {
    let codecs = CodecSet::new()
        .with_decoder(QoiDecoderConfig::new())
        .with_decoder(BmpDecoderConfig::new());
    let image = codecs.decode(bytes)?;
    Ok((image.width(), image.height()))
}
