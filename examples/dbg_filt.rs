use zencodec::exif::{ExifPolicy, Retention};
use zencodec::{IccRetention, Metadata, MetadataFields, MetadataPolicy};
fn main() {
    let data = std::fs::read(std::env::args().nth(1).unwrap()).unwrap();
    let (sel, rest) = data.split_first().unwrap();
    let meta = Metadata::none()
        .with_exif(rest.to_vec())
        .with_xmp(rest.to_vec())
        .with_icc(rest.to_vec());
    let policy = match sel % 6 {
        0 => MetadataPolicy::PreserveExact,
        1 => MetadataPolicy::Preserve,
        2 => MetadataPolicy::Web,
        3 => MetadataPolicy::ColorAndRotation,
        4 => MetadataPolicy::Custom(
            MetadataFields::KEEP_ALL.with_exif(ExifPolicy::KEEP_ALL.with_gps(Retention::Discard)),
        ),
        _ => MetadataPolicy::Custom(
            MetadataFields::DISCARD_ALL
                .with_icc(IccRetention::KeepNonSrgb)
                .with_exif(ExifPolicy::ATTRIBUTED_ORIENTATION),
        ),
    };
    println!("sel%6={}", sel % 6);
    let out = meta.filtered(&policy);
    let out2 = out.filtered(&policy);
    println!("out.exif  ={:02x?}", out.exif);
    println!("out2.exif ={:02x?}", out2.exif);
}
