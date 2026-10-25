// Supplemental semantic check; not part of the frozen learning score.
#[test]
fn orientation_survives_when_icc_is_absent() {
    for progressive in [false, true] {
        let clean = fixtures::base(progressive);
        let source = [&clean[..2], fixtures::descriptive().as_slice(), &clean[2..]].concat();
        let mut output = Vec::new();
        candidate::rewrite_jpeg(&source, &mut output).unwrap();
        let exif = exif::Reader::new()
            .read_from_container(&mut std::io::Cursor::new(&output))
            .unwrap();
        assert_eq!(
            exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY)
                .unwrap()
                .value
                .get_uint(0),
            Some(6)
        );
        assert!(
            exif.get_field(exif::Tag::Artist, exif::In::PRIMARY)
                .is_none()
        );
    }
}
