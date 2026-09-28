//! Generate native fixture bitstreams and an independently authored pixel manifest.
use sha2::{Digest, Sha256};
use std::{error::Error, fs, path::PathBuf};
use zencodec_media_codec_tests::{Format, fixtures};

fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let directory = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("output directory required")?,
    );
    fs::create_dir_all(&directory)?;
    let mut entries = Vec::new();
    for fixture in fixtures() {
        let frames: Vec<_> = fixture
            .frames
            .iter()
            .zip(&fixture.durations)
            .map(|(frame, duration)| {
                let mut hash = Sha256::new();
                for y in 0..frame.height() {
                    hash.update(frame.as_slice().row(y));
                }
                serde_json::json!({
                    "rgba8_sha256": format!("{:x}", hash.finalize()),
                    "duration": [duration.numerator(), u64::from(duration.denominator())]
                })
            })
            .collect();
        for format in Format::ALL {
            let filename = format!("{}.{}", fixture.name, format.extension());
            let encoded = fixture.encode(format)?;
            fs::write(directory.join(&filename), &encoded)?;
            entries.push(serde_json::json!({
                "file": filename,
                "format": format,
                "width": fixture.frames[0].width(),
                "height": fixture.frames[0].height(),
                "total_plays": fixture.plays,
                "sha256": format!("{:x}", Sha256::digest(&encoded)),
                "bytes": encoded.len(),
                "frames": frames,
            }));
        }
    }
    let manifest = serde_json::json!({
        "schema": 1,
        "pixel_source": "crates/zencodec-media-codec-tests/src/lib.rs:fixtures",
        "pixel_format": "RGBA8 straight-alpha sRGB, tightly packed rows",
        "codec_revisions": serde_json::from_str::<serde_json::Value>(include_str!("../../../integration/repos.lock.json"))?,
        "fixtures": entries,
    });
    fs::write(
        directory.join("manifest.json"),
        format!("{}\n", serde_json::to_string_pretty(&manifest)?),
    )?;
    Ok(())
}
