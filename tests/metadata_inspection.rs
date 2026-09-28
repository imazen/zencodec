#![cfg(feature = "xmp")]
use zencodec::{
    exif::{Exif, TextEncoding},
    metadata_audit::{Change, Finding, Report},
    xmp::Packet,
};
fn packet(body: &str) -> String {
    format!(
        r#"<r:RDF xmlns:r="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><r:Description xmlns:g="urn:test">{body}</r:Description></r:RDF>"#
    )
}
#[test]
fn namespace_aliases_entities_arrays_and_unknown_fields() {
    let xml = packet(
        "<g:gain><r:Seq><r:li>1</r:li><r:li>2</r:li><r:li>3</r:li></r:Seq></g:gain><g:secret>A&amp;B</g:secret>",
    );
    let p = Packet::parse(&xml).unwrap();
    assert_eq!(
        p.property("urn:test", "gain").unwrap().unwrap(),
        ["1", "2", "3"]
    );
    assert_eq!(p.property("urn:test", "secret").unwrap().unwrap(), ["A&B"]);
    let alias = xml
        .replace("g:", "other:")
        .replace("xmlns:g=", "xmlns:other=");
    assert!(Report::xmp(&xml).diff(&Report::xmp(&alias)).is_empty());
    let changed = xml.replace("A&amp;B", "private");
    assert!(matches!(
        Report::xmp(&xml).diff(&Report::xmp(&changed)).as_slice(),
        [Change::Modified { .. }]
    ));
}
#[test]
fn duplicates_spoofing_and_qualifiers_are_not_silently_selected() {
    let xml = packet("<g:gain>1</g:gain><g:gain>2</g:gain>");
    assert!(
        Packet::parse(&xml)
            .unwrap()
            .property("urn:test", "gain")
            .is_err()
    );
    assert_eq!(
        Report::xmp(&xml)
            .entries()
            .iter()
            .filter(|e| e.value() == "1" || e.value() == "2")
            .count(),
        2
    );
    let xml = packet("<!-- g:gain=\"999\" --><g:gain xml:lang='en'>1</g:gain>");
    assert!(
        Packet::parse(&xml)
            .unwrap()
            .property("urn:test", "gain")
            .is_err()
    );
    assert!(
        Packet::parse(&xml)
            .unwrap()
            .property("urn:wrong", "gain")
            .unwrap()
            .is_none()
    );
}
#[test]
fn rejects_dtd_malformed_and_deep_xml() {
    for xml in [
        "<!DOCTYPE x [<!ENTITY x 'secret'>]><x>&x;</x>".into(),
        "<x>".into(),
        format!("{}{}", "<x>".repeat(66), "</x>".repeat(66)),
    ] {
        assert!(Packet::parse(&xml).is_err());
    }
}
#[test]
fn exif_typed_inspection_diff_and_coverage() {
    let mut exif = Exif::new(TextEncoding::Ascii);
    exif.set_artist("Public artist");
    let before = Report::exif(&exif.to_bytes());
    exif.set_artist("Another artist");
    let after = Report::exif(&exif.to_bytes());
    assert!(matches!(
        before.diff(&after).as_slice(),
        [Change::Modified { .. }]
    ));
    assert!(before.findings().contains(&Finding::ExifCoverageLimited));
    assert_eq!(Report::exif(b"bad").findings(), &[Finding::InvalidExif]);
}

#[test]
fn resource_shape_and_array_qualifiers_refuse_ambiguous_rendering() {
    let xml = packet(
        "<g:Directory><r:Seq><r:li r:parseType='Resource'><g:Item g:Length='12'/></r:li></r:Seq></g:Directory>",
    );
    let p = Packet::parse(&xml).unwrap();
    assert_eq!(
        p.resource_sequence("urn:test", "Directory", "urn:test", "Item")
            .unwrap()[0][0]
            .value(),
        "12"
    );
    for bad in [
        xml.replace("g:Item", "g:Wrong"),
        xml.replace("g:Length='12'", "g:Length='12' r:resource='private'"),
        xml.replace("<r:Seq>", "<r:Seq r:about='other'>"),
    ] {
        assert!(
            Packet::parse(&bad)
                .unwrap()
                .resource_sequence("urn:test", "Directory", "urn:test", "Item")
                .is_err()
        );
        assert!(!Report::xmp(&bad).entries().is_empty());
    }
    let qualified = packet("<g:gain><r:Seq r:about='other'><r:li>1</r:li></r:Seq></g:gain>");
    assert!(
        Packet::parse(&qualified)
            .unwrap()
            .property("urn:test", "gain")
            .is_err()
    );
}
