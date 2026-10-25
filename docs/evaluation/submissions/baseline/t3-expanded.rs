use zenbitmaps::QoiDecoderConfig;
use zencodec::decode::{DecodeJob, DecoderConfig};
use zencodec::source::ReadAt;
use zencodec_io::{CollectSink, DriveOptions, drive_from_read_at};

pub fn decode_positioned(
    source: &mut impl ReadAt,
) -> Result<CollectSink, Box<dyn std::error::Error + Send + Sync>> {
    let config = QoiDecoderConfig::new();
    let mut decoder = config.job().dyn_incremental_decoder(&[])?;
    let mut sink = CollectSink::new();
    drive_from_read_at(
        decoder.as_mut(),
        source,
        &mut sink,
        None,
        DriveOptions::default(),
    )
    .map_err(|e| e.to_string())?;
    Ok(sink)
}
