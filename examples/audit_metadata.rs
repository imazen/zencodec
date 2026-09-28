//! Inspect/diff extracted EXIF or XMP without decoding pixels.
//! cargo run --example audit_metadata --features xmp -- exif before.exif [after.exif]
use std::{error::Error, io::Read};
use zencodec::metadata_audit::Report;
fn read(path: &str, kind: &str) -> Result<Report, Box<dyn Error>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err("metadata input exceeds 16 MiB".into());
    }
    match kind {
        "exif" => Ok(Report::exif(&bytes)),
        "xmp" => Ok(Report::xmp(std::str::from_utf8(&bytes)?)),
        _ => Err("expected exif or xmp".into()),
    }
}
fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().collect();
    if !(3..=4).contains(&args.len()) {
        return Err("usage: audit_metadata exif|xmp before [after]".into());
    }
    let before = read(&args[2], &args[1])?;
    for finding in before.findings() {
        println!("before finding: {finding:?}");
    }
    if let Some(path) = args.get(3) {
        let after = read(path, &args[1])?;
        for finding in after.findings() {
            println!("after finding: {finding:?}");
        }
        for change in before.diff(&after) {
            println!("{change:?}");
        }
    } else {
        for field in before.entries() {
            println!("{} [{}] {:?}", field.path(), field.kind(), field.value());
        }
    }
    Ok(())
}
