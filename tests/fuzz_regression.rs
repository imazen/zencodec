//! Fuzz crash regression suite.
//!
//! Runs every file in `fuzz/regression/` through the same logic as the
//! `exif_parse`, `exif_roundtrip`, `exif_filter`, and `exif_author` fuzz
//! targets, but as a regular `cargo test` — no nightly toolchain needed. Each
//! seed is a previously-found crash that has been fixed; a failure here is a
//! regression.
//!
//! To add a seed: drop the (minimized) crash file into `fuzz/regression/` with
//! a `crash-<sha>` name. The working corpus and unminimized artifacts live in
//! block storage (`/mnt/v/fuzzes/zencodec/`), not git.

use std::fs;
use std::path::PathBuf;

use zencodec::Orientation;
use zencodec::exif::{Exif, ExifPolicy, Retention, TextEncoding};

fn regression_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fuzz/regression")
}

fn ret(bit: bool) -> Retention {
    if bit {
        Retention::Keep
    } else {
        Retention::Discard
    }
}

/// Mirror of `exif_parse`: parse + accessors never panic.
fn run_parse(data: &[u8]) {
    if let Some(x) = Exif::parse(data) {
        let _ = x.orientation();
        let _ = x.copyright();
        let _ = x.artist();
        let _ = x.has_gps();
        let _ = x.has_thumbnail();
    }
}

/// Mirror of `exif_roundtrip`: serializer output re-parses, accessors preserved.
fn run_roundtrip(data: &[u8]) {
    if let Some(x) = Exif::parse(data) {
        let bytes = x.to_bytes();
        let y = Exif::parse(&bytes).expect("serializer output must re-parse");
        assert_eq!(x.orientation(), y.orientation());
        assert_eq!(x.copyright(), y.copyright());
        assert_eq!(x.artist(), y.artist());
        assert_eq!(x.has_gps(), y.has_gps());
        assert_eq!(x.has_thumbnail(), y.has_thumbnail());
    }
}

/// Mirror of `exif_filter`: prune + serialize + retain never panic.
fn run_filter(data: &[u8]) {
    let (cfg, rest) = match data.split_first() {
        Some((c, r)) => (*c, r),
        None => return,
    };
    let policy = ExifPolicy::DISCARD_ALL
        .with_orientation(ret(cfg & 0x01 != 0))
        .with_rights(ret(cfg & 0x02 != 0))
        .with_thumbnail(ret(cfg & 0x04 != 0))
        .with_gps(ret(cfg & 0x08 != 0))
        .with_datetimes(ret(cfg & 0x10 != 0))
        .with_camera(ret(cfg & 0x20 != 0))
        .with_other(ret(cfg & 0x40 != 0));
    if let Some(x) = Exif::parse(rest) {
        let bytes = x.filtered(&policy).to_bytes();
        // Canonical / idempotent: re-filtering the output is a byte-exact
        // fixpoint (regression for a fuzz-found non-idempotence).
        if let Some(y) = Exif::parse(&bytes) {
            assert_eq!(
                bytes,
                y.filtered(&policy).to_bytes(),
                "filter not idempotent"
            );
        }
    }
    let _ = zencodec::exif::retain(rest, &policy);
}

/// Mirror of `exif_author`: the write API (`Exif::new` / edit-after-parse +
/// `set_orientation` / `set_copyright` / `set_artist`) always produces a
/// parseable, value-faithful, canonical TIFF. Input layout matches the fuzz
/// target: `[cfg, o, len_c, c…, len_a, a…, rest…]`.
fn run_author(data: &[u8]) {
    fn take_string(data: &mut &[u8]) -> String {
        let Some((&len, rest)) = data.split_first() else {
            return String::new();
        };
        let len = (len as usize).min(rest.len());
        let (s, tail) = rest.split_at(len);
        *data = tail;
        String::from_utf8_lossy(s).into_owned()
    }
    fn expected_read(s: &str) -> Option<&str> {
        let head = s.split('\0').next().unwrap_or("");
        if head.is_empty() { None } else { Some(head) }
    }

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
        assert_eq!(
            y.orientation(),
            Some(orientation),
            "orientation not faithful"
        );
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
    assert_eq!(
        out,
        y.to_bytes(),
        "authored output not a serializer fixpoint"
    );
}

#[test]
fn fuzz_regression_seeds() {
    let dir = regression_dir();
    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return, // no seeds yet — nothing to regress
    };
    let mut count = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        // Skip dotfiles (e.g. .gitkeep).
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with('.'))
        {
            continue;
        }
        let data = fs::read(&path).expect("read seed");
        run_parse(&data);
        run_roundtrip(&data);
        run_filter(&data);
        run_author(&data);
        count += 1;
    }
    eprintln!("fuzz_regression: replayed {count} seed(s)");
}
