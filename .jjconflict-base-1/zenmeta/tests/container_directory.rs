#![cfg(feature = "xmp")]

use zenmeta::xmp::container::Directory;

fn packet(items: &str) -> String {
    format!(
        r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description xmlns:Container="http://ns.google.com/photos/1.0/container/" xmlns:Item="http://ns.google.com/photos/1.0/container/item/"><Container:Directory><rdf:Seq>{items}</rdf:Seq></Container:Directory></rdf:Description></rdf:RDF>"#
    )
}
fn item(fields: &str) -> String {
    format!("<rdf:li rdf:parseType='Resource'><Container:Item {fields}/></rdf:li>")
}
fn primary() -> String {
    item("Item:Semantic='Primary' Item:Mime='image/jpeg'")
}

// Recovered lossless-rewrite tests: semantic matching is independent of MPF
// payload order, and an unlisted thumbnail never takes the gain map's length.
#[test]
fn rewrite_item_lengths_matches_items_by_semantic() {
    let xml = zenmeta::xmp::gainmap::generate_primary_xmp(10_000);
    let directory = Directory::parse(&xml).unwrap();
    let out = directory.rewrite_item_lengths(&[("GainMap", 123)]).unwrap();
    if let Ok(directory) = std::env::var("ZENMETA_DIRECTORY_OUTPUT") {
        let directory = std::path::Path::new(&directory);
        std::fs::create_dir_all(directory).unwrap();
        std::fs::write(directory.join("rewritten.xmp"), &out).unwrap();
    }
    assert!(out.contains("Item:Length=\"123\""));
    assert!(!out.contains("Item:Length=\"10000\""));
    assert_eq!(
        Directory::parse(&out)
            .unwrap()
            .items()
            .filter_map(|i| i.length)
            .collect::<Vec<_>>(),
        [123]
    );
    let out = directory
        .rewrite_item_lengths(&[("", 7), ("GainMap", 8)])
        .unwrap();
    assert!(out.contains("Item:Length=\"8\""));
    assert!(!out.contains("Item:Length=\"7\""));
    assert!(directory.rewrite_item_lengths(&[]).is_err());
    assert!(directory.rewrite_item_lengths(&[("DepthMap", 5)]).is_err());
    assert!(
        directory
            .rewrite_item_lengths(&[("GainMap", 5), ("GainMap", 6)])
            .is_err()
    );
    assert_eq!(
        Directory::parse("<x/>")
            .unwrap()
            .rewrite_item_lengths(&[("GainMap", 1)])
            .unwrap(),
        "<x/>"
    );
}

#[test]
fn rewrite_item_lengths_reads_element_and_single_quote_forms() {
    let xml = packet(&format!(
        "{}<rdf:li rdf:parseType='Resource'><Container:Item>\
        <Item:Semantic>GainMap</Item:Semantic><Item:Mime>image/jpeg</Item:Mime>\
        <Item:Length>4000</Item:Length><Item:Padding>16</Item:Padding>\
        </Container:Item></rdf:li>{}",
        primary(),
        item("Item:Semantic='DepthMap' Item:Mime='image/jpeg' Item:Length='900'")
    ));
    let directory = Directory::parse(&xml).unwrap();
    assert_eq!(
        directory
            .items()
            .map(|i| i.semantic.as_xmp_str())
            .collect::<Vec<_>>(),
        ["Primary", "GainMap", "DepthMap"]
    );
    let out = directory
        .rewrite_item_lengths(&[("DepthMap", 11), ("GainMap", 22)])
        .unwrap();
    assert!(out.contains("<Item:Length>22</Item:Length>"));
    assert!(out.contains("<Item:Padding>0</Item:Padding>"));
    assert!(out.contains("Item:Length='11'"));
    assert_eq!(
        out,
        xml.replace(">4000<", ">22<")
            .replace(">16<", ">0<")
            .replace("Length='900'", "Length='11'")
    );
}

