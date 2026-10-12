use super::*;
use crate::{Metadata, MetadataPolicy};
use alloc::vec;

const SECRET: &[u8] = b"private-owner-serial-location\0";

fn entry(tag: u16, kind: u16, count: u32, value: &[u8]) -> Entry<'static> {
    Entry {
        tag,
        kind,
        count,
        value: Cow::Owned(value.to_vec()),
        value_offset: 0,
    }
}

fn text(tag: u16) -> Entry<'static> {
    entry(tag, TIFF_ASCII, SECRET.len() as u32, SECRET)
}

fn fixture(order: ByteOrder) -> Exif<'static> {
    let mut x = Exif::new(TextEncoding::Ascii);
    x.order = order;
    x.set_orientation(Orientation::Rotate90);
    x.set_copyright("Public attribution");
    // Standard EXIF, DNG, Windows XP, free text, embedded metadata and private
    // carriers, as identified in ExifTool's Exif.pm. No real people's data.
    let sensitive = [
        TAG_CAMERA_OWNER_NAME,
        TAG_BODY_SERIAL_NUMBER,
        TAG_LENS_SERIAL_NUMBER,
        TAG_DNG_CAMERA_SERIAL,
        TAG_CAMERA_LABEL,
        TAG_IMAGE_UNIQUE_ID,
        TAG_HOST_COMPUTER,
        TAG_MAKER_NOTE,
        0x010D,
        0x010E,
        0x9286,
        0x9C9B,
        0x9C9C,
        0x9C9D,
        0x9C9E,
        0x9C9F,
        0x02BC,
        0x83BB,
        0x8649,
        0xC634,
        0xC68B,
        0xC68C,
        0xC6F3,
        TAG_PSRAW_OWNER,
        TAG_PSRAW_SERIAL,
    ];
    // Duplicate/wrong-directory identifiers must not evade removal.
    x.ifd0.extend(sensitive.into_iter().map(text));
    let mut exif: Vec<_> = sensitive.into_iter().map(text).collect();
    let color = [0xFF, 0xFF];
    exif.push(entry(TAG_COLOR_SPACE, TIFF_SHORT, 1, &color));
    x.exif_ifd = Some(exif);
    x.interop_ifd = Some(vec![
        entry(TAG_INTEROP_INDEX, TIFF_ASCII, 4, b"R03\0"),
        entry(TAG_INTEROP_VERSION, TIFF_UNDEFINED, 4, b"0100"),
        text(0x1000), // RelatedImageFileFormat is not a color declaration.
        text(TAG_CAMERA_OWNER_NAME),
        text(0xC001),
    ]);
    x.gps_ifd = Some(vec![text(0x001B)]); // GPSProcessingMethod
    x.ifd1 = Some(sensitive.into_iter().map(text).collect());
    x.thumbnail = Some(SECRET); // Thumbnail bytes can themselves embed metadata.
    x
}

#[test]
fn publishing_removes_identifiers_in_all_directories_and_preserves_color() {
    for order in [ByteOrder::Little, ByteOrder::Big] {
        for prefix in [false, true] {
            let mut x = fixture(order);
            x.had_prefix = prefix;
            let mut src = x.to_bytes();
            src.extend_from_slice(SECRET); // Unreferenced/trailing bytes must go too.
            for policy in [MetadataPolicy::Web, MetadataPolicy::ColorAndRotation] {
                let original = Metadata::none()
                    .with_exif(src.clone())
                    .with_xmp(SECRET.to_vec())
                    .with_orientation(Orientation::Rotate90);
                let filtered = original.filtered(&policy);
                assert!(filtered.xmp.is_none());
                let bytes = filtered.exif.as_deref().unwrap();
                assert!(!bytes.windows(SECRET.len()).any(|w| w == SECRET));
                let actual = Exif::parse(bytes).unwrap();
                assert_eq!(actual.orientation(), Some(Orientation::Rotate90));
                assert!(!actual.has_camera() && !actual.has_gps() && !actual.has_thumbnail());
                assert_eq!(actual.interop_ifd.as_ref().unwrap().len(), 2);
                assert_eq!(actual.copyright().is_some(), policy == MetadataPolicy::Web);
                assert_eq!(filtered.filtered(&policy).exif.as_deref(), Some(bytes));
                // Independent TIFF/EXIF parser must see the intended color.
                let tiff = bytes.strip_prefix(EXIF_PREFIX).unwrap_or(bytes);
                let (fields, _) = ::exif::parse_exif(tiff).unwrap();
                assert!(fields.iter().any(
                    |f| f.tag == ::exif::Tag::ColorSpace && f.value.get_uint(0) == Some(65535)
                ));
                assert!(fields.iter().any(|f| f.tag == ::exif::Tag::InteroperabilityIndex
                    && matches!(&f.value, ::exif::Value::Ascii(v) if v == &[b"R03".to_vec()])));
            }
            assert_eq!(retain(&src, &ExifPolicy::KEEP_ALL).unwrap().as_ref(), src);
        }
    }
}

