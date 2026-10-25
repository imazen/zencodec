use zencodec::{BmpDecoderConfig, CodecSet, QoiDecoderConfig};

pub fn decode_upload(
    bytes: &[u8],
) -> Result<(u32, u32), Box<dyn std::error::Error + Send + Sync>> {
    let codecs = CodecSet::new()
        .with_decoder(QoiDecoderConfig::default())
        .with_decoder(BmpDecoderConfig::default());
    let image = codecs.decode(bytes)?;
    Ok((image.width(), image.height()))
}
