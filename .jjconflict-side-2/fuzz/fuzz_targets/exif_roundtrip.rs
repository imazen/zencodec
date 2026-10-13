#![no_main]
//! `parse → to_bytes → parse` must round-trip: the serializer always produces
//! a re-parseable TIFF, and the key accessors are preserved.
use libfuzzer_sys::fuzz_target;
use zencodec::exif::Exif;

fuzz_target!(|data: &[u8]| {
    if let Some(x) = Exif::parse(data) {
        let bytes = x.to_bytes();
        let y = Exif::parse(&bytes).expect("serializer output must re-parse");
        assert_eq!(x.orientation(), y.orientation(), "orientation drift");
        assert_eq!(x.copyright(), y.copyright(), "copyright drift");
        assert_eq!(x.artist(), y.artist(), "artist drift");
        assert_eq!(x.has_gps(), y.has_gps(), "gps presence drift");
        assert_eq!(x.has_thumbnail(), y.has_thumbnail(), "thumbnail presence drift");
    }
});
