//! Differential tests: parse the same EXIF blobs with `zencodec::exif::Exif`
//! and the mature `kamadak-exif` crate, and assert the accessor outputs agree.
//!
//! Scope: well-formed blobs where parity is meaningful (orientation as
//! SHORT/LONG, copyright/artist inline + out-of-line, both byte orders). The
//! oracle's raw path doesn't strip the `Exif\0\0` prefix, so the harness does.
//! Behavioral seams where zencodec is deliberately lenient (missing next-IFD
//! offset, child next pointers, >8 IFD chains) are out of scope here — those
//! are covered by the in-crate unit tests and fuzzing.

use exif::{In, Tag, Value};
use zencodec::exif::Exif;

/// Read orientation + copyright + artist from the oracle (`kamadak-exif`).
fn oracle(blob: &[u8]) -> Option<(Option<u32>, Option<String>, Option<String>)> {
    let tiff: &[u8] = blob.strip_prefix(b"Exif\0\0").unwrap_or(blob);
    let (fields, _le) = exif::parse_exif(tiff).ok()?;
    let get = |t: Tag| {
        fields
            .iter()
            .find(|f| f.tag == t && f.ifd_num == In::PRIMARY)
    };
    let orientation = get(Tag::Orientation).and_then(|f| f.value.get_uint(0));
    let ascii = |t: Tag| -> Option<String> {
        match &get(t)?.value {
            Value::Ascii(v) if !v.is_empty() && !v[0].is_empty() => {
                Some(String::from_utf8_lossy(&v[0]).into_owned())
            }
            _ => None,
        }
    };
    Some((orientation, ascii(Tag::Copyright), ascii(Tag::Artist)))
}

/// Build a well-formed TIFF (`be` = big-endian). IFD0 at offset 8 with an
/// orientation entry (SHORT or LONG) plus optional copyright/artist ASCII
/// entries (out-of-line when > 4 bytes). Tag-sorted: 0x0112 < 0x013B < 0x8298.
fn build(
    be: bool,
    orientation: u16,
    ori_long: bool,
    copyright: Option<&str>,
    artist: Option<&str>,
) -> Vec<u8> {
    let w16 = |v: &mut Vec<u8>, x: u16| {
        v.extend_from_slice(&if be { x.to_be_bytes() } else { x.to_le_bytes() })
    };
    let w32 = |v: &mut Vec<u8>, x: u32| {
        v.extend_from_slice(&if be { x.to_be_bytes() } else { x.to_le_bytes() })
    };

    // Collect entries as (tag, type, count, inline-or-offset value bytes).
    struct E {
        tag: u16,
        kind: u16,
        count: u32,
        inline: Option<[u8; 4]>,
        ext: Vec<u8>,
    }
    let mut entries: Vec<E> = Vec::new();

    // Orientation.
    if ori_long {
        let mut v = [0u8; 4];
        v.copy_from_slice(&if be {
            u32::from(orientation).to_be_bytes()
        } else {
            u32::from(orientation).to_le_bytes()
        });
        entries.push(E {
            tag: 0x0112,
            kind: 4,
            count: 1,
            inline: Some(v),
            ext: Vec::new(),
        });
    } else {
        let mut v = [0u8; 4];
        let b = if be {
            orientation.to_be_bytes()
        } else {
            orientation.to_le_bytes()
        };
        v[..2].copy_from_slice(&b);
        entries.push(E {
            tag: 0x0112,
            kind: 3,
            count: 1,
            inline: Some(v),
            ext: Vec::new(),
        });
    }
    // Artist (0x013B) then Copyright (0x8298) — ASCII, NUL-terminated.
    let push_ascii = |entries: &mut Vec<E>, tag: u16, s: &str| {
        let mut bytes = s.as_bytes().to_vec();
        bytes.push(0);
        if bytes.len() <= 4 {
            let mut v = [0u8; 4];
            v[..bytes.len()].copy_from_slice(&bytes);
            entries.push(E {
                tag,
                kind: 2,
                count: bytes.len() as u32,
                inline: Some(v),
                ext: Vec::new(),
            });
        } else {
            entries.push(E {
                tag,
                kind: 2,
                count: bytes.len() as u32,
                inline: None,
                ext: bytes,
            });
        }
    };
    if let Some(a) = artist {
        push_ascii(&mut entries, 0x013B, a);
    }
    if let Some(c) = copyright {
        push_ascii(&mut entries, 0x8298, c);
    }

    let n = entries.len();
    let ext_base = 8 + 2 + 12 * n + 4; // header + count + entries + next-IFD

    let mut v = Vec::new();
    v.extend_from_slice(if be { b"MM" } else { b"II" });
    w16(&mut v, 42);
    w32(&mut v, 8);
    w16(&mut v, n as u16);
    let mut ext = Vec::new();
    for e in &entries {
        w16(&mut v, e.tag);
        w16(&mut v, e.kind);
        w32(&mut v, e.count);
        match &e.inline {
            Some(b) => v.extend_from_slice(b),
            None => {
                w32(&mut v, (ext_base + ext.len()) as u32);
                ext.extend_from_slice(&e.ext);
                if ext.len() % 2 == 1 {
                    ext.push(0);
                }
            }
        }
    }
    w32(&mut v, 0); // next-IFD offset
    v.extend_from_slice(&ext);
    v
}

