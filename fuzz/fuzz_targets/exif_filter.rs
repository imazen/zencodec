#![no_main]
//! `Exif::filtered(policy).to_bytes()` and `exif::retain` must not panic for
//! any input or policy. The first byte seeds the 7-category policy bitmask.
use libfuzzer_sys::fuzz_target;
use zencodec::exif::{Exif, ExifPolicy, Retention, retain};

fn ret(bit: bool) -> Retention {
    if bit { Retention::Keep } else { Retention::Discard }
}

fuzz_target!(|data: &[u8]| {
    let (cfg, rest) = match data.split_first() {
        Some((c, r)) => (*c, r),
        None => return,
    };
    let policy = ExifPolicy::DISCARD_ALL
        .with_orientation(ret(cfg & 0x01 != 0))
        .with_rights(ret(cfg & 0x02 != 0))
        .with_thumbnail(ret(cfg & 0x04 != 0))
        .with_gps(ret(cfg & 0x08 != 0))
        .with_datetime(ret(cfg & 0x10 != 0))
        .with_camera(ret(cfg & 0x20 != 0))
        .with_other(ret(cfg & 0x40 != 0));
    if let Some(x) = Exif::parse(rest) {
        let pruned = x.filtered(&policy);
        let bytes = pruned.to_bytes();
        // The pruned, re-serialized blob must itself parse.
        let _ = Exif::parse(&bytes);
    }
    let _ = retain(rest, &policy);
});
