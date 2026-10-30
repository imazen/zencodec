//! Diagnostic witness, not a passing conformance substitute.
//! Exit failure if supplied mastering metadata is not demonstrably retained.
use std::{borrow::Cow, error::Error, io::Write, path::Path};
use zencodec::{
    ContentLightLevel, MasteringDisplay, Metadata, MetadataPolicy,
    decode::{Decode, DecodeJob, DecoderConfig},
    encode::{EncodeJob, Encoder, EncoderConfig},
};
use zencodec_testkit::TestImage;

type Failure = Box<dyn Error + Send + Sync>;

// Same fixed-point tolerances as testkit's established HDR contract.
fn close(got: f32, want: f32) -> bool {
    (got - want).abs()
        <= if want.abs() < 1.0 {
            2e-5
        } else {
            want.abs() * 1e-5
        }
}
fn mastering_matches(got: &MasteringDisplay, want: &MasteringDisplay) -> bool {
    got.primaries_xy
        .iter()
        .zip(&want.primaries_xy)
        .all(|(g, w)| close(g[0], w[0]) && close(g[1], w[1]))
        && close(got.white_point_xy[0], want.white_point_xy[0])
        && close(got.white_point_xy[1], want.white_point_xy[1])
        && close(got.max_luminance, want.max_luminance)
        && close(got.min_luminance, want.min_luminance)
}
fn encode<E>(config: &E, pixels: &TestImage, metadata: Metadata) -> Result<Vec<u8>, Failure>
where
    E: EncoderConfig,
    <E::Job as EncodeJob>::Enc: Encoder<Error = E::Error>,
{
    Ok(config
        .clone()
        .job()
        .with_metadata_policy(metadata, MetadataPolicy::PreserveExact)
        .encoder()?
        .encode(pixels.as_slice())?
        .into_vec())
}
fn probe<E, D>(name: &str, encoder: E, decoder: D, directory: &Path) -> Result<usize, Failure>
where
    E: EncoderConfig,
    D: DecoderConfig,
    <E::Job as EncodeJob>::Enc: Encoder<Error = E::Error>,
{
    let pixels = TestImage::rgb8_gradient(97, 65);
    let baseline = encode(&encoder, &pixels, Metadata::none())?;
    save(directory, name, "empty", &baseline)?;
    let empty = decoder
        .clone()
        .job()
        .decoder(Cow::Borrowed(&baseline), &[])?
        .decode()?;
    println!(
        "{name}\tcontrol\tcll={:?}\tmd={:?}",
        empty.metadata().content_light_level,
        empty.metadata().mastering_display
    );
    let cll = ContentLightLevel::new(1000, 400);
    let md = MasteringDisplay::HDR10_REFERENCE;
    let mut missing = 0;
    for (case, metadata) in [
        ("cll", Metadata::none().with_content_light_level(cll)),
        ("mastering", Metadata::none().with_mastering_display(md)),
        (
            "both",
            Metadata::none()
                .with_content_light_level(cll)
                .with_mastering_display(md),
        ),
    ] {
        let encoded = match encode(&encoder, &pixels, metadata.clone()) {
            Ok(bytes) => bytes,
            Err(error) => {
                println!("{name}\t{case}\tencode-error-unqualified\t{error}");
                missing += 1;
                continue;
            }
        };
        save(directory, name, case, &encoded)?;
        let out = decoder
            .clone()
            .job()
            .decoder(Cow::Borrowed(&encoded), &[])?
            .decode()?;
        let got = out.metadata();
        let retained = metadata
            .content_light_level
            .is_none_or(|want| got.content_light_level == Some(want))
            && metadata.mastering_display.is_none_or(|want| {
                got.mastering_display
                    .is_some_and(|got| mastering_matches(&got, &want))
            });
        println!(
            "{name}\t{case}\tretained={retained}\tidentical-to-empty={}\tcll={:?}\tmd={:?}",
            encoded == baseline,
            got.content_light_level,
            got.mastering_display
        );
        missing += usize::from(!retained);
    }
    Ok(missing)
}
fn save(directory: &Path, codec: &str, case: &str, bytes: &[u8]) -> Result<(), Failure> {
    let path = directory.join(format!("{codec}-{case}.{codec}"));
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?
        .write_all(bytes)?;
    Ok(())
}
fn main() -> Result<(), Failure> {
    let directory = std::env::args_os()
        .nth(1)
        .ok_or("provide a fresh artifact directory")?;
    let directory = Path::new(&directory);
    std::fs::create_dir_all(directory)?;
    let missing = probe(
        "jpeg",
        zenjpeg::JpegEncoderConfig::new(),
        zenjpeg::JpegDecoderConfig::new(),
        directory,
    )? + probe(
        "png",
        zenpng::PngEncoderConfig::new(),
        zenpng::PngDecoderConfig::new(),
        directory,
    )?;
    if missing != 0 {
        return Err(format!(
            "{missing}/6 metadata retention routes unqualified; no capability-based waiver applied"
        )
        .into());
    }
    Ok(())
}
