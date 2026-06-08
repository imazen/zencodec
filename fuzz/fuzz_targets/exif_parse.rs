#![no_main]
//! `Exif::parse` and every accessor must not panic on arbitrary input.
use libfuzzer_sys::fuzz_target;
use zencodec::exif::Exif;

fuzz_target!(|data: &[u8]| {
    if let Some(x) = Exif::parse(data) {
        let _ = x.orientation();
        let _ = x.copyright();
        let _ = x.artist();
        let _ = x.has_gps();
        let _ = x.has_thumbnail();
    }
});