#[test]
fn differential_orientation_copyright_artist() {
    let mut compared = 0usize;
    for &be in &[false, true] {
        for &ori_long in &[false, true] {
            for ori in 1u16..=8 {
                for copyright in [None, Some("(c)"), Some("Copyright 2026 Lilith")] {
                    for artist in [None, Some("Me"), Some("Lilith Ver{}er")] {
                        let blob = build(be, ori, ori_long, copyright, artist);

                        // zencodec must always parse a well-formed blob.
                        let x = Exif::parse(&blob).expect("zencodec parses well-formed blob");
                        let zen = (
                            x.orientation().map(|o| u32::from(o.to_exif())),
                            x.copyright().map(|c| c.into_owned()),
                            x.artist().map(|a| a.into_owned()),
                        );

                        // Oracle: where it agrees to parse, accessor outputs must match.
                        if let Some(orc) = oracle(&blob) {
                            assert_eq!(
                                zen.0, orc.0,
                                "orientation mismatch (be={be}, long={ori_long}, ori={ori})"
                            );
                            assert_eq!(zen.1, orc.1, "copyright mismatch ({copyright:?})");
                            assert_eq!(zen.2, orc.2, "artist mismatch ({artist:?})");
                            compared += 1;
                        }
                    }
                }
            }
        }
    }
    // Sanity: the oracle actually parsed a substantial share, so the assertions ran.
    assert!(compared >= 100, "too few oracle comparisons: {compared}");
}

#[test]
fn differential_exif_prefix_framing() {
    let bare = build(false, 6, false, Some("Copyright 2026"), None);
    let mut prefixed = b"Exif\0\0".to_vec();
    prefixed.extend_from_slice(&bare);

    let x = Exif::parse(&prefixed).expect("parses prefixed");
    let orc = oracle(&prefixed).expect("oracle parses (after prefix strip)");
    assert_eq!(x.orientation().map(|o| u32::from(o.to_exif())), orc.0);
    assert_eq!(x.copyright().map(|c| c.into_owned()), orc.1);
}

// ── Colour signalling survives every retaining preset ───────────────────────

