#![no_main]
//! Authoring-path fuzz: drive the EXIF *write* API — `Exif::new` /
//! edit-after-parse, `set_orientation` / `set_copyright` / `set_artist` —
//! and require the output to be a parseable, value-faithful, canonical TIFF.
//! (Mirrored as `run_author` in `tests/fuzz_regression.rs`.)
//!
//! Input layout: `[cfg, o, len_c, c…, len_a, a…, rest…]`
//! - `cfg` bit 0: `TextEncoding` for from-scratch authoring (0 = Ascii, 1 = Utf8)
//! - `cfg` bit 1: edit `parse(rest)` instead of starting from `Exif::new`
//! - `cfg` bit 2: `set_orientation((o % 8) + 1)`
//! - `cfg` bit 3: `set_copyright(c)` (lossy-UTF-8 of the `c` bytes)
//! - `cfg` bit 4: `set_artist(a)`
use libfuzzer_sys::fuzz_target;
use zencodec::Orientation;
use zencodec::exif::{Exif, TextEncoding};

/// Split a 1-byte-length-prefixed chunk off `data`, lossy-decoded to UTF-8.
fn take_string(data: &mut &[u8]) -> String {
    let Some((&len, rest)) = data.split_first() else {
        return String::new();
    };
    let len = (len as usize).min(rest.len());
    let (s, tail) = rest.split_at(len);
    *data = tail;
    String::from_utf8_lossy(s).into_owned()
}

/// The string a reader must report after `set_copyright(s)` / `set_artist(s)`:
/// the field is NUL-terminated, so reads stop at the first embedded NUL, and
/// an empty (or NUL-leading) value reads back as absent.
fn expected_read(s: &str) -> Option<&str> {
    let head = s.split('\0').next().unwrap_or("");
    if head.is_empty() { None } else { Some(head) }
}

fuzz_target!(|data: &[u8]| {
    let mut data = data;
    let Some((&cfg, rest)) = data.split_first() else {
        return;
    };
    data = rest;
    let Some((&o, rest)) = data.split_first() else {
        return;
    };
    data = rest;
    let copyright = take_string(&mut data);
    let artist = take_string(&mut data);

    let encoding = if cfg & 0x01 != 0 {
        TextEncoding::Utf8
    } else {
        TextEncoding::Ascii
    };
    let mut x = if cfg & 0x02 != 0 {
        match Exif::parse(data) {
            Some(x) => x,
            None => return,
        }
    } else {
        Exif::new(encoding)
    };

    let orientation = Orientation::from_exif((o % 8) + 1).expect("1-8 is always valid");
    if cfg & 0x04 != 0 {
        x.set_orientation(orientation);
    }
    if cfg & 0x08 != 0 {
        x.set_copyright(&copyright);
    }
    if cfg & 0x10 != 0 {
        x.set_artist(&artist);
    }

    let out = x.to_bytes();
    let y = Exif::parse(&out).expect("authored output must parse");
    if cfg & 0x04 != 0 {
        assert_eq!(y.orientation(), Some(orientation), "orientation not faithful");
    }
    if cfg & 0x08 != 0 {
        assert_eq!(
            y.copyright().as_deref(),
            expected_read(&copyright),
            "copyright not faithful"
        );
    }
    if cfg & 0x10 != 0 {
        assert_eq!(
            y.artist().as_deref(),
            expected_read(&artist),
            "artist not faithful"
        );
    }
    // Authored output is already canonical: re-serializing is a fixpoint.
    assert_eq!(out, y.to_bytes(), "authored output not a serializer fixpoint");
});
