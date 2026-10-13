#![no_main]
//! `Metadata::filtered` integration: build Metadata from arbitrary EXIF/XMP/ICC
//! bytes and apply every policy. Must never panic, and the result must be
//! internally consistent (re-filtering is idempotent under the same policy).
use libfuzzer_sys::fuzz_target;
use zencodec::exif::{ExifPolicy, Retention};
use zencodec::{IccRetention, Metadata, MetadataFields, MetadataPolicy};

fuzz_target!(|data: &[u8]| {
    let (sel, rest) = match data.split_first() {
        Some(x) => x,
        None => return,
    };
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

    let out = meta.filtered(&policy);
    // Idempotence: filtering the result again with the same policy is stable
    // (catches a filter that produces output it can't re-process).
    let out2 = out.filtered(&policy);
    assert_eq!(out.exif, out2.exif, "filtered EXIF not idempotent");
    assert_eq!(out.orientation, out2.orientation);
});