#[test]
fn camera_and_time_opt_ins_do_not_enable_sensitive_siblings() {
    let mut x = fixture(ByteOrder::Little);
    x.ifd0.push(entry(TAG_MAKE, TIFF_ASCII, 4, b"Cam\0"));
    x.exif_ifd
        .as_mut()
        .unwrap()
        .extend([text(TAG_DATETIME_ORIGINAL), text(TAG_OFFSET_TIME_ORIGINAL)]);
    let p = ExifPolicy::ATTRIBUTED_ORIENTATION
        .with_camera(Retention::Keep)
        .with_datetimes(Retention::Keep);
    let kept = x.filtered(&p);
    assert!(kept.has_camera() && kept.has_datetimes());
    assert!(
        !kept.has_camera_owner()
            && !kept.has_device_ids()
            && !kept.has_image_unique_id()
            && !kept.has_time_offsets()
    );
    assert_eq!(kept.copyright().as_deref(), Some("Public attribution"));

    // Removing camera information from a keep-all policy must remain safe,
    // even after a later descriptive-only opt-in.
    let p = ExifPolicy::KEEP_ALL
        .with_camera(Retention::Discard)
        .with_camera(Retention::Keep);
    let kept = x.filtered(&p);
    assert!(kept.has_camera());
    assert!(!kept.has_camera_owner() && !kept.has_device_ids() && !kept.has_image_unique_id());
    for (p, owner, device, image) in [
        (
            ExifPolicy::DISCARD_ALL.with_camera_owner(Retention::Keep),
            true,
            false,
            false,
        ),
        (
            ExifPolicy::DISCARD_ALL.with_device_ids(Retention::Keep),
            false,
            true,
            false,
        ),
        (
            ExifPolicy::DISCARD_ALL.with_image_unique_id(Retention::Keep),
            false,
            false,
            true,
        ),
    ] {
        let kept = x.filtered(&p);
        assert_eq!(kept.has_camera_owner(), owner);
        assert_eq!(kept.has_device_ids(), device);
        assert_eq!(kept.has_image_unique_id(), image);
        assert!(!p.keeps_everything());
    }
}

// Regression derived from the repro corpus: a TIFF stores ColorSpace in IFD0.
// Retained tags must be valid in their directory as well as their type/count.
#[test]
fn misplaced_allowlisted_tags_are_not_trusted() {
    let mut x = Exif::new(TextEncoding::Ascii);
    x.ifd0 = vec![
        entry(TAG_COLOR_SPACE, TIFF_SHORT, 1, &[1, 0]),
        entry(TAG_PHOTOGRAPHER, TIFF_ASCII, 4, b"PII\0"),
    ];
    x.exif_ifd = Some(vec![
        entry(TAG_ORIENTATION, TIFF_SHORT, 1, &[6, 0]),
        entry(TAG_COPYRIGHT, TIFF_ASCII, 4, b"PII\0"),
    ]);
    assert!(retain(&x.to_bytes(), &ExifPolicy::ATTRIBUTED_ORIENTATION).is_none());
}

#[test]
fn malformed_display_tags_cannot_smuggle_arbitrary_payloads() {
    for order in [ByteOrder::Little, ByteOrder::Big] {
        let mut x = Exif::new(TextEncoding::Ascii);
        x.order = order;
        x.ifd0 = vec![
            text(TAG_ORIENTATION),
            text(TAG_COLOR_SPACE),
            text(TAG_GAMMA),
        ];
        x.exif_ifd = Some(x.ifd0.clone());
        x.interop_ifd = Some(vec![text(TAG_INTEROP_INDEX), text(TAG_INTEROP_VERSION)]);
        let src = x.to_bytes();
        assert!(retain(&src, &ExifPolicy::ORIENTATION_ONLY).is_none());
        // Right type but excessive count must also be dropped.
        for (tag, kind, count, value) in [
            (TAG_ORIENTATION, TIFF_SHORT, 3, &[6, 0, 65, 66, 67, 68][..]),
            (TAG_COLOR_SPACE, TIFF_SHORT, 3, &[1, 0, 65, 66, 67, 68][..]),
            (TAG_GAMMA, TIFF_RATIONAL, 2, &[65; 16][..]),
            (TAG_GAMMA, TIFF_RATIONAL, 1, &[0; 8][..]),
        ] {
            x.ifd0 = vec![entry(tag, kind, count, value)];
            x.exif_ifd = None;
            x.interop_ifd = None;
            assert!(retain(&x.to_bytes(), &ExifPolicy::ORIENTATION_ONLY).is_none());
        }
    }
}

#[test]
fn truncated_and_mutated_exif_is_fail_closed_and_idempotent() {
    let source = fixture(ByteOrder::Little).to_bytes();
    let check = |src: &[u8]| {
        if let Some(out) = retain(src, &ExifPolicy::ORIENTATION_ONLY) {
            assert!(!out.windows(SECRET.len()).any(|w| w == SECRET));
            assert_eq!(
                retain(&out, &ExifPolicy::ORIENTATION_ONLY).as_deref(),
                Some(out.as_ref())
            );
        }
    };
    for end in 0..source.len() {
        check(&source[..end]);
    }
    // Invalid counts, types, offsets, cycles/aliases, and duplicated tag IDs.
    for i in 0..source.len() {
        for replacement in [0, 0xFF] {
            let mut src = source.clone();
            src[i] = replacement;
            check(&src);
        }
    }
}