/// Big-endian TIFF: IFD0 → Exif IFD {ColorSpace, Interop pointer} → Interop
/// IFD {InteropIndex}. This is how a camera declares Adobe RGB without an ICC
/// profile: `ColorSpace = 0xFFFF` (uncalibrated) plus `InteropIndex = "R03"`.
fn build_colour_only_exif(color_space: u16, interop_index: &[u8; 4]) -> Vec<u8> {
    let mut t = vec![b'M', b'M', 0, 0x2A, 0, 0, 0, 8];
    // IFD0 @ 8: one entry (Exif IFD pointer → 26), next = 0.
    t.extend_from_slice(&[0, 1]);
    t.extend_from_slice(&[0x87, 0x69, 0, 4, 0, 0, 0, 1, 0, 0, 0, 26]);
    t.extend_from_slice(&[0, 0, 0, 0]);
    assert_eq!(t.len(), 26);
    // Exif IFD @ 26: ColorSpace SHORT, Interop pointer → 56, next = 0.
    t.extend_from_slice(&[0, 2]);
    t.extend_from_slice(&[0xA0, 0x01, 0, 3, 0, 0, 0, 1]);
    t.extend_from_slice(&color_space.to_be_bytes());
    t.extend_from_slice(&[0, 0]);
    t.extend_from_slice(&[0xA0, 0x05, 0, 4, 0, 0, 0, 1, 0, 0, 0, 56]);
    t.extend_from_slice(&[0, 0, 0, 0]);
    assert_eq!(t.len(), 56);
    // Interop IFD @ 56: InteropIndex ASCII[4], next = 0.
    t.extend_from_slice(&[0, 1]);
    t.extend_from_slice(&[0x00, 0x01, 0, 2, 0, 0, 0, 4]);
    t.extend_from_slice(interop_index);
    t.extend_from_slice(&[0, 0, 0, 0]);
    t
}

/// Oracle view of the colour declaration: (ColorSpace, InteropIndex).
fn oracle_colour(blob: &[u8]) -> (Option<u32>, Option<String>) {
    let (fields, _) = exif::parse_exif(blob).expect("oracle parses");
    let cs = fields
        .iter()
        .find(|f| f.tag == Tag::ColorSpace)
        .and_then(|f| f.value.get_uint(0));
    let idx = fields
        .iter()
        .find(|f| f.tag == Tag::InteroperabilityIndex)
        .map(|f| match &f.value {
            Value::Ascii(v) if !v.is_empty() => String::from_utf8_lossy(&v[0]).into_owned(),
            other => panic!("InteropIndex has type {other:?}"),
        });
    (cs, idx)
}

/// An Adobe RGB camera JPEG whose only colour declaration is EXIF must not
/// become implicit sRGB under a preset that promises to keep colour
/// (`Web` = `ATTRIBUTED_ORIENTATION`, `ColorAndRotation` = `ORIENTATION_ONLY`).
#[test]
fn colour_declaration_survives_retaining_presets() {
    use zencodec::exif::{ExifPolicy, retain};

    let src = build_colour_only_exif(0xFFFF, b"R03\0");
    assert_eq!(
        oracle_colour(&src),
        (Some(0xFFFF), Some("R03".into())),
        "fixture as seen by the oracle"
    );

    for (name, policy) in [
        ("KEEP_ALL", ExifPolicy::KEEP_ALL),
        ("ATTRIBUTED_ORIENTATION", ExifPolicy::ATTRIBUTED_ORIENTATION),
        ("ORIENTATION_ONLY", ExifPolicy::ORIENTATION_ONLY),
    ] {
        let out = retain(&src, &policy).unwrap_or_else(|| panic!("{name}: EXIF dropped entirely"));
        assert_eq!(
            oracle_colour(&out),
            (Some(0xFFFF), Some("R03".into())),
            "{name}: colour declaration lost"
        );
        // Stable under a second pass (serializer fixpoint).
        let again = retain(&out, &policy).expect("second pass");
        assert_eq!(again.as_ref(), out.as_ref(), "{name}: not a fixpoint");
    }
    assert!(retain(&src, &ExifPolicy::DISCARD_ALL).is_none());
}
