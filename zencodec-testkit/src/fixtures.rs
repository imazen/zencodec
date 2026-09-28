//! Deterministic metadata fixtures for conformance checks.

use std::vec::Vec;

/// A little-endian TIFF/EXIF blob with the privacy-sensitive structures a
/// retention policy must be able to strip: a GPS sub-IFD, a thumbnail IFD1 (with
/// a duplicated camera `Make`), an IFD0 `Make`, and an IFD0 `Copyright`
/// (a *rights* tag that privacy policies keep), plus camera ownership, device
/// and image identifiers, a timestamp and its UTC offset in the Exif sub-IFD.
///
/// Offsets are fixed by the layout below; [`crate`]'s own tests parse this and
/// assert `has_gps()` / `has_thumbnail()` / `copyright()`, so a wrong offset
/// fails loudly rather than yielding a silently-malformed fixture.
pub fn rich_exif_le() -> Vec<u8> {
    // Segment layout: header(8) | IFD0 | GPS-IFD | IFD1 | overflow pool.
    const IFD0_OFF: u32 = 8;
    const GPS_OFF: u32 = IFD0_OFF + 2 + 5 * 12 + 4;
    const IFD1_OFF: u32 = GPS_OFF + 2 + 12 + 4;
    const EXIF_OFF: u32 = IFD1_OFF + 2 + 4 * 12 + 4;
    const POOL_OFF: u32 = EXIF_OFF + 2 + 5 * 12 + 4;
    const MAKE_OFF: u32 = POOL_OFF; // "TestCam\0" (8)
    const COPYRIGHT_OFF: u32 = POOL_OFF + 8; // "(C) 2026 Test\0" (14)
    const THUMB_OFF: u32 = COPYRIGHT_OFF + 14; // FF D8 FF D9 (4)

    // TIFF type codes.
    const ASCII: u16 = 2;
    const SHORT: u16 = 3;
    const LONG: u16 = 4;

    let mut b = Vec::new();
    let entry = |b: &mut Vec<u8>, tag: u16, kind: u16, count: u32, val: u32| {
        b.extend_from_slice(&tag.to_le_bytes());
        b.extend_from_slice(&kind.to_le_bytes());
        b.extend_from_slice(&count.to_le_bytes());
        b.extend_from_slice(&val.to_le_bytes());
    };

    // Header.
    b.extend_from_slice(b"II");
    b.extend_from_slice(&42u16.to_le_bytes());
    b.extend_from_slice(&IFD0_OFF.to_le_bytes());
    debug_assert_eq!(b.len(), IFD0_OFF as usize);

    // IFD0: Make, Orientation, Copyright, Exif/GPS pointers (ascending tags).
    b.extend_from_slice(&5u16.to_le_bytes());
    entry(&mut b, 0x010F, ASCII, 8, MAKE_OFF); // Make
    entry(&mut b, 0x0112, SHORT, 1, 6); // Orientation = Rotate90 (inline)
    entry(&mut b, 0x8298, ASCII, 14, COPYRIGHT_OFF); // Copyright
    entry(&mut b, 0x8769, LONG, 1, EXIF_OFF);
    entry(&mut b, 0x8825, LONG, 1, GPS_OFF); // GPSInfo IFD pointer
    b.extend_from_slice(&IFD1_OFF.to_le_bytes()); // next IFD = IFD1
    debug_assert_eq!(b.len(), GPS_OFF as usize);

    // GPS IFD: GPSLatitudeRef "N\0" (inline).
    b.extend_from_slice(&1u16.to_le_bytes());
    entry(
        &mut b,
        0x0001,
        ASCII,
        2,
        u32::from_le_bytes([b'N', 0, 0, 0]),
    );
    b.extend_from_slice(&0u32.to_le_bytes()); // next = 0
    debug_assert_eq!(b.len(), IFD1_OFF as usize);

    // IFD1 (thumbnail): Compression, Make, JPEGInterchangeFormat + Length.
    b.extend_from_slice(&4u16.to_le_bytes());
    entry(&mut b, 0x0103, SHORT, 1, 6); // Compression = JPEG
    entry(&mut b, 0x010F, ASCII, 8, MAKE_OFF); // Make (camera id in thumbnail dir)
    entry(&mut b, 0x0201, LONG, 1, THUMB_OFF); // JPEGInterchangeFormat
    entry(&mut b, 0x0202, LONG, 1, 4); // JPEGInterchangeFormatLength
    b.extend_from_slice(&0u32.to_le_bytes()); // next = 0
    debug_assert_eq!(b.len(), EXIF_OFF as usize);
    // ExifIFD: dates, UTC offset, image identity, owner, body serial.
    b.extend_from_slice(&5u16.to_le_bytes());
    entry(&mut b, 0x9003, ASCII, 20, THUMB_OFF + 4);
    entry(&mut b, 0x9011, ASCII, 7, THUMB_OFF + 24);
    for tag in [0xA420, 0xA430, 0xA431] {
        entry(&mut b, tag, ASCII, 4, u32::from_le_bytes(*b"PII\0"));
    }
    b.extend_from_slice(&0u32.to_le_bytes());
    debug_assert_eq!(b.len(), POOL_OFF as usize);

    // Overflow pool.
    b.extend_from_slice(b"TestCam\0"); // 8 @ MAKE_OFF
    b.extend_from_slice(b"(C) 2026 Test\0"); // 14 @ COPYRIGHT_OFF
    debug_assert_eq!(b.len(), THUMB_OFF as usize);
    b.extend_from_slice(&[0xFF, 0xD8, 0xFF, 0xD9]); // thumbnail @ THUMB_OFF
    b.extend_from_slice(b"2026:01:02 03:04:05\0+01:00\0");
    b
}

/// A minimal, syntactically-plausible XMP packet (rights statement).
pub fn sample_xmp() -> Vec<u8> {
    br#"<?xpacket begin="" id="W5M0MpCehiHzreSzNTczkc9d"?><x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:rights>Test</dc:rights></rdf:Description></rdf:RDF></x:xmpmeta><?xpacket end="w"?>"#.to_vec()
}

/// A tiny but valid-enough ICC-shaped blob (not a real profile; opaque bytes the
/// codec is expected to carry verbatim or drop, never inspect).
pub fn sample_icc() -> Vec<u8> {
    sample_icc_with_class(b"RGB ")
}

/// A minimal ICC header (132 bytes, `acsp` signature) whose data colour space
/// field (header bytes 16..20) is `class` — `b"RGB "`, `b"GRAY"`, `b"CMYK"`, or
/// garbage for an unclassifiable profile. The device class is what the
/// buffer-context class gate reads, so the fixtures must carry a real one.
pub fn sample_icc_with_class(class: &[u8; 4]) -> Vec<u8> {
    let mut v = std::vec![0u8; 132];
    v[16..20].copy_from_slice(class); // data colour space (device class)
    v[36..40].copy_from_slice(b"acsp"); // ICC signature at offset 36
    v
}
