//! Fuzz crash regression suite.
//!
//! Runs every file in `fuzz/regression/` through the same logic as the
//! `exif_parse`, `exif_roundtrip`, and `exif_filter` fuzz targets, but as a
//! regular `cargo test` — no nightly toolchain needed. Each seed is a
//! previously-found crash that has been fixed; a failure here is a regression.
//!
//! To add a seed: drop the (minimized) crash file into `fuzz/regression/` with
//! a `crash-<sha>` name. The working corpus and unminimized artifacts live in
//! block storage (`/mnt/v/fuzzes/zencodec/`), not git.

use std::fs;
use std::path::PathBuf;

use zencodec::exif::{Exif, ExifPolicy, Retention};

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
        count += 1;
    }
    eprintln!("fuzz_regression: replayed {count} seed(s)");
}
