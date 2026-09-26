//! Used by scripts/audit-exif-privacy.py: raw EXIF in, filtered EXIF out.
//! Output directories must be fresh; never edits the source files.
use std::{env, fs, path::PathBuf};
use zencodec::{Exif, Metadata, MetadataPolicy};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err("usage: audit_exif_privacy INPUT_DIRECTORY OUTPUT_DIRECTORY".into());
    }
    let input = PathBuf::from(&args[0]);
    let output = PathBuf::from(&args[1]);
    fs::create_dir(&output)?;
    let mut counts = [0usize; 4]; // input, parsed, Web output, ColorAndRotation output
    for item in fs::read_dir(input)? {
        let path = item?.path();
        let bytes = fs::read(&path)?;
        counts[0] += 1;
        let parsed = Exif::parse(&bytes);
        counts[1] += usize::from(parsed.is_some());
        let orientation = parsed.and_then(|x| x.orientation()).unwrap_or_default();
        let meta = Metadata::none()
            .with_exif(bytes)
            .with_orientation(orientation);
        for (i, policy) in [MetadataPolicy::Web, MetadataPolicy::ColorAndRotation]
            .iter()
            .enumerate()
        {
            let out = meta.filtered(policy);
            assert_eq!(out.orientation, orientation, "orientation field changed");
            assert_eq!(
                out.filtered(policy).exif,
                out.exif,
                "filter not idempotent: {path:?}"
            );
            if let Some(exif) = out.exif {
                let parsed = Exif::parse(&exif).expect("filtered output must parse");
                assert!(!parsed.has_camera() && !parsed.has_gps() && !parsed.has_thumbnail());
                counts[i + 2] += 1;
                // ExifTool expects bare TIFF, not JPEG's APP1 signature.
                let tiff = exif.strip_prefix(b"Exif\0\0").unwrap_or(&exif);
                let name = path.file_stem().unwrap().to_string_lossy();
                fs::write(output.join(format!("{name}-{i}.exif")), tiff)?;
            }
        }
    }
    println!(
        "inputs={} parsed={} web_outputs={} color_outputs={}",
        counts[0], counts[1], counts[2], counts[3]
    );
    Ok(())
}