#[test]
fn directory_field_names_inside_values_are_text() {
    let xml = packet(&format!(
        "{}{}",
        primary(),
        item(r#"Item:Semantic="GainMap" Item:Mime="image/jpeg" Item:Length="Item:Padding='5'""#)
    ));
    // The recovered permissive scanner accepted this malformed numeric value.
    assert!(Directory::parse(&xml).is_err());
    let xml = xml.replace(
        "Item:Length=\"Item:Padding='5'\"",
        "Item:Length=\"9\" Item:VendorNote=\"Item:Padding='5'\"",
    );
    let out = Directory::parse(&xml)
        .unwrap()
        .rewrite_item_lengths(&[("GainMap", 42)])
        .unwrap();
    assert_eq!(out, xml.replace("Length=\"9\"", "Length=\"42\""));
}

#[test]
fn aliases_entities_long_attribute_spacing_and_cdata_preserve_boundaries() {
    let xml = packet(&format!(
        "{}{}",
        primary(),
        item("Item:Semantic='Gain&#77;ap' Item:Mime='image/jpeg' Item:Length='9' Item:Padding='3'")
    ))
    .replace("Container:", "c:")
    .replace("xmlns:Container", "xmlns:c")
    .replace("Item:", "i:")
    .replace("xmlns:Item", "xmlns:i")
    .replace("Length=", &format!("Length{}=", " ".repeat(1024)));
    let out = Directory::parse(&xml)
        .unwrap()
        .rewrite_item_lengths(&[("GainMap", 222)])
        .unwrap();
    assert_eq!(
        out,
        xml.replace("='9'", "='222'")
            .replace("Padding='3'", "Padding='0'")
    );
    let xml = packet(&format!(
        "{}<rdf:li><Container:Item Item:Semantic='GainMap' Item:Mime='image/jpeg'><Item:Length><![CDATA[9]]></Item:Length></Container:Item></rdf:li>",
        primary()
    ));
    assert_eq!(
        Directory::parse(&xml)
            .unwrap()
            .rewrite_item_lengths(&[("GainMap", 22)])
            .unwrap(),
        xml.replace("<![CDATA[9]]>", "22")
    );
}

#[test]
fn ambiguous_incomplete_and_misplaced_directories_fail() {
    let secondary = item("Item:Semantic='GainMap' Item:Mime='image/jpeg' Item:Length='9'");
    for xml in [
        packet(""),
        packet(&secondary),
        packet(&format!("{}{}{}", primary(), secondary, secondary)),
        packet(&format!(
            "{}{}",
            primary(),
            item("Item:Semantic='GainMap' Item:Mime='image/jpeg'")
        )),
        packet(&format!(
            "{}{}",
            primary(),
            item("Item:Semantic='GainMap' Item:Mime='image/jpeg' Item:Length='+9'")
        )),
        packet(&format!(
            "{}<rdf:li><Container:Item Item:Semantic='GainMap' Item:Mime='image/jpeg' Item:Length='9'><Item:Length>10</Item:Length></Container:Item></rdf:li>",
            primary()
        )),
        "<Container:Directory/>".into(),
        "<x xmlns:c='http://ns.google.com/photos/1.0/container/'><c:Directory/></x>".into(),
    ] {
        assert!(Directory::parse(&xml).is_err(), "{xml}");
    }
}

// Keep the recovered 4 MiB malformed inputs and two-second time bound.
#[test]
fn directory_scan_is_linear_and_capped() {
    let head = packet(&primary());
    let head = head.split("</rdf:Seq>").next().unwrap();
    let n = 4 << 20;
    let cases = [
        format!("{head}{}", "<rdf:li".repeat(n / 7)),
        format!("{head}<rdf:li Item:Length='{}", "<rdf:li".repeat(n / 7)),
        format!("{head}<rdf:li>{}", "<x".repeat(n / 2)),
        format!("{head}<rdf:li><Item:Length>{}", "1".repeat(n)),
    ];
    let start = std::time::Instant::now();
    for xml in &cases {
        assert!(Directory::parse(xml).is_err());
    }
    assert!(start.elapsed() < std::time::Duration::from_secs(2));
    let mut items = primary();
    for i in 0..256 {
        items.push_str(&item(&format!(
            "Item:Semantic='Other{i}' Item:Mime='image/jpeg' Item:Length='1'"
        )));
    }
    assert!(matches!(
        Directory::parse(&packet(&items)),
        Err(zenmeta::xmp::Error::Limit)
    ));
}
