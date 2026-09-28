//! Runnable local/HTTP extraction and exact animation transcode examples.
use std::{
    fs::File,
    io::{BufReader, Read, Seek, Write},
    path::Path,
    time::Duration,
};
use zencodec::encode::{EncodeJob, EncoderConfig};
use zencodec_media::{
    animation::{AnimationLimits, TimingPolicy, transcode_dyn_with},
    av1_index::{FrameScope, FrameSelection, IndexedAv1},
    color::ChromaLocation,
    display::OutOfRange,
    frame::RgbStorage,
    http::HttpRangeReader,
    source::read_bounded,
    time::{TimeBase, Timestamp},
    video::{VideoRgb, encode_extracted},
};
use zencodec_media_codec_tests::Format;

type Error = Box<dyn std::error::Error + Send + Sync>;
trait ReadSeek: Read + Seek {}
impl<T: Read + Seek> ReadSeek for T {}

fn source(name: &str) -> Result<Box<dyn ReadSeek>, Error> {
    Ok(
        if name.starts_with("http://") || name.starts_with("https://") {
            Box::new(HttpRangeReader::open(
                name,
                256 * 1024,
                Duration::from_secs(30),
            )?)
        } else {
            Box::new(BufReader::new(File::open(name)?))
        },
    )
}
fn format(name: &str) -> Result<Format, Error> {
    match name {
        "gif" => Ok(Format::Gif),
        "apng" => Ok(Format::Apng),
        "webp" => Ok(Format::Webp),
        "jxl" => Ok(Format::Jxl),
        "avif" => Ok(Format::Avif),
        _ => Err("format must be gif, apng, webp, jxl, or avif".into()),
    }
}
fn save(path: &str, bytes: &[u8]) -> Result<(), Error> {
    let path = Path::new(path);
    let directory = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    temporary.write_all(bytes)?;
    temporary.flush()?;
    temporary.persist_noclobber(path)?;
    Ok(())
}
fn main() -> Result<(), Error> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("extract") if args.len() == 10 => {
            let ticks = args[2].parse()?;
            let clock = TimeBase::new(args[3].parse()?, args[4].parse()?)?;
            let selection = match args[5].as_str() {
                "nearest" => FrameSelection::Nearest,
                "before" => FrameSelection::AtOrBefore,
                "after" => FrameSelection::AtOrAfter,
                _ => return Err("selection must be nearest, before, or after".into()),
            };
            let scope = match args[6].as_str() {
                "all" => FrameScope::All,
                "keyframes" => FrameScope::Keyframes,
                _ => return Err("scope must be all or keyframes".into()),
            };
            let chroma = match args[7].as_str() {
                "unknown" => None,
                "center" => Some(ChromaLocation::Center),
                "left" => Some(ChromaLocation::Left),
                "top-left" => Some(ChromaLocation::TopLeft),
                _ => return Err("chroma must be unknown, center, left, or top-left".into()),
            };
            let clipping = match args[8].as_str() {
                "reject" => OutOfRange::Reject,
                "clamp" => OutOfRange::Clamp,
                _ => return Err("packing policy must be reject or clamp".into()),
            };
            let mut settings = rav1d_safe::Settings::default();
            settings.threads = 1;
            settings.max_frame_delay = 1;
            settings.all_layers = false;
            settings.frame_size_limit = 16 * 1024 * 1024;
            let mut index = IndexedAv1::build(
                source(&args[1])?,
                16 * 1024 * 1024,
                100_000,
                1024 * 1024,
                settings,
                None,
            )?;
            let extracted = index
                .extract(Timestamp::new(ticks, clock), selection, scope)?
                .ok_or("no presentation meets the requested selection")?;
            let precision = if extracted.frame().bit_depth() > 8 {
                RgbStorage::U16
            } else {
                RgbStorage::U8
            };
            let mut render = VideoRgb::new(precision, clipping, 256 * 1024 * 1024);
            if let Some(chroma) = chroma {
                render = render.with_unspecified_chroma(chroma);
            }
            let image = encode_extracted(
                &extracted,
                zenpng::PngEncoderConfig::new().job().encoder()?,
                &render,
                None,
            )?;
            save(&args[9], image.data())?;
            println!(
                "presentation={} actual_pts={:?} keyframe={} extraction_packets={}",
                extracted.presentation_index(),
                extracted.frame().timestamp(),
                extracted.frame().is_keyframe(),
                extracted.packets_read()
            );
        }
        Some("transcode" | "transcode-srgb") if args.len() == 5 => {
            let convert_srgb = args[0] == "transcode-srgb";
            let from = format(&args[1])?;
            let to = format(&args[2])?;
            let bytes = read_bounded(source(&args[3])?, 64 * 1024 * 1024, None)?;
            let mut decoder = from.decoder_with_preference(bytes, &[])?;
            let (output, report) = transcode_dyn_with(
                decoder.as_mut(),
                |info, plays| {
                    // Native packed precision is retained through the decoder.
                    // AVIF has at most 12 coded bits; a U16 source is not silently
                    // truncated to make this example accept every destination.
                    if to == Format::Avif && !convert_srgb {
                        let bits = info
                            .source_color
                            .bit_depth
                            .ok_or("source precision is unknown")?;
                        if bits > 8 {
                            return Err("this CLI's AVIF encoder is 8-bit; use the library with an explicit 10/12-bit quantization policy".into());
                        }
                    }
                    to.encoder(plays)
                },
                |pixels| {
                    if !convert_srgb {
                        return Ok(pixels);
                    }
                    if matches!(
                        pixels.descriptor().transfer(),
                        zenpixels::TransferFunction::Pq | zenpixels::TransferFunction::Hlg
                    ) {
                        return Err(
                            "HDR-to-SDR requires an explicit display and tone-map policy".into(),
                        );
                    }
                    let ready = zenpixels_convert::finalize_for_output_with(
                        &pixels,
                        &zenpixels::ColorOrigin::assumed(),
                        zenpixels_convert::output::OutputProfile::Named(zenpixels::Cicp::SRGB),
                        zenpixels::PixelFormat::Rgba8,
                        Some(&zenpixels_convert::cms_moxcms::MoxCms),
                    )?;
                    Ok(ready.into_parts().0)
                },
                TimingPolicy::Exact,
                AnimationLimits::new(1000, 1024 * 1024 * 1024)?,
                None,
            )?;
            save(&args[4], output.data())?;
            println!(
                "frames={} total_plays={:?} changed_durations={}",
                report.frames(),
                report.source_plays(),
                report.changed_durations()
            );
        }
        _ => {
            return Err(concat!(
                "usage:\n",
                "  media extract INPUT TICKS NUM DEN nearest|before|after all|keyframes ",
                "unknown|center|left|top-left reject|clamp OUTPUT.png\n",
                "  media transcode|transcode-srgb FROM TO INPUT OUTPUT\n",
                "INPUT may be a local file or an HTTP(S) Range resource with a strong ETag.\n",
                "Extraction preserves source color signaling; clamp is not tone mapping.\n",
                "Outputs must not exist. Animation inputs/outputs are buffered by their codecs.\n",
                "transcode retains native precision; transcode-srgb explicitly converts to straight RGBA8 sRGB.\n",
                "HDR-to-SDR conversion needs an explicit luminance/tone-map policy through the library."
            )
            .into());
        }
    }
    Ok(())
}
