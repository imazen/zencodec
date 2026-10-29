//! Native XML/RDF parsing with a deliberately small output schema. These are
//! carrier controls, not implementations of five complete transcode pipelines.
#![forbid(unsafe_code)]
#[path = "../src/policy/mod.rs"]
mod policy;
#[path = "../src/policy_disclosure.rs"]
mod policy_disclosure;
use core::convert::Infallible;
use policy::*;
use policy_disclosure::*;
use zencodec::{At, NoPulse};

const STRICT: Privacy = Privacy::OrientationOnlyV1;
const ROUTES: [TranscodeRoute; 5] = [
    TranscodeRoute::Copy,
    TranscodeRoute::Rewrite,
    TranscodeRoute::Coefficients,
    TranscodeRoute::Pixels,
    TranscodeRoute::Media,
];
fn engine() -> Engine {
    Engine::new(PolicySet::default(), 32)
}
fn first(question: Question<'_>) -> Result<Decision, At<Infallible>> {
    Ok(Decision::choose(
        question.choices().unwrap().first().unwrap(),
    ))
}
fn xml(attributes: &str, content: &str) -> Vec<u8> {
    format!(
        "<rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><rdf:Description rdf:about=\"\" xmlns:t=\"http://ns.adobe.com/tiff/1.0/\" xmlns:private=\"urn:private\" {attributes}>{content}</rdf:Description></rdf:RDF>"
    )
    .into_bytes()
}
fn prepare(bytes: &[u8], obligation: Obligation, route: TranscodeRoute) -> Prepared<'_> {
    Prepared::xmp(bytes, true, 8192, obligation, STRICT, route)
}

#[test]
fn parsed_xml_is_not_privacy_evidence_on_any_transcode_route() {
    let private = xml("private:contact=\"fixture@example.test\"", "");
    assert!(zenmeta::xmp::Packet::parse(core::str::from_utf8(&private).unwrap()).is_ok());
    for route in ROUTES {
        let packet = prepare(&private, Obligation::OptionalRemovable, route);
        assert_eq!(packet.inspection(), Inspection::SyntaxOnly);
        let approved = packet
            .resolve(
                &mut engine(),
                Context::default(),
                &mut first,
                Some(&NoPulse),
            )
            .unwrap();
        assert_eq!(approved.action, BoundaryAction::Drop);
        assert_eq!(approved.route, route);
        assert!(approved.bytes().is_empty());
        let failure = prepare(&private, Obligation::ExactBytes, route)
            .resolve(
                &mut engine(),
                Context::default(),
                &mut first,
                Some(&NoPulse),
            )
            .err()
            .unwrap();
        assert_eq!(failure.error().code(), Code::NoAlternative);
    }
}

#[test]
fn regenerate_known_orientation_removes_unknown_data_without_changing_the_value() {
    for value in 1..=8 {
        let bytes = xml(
            &format!("t:Orientation=\"{value}\" private:contact=\"fixture@example.test\""),
            "<!-- private comment --><private:Blob>opaque payload</private:Blob>",
        );
        let approved = prepare(&bytes, Obligation::Orientation, TranscodeRoute::Rewrite)
            .resolve(
                &mut engine(),
                Context::default(),
                &mut first,
                Some(&NoPulse),
            )
            .unwrap();
        assert_eq!(approved.action, BoundaryAction::Regenerate);
        // Exact output schema, plus semantic read-back with the existing reader.
        // This is not an independent XML parser oracle.
        assert_eq!(approved.bytes(), orientation_packet(value));
        let packet =
            zenmeta::xmp::Packet::parse(core::str::from_utf8(approved.bytes()).unwrap()).unwrap();
        assert_eq!(
            packet
                .property("http://ns.adobe.com/tiff/1.0/", "Orientation")
                .unwrap(),
            Some(vec![value.to_string()])
        );
        let text = core::str::from_utf8(approved.bytes()).unwrap();
        assert!(!text.contains("private"));
        assert!(!text.contains("fixture@example.test"));
        // Same schema can now retain exactly; extra unknown attributes could not.
        let exact = prepare(
            approved.bytes(),
            Obligation::ExactBytes,
            TranscodeRoute::Copy,
        )
        .resolve(
            &mut engine(),
            Context::default(),
            &mut first,
            Some(&NoPulse),
        )
        .unwrap();
        assert_eq!(exact.action, BoundaryAction::Keep);
        assert_eq!(exact.bytes(), approved.bytes());
    }
}

#[test]
fn missing_service_malformed_xml_dtd_and_limits_drop_or_refuse_never_keep() {
    let malformed = b"<broken".to_vec();
    let dtd = b"<!DOCTYPE x [<!ENTITY secret SYSTEM 'file:///not-read'>]><x>&secret;</x>".to_vec();
    let deep = format!("{}{}", "<a>".repeat(65), "</a>".repeat(65)).into_bytes();
    let canonical = orientation_packet(6);
    let cases = [
        (canonical.clone(), false, 8192, Inspection::Unavailable),
        (malformed, true, 8192, Inspection::Malformed),
        (dtd, true, 8192, Inspection::Malformed),
        (vec![0xff], true, 8192, Inspection::Malformed),
        (deep, true, 8192, Inspection::Limit),
        (canonical, true, 16, Inspection::Limit),
    ];
    for (bytes, available, cap, inspection) in cases {
        for obligation in [
            Obligation::OptionalRemovable,
            Obligation::Orientation,
            Obligation::ExactBytes,
        ] {
            let prepared = Prepared::xmp(
                &bytes,
                available,
                cap,
                obligation,
                STRICT,
                TranscodeRoute::Pixels,
            );
            assert_eq!(prepared.inspection(), inspection);
            let result = prepared.resolve(
                &mut engine(),
                Context::default(),
                &mut first,
                Some(&NoPulse),
            );
            if obligation == Obligation::OptionalRemovable {
                let approved = result.ok().unwrap();
                assert_eq!(approved.action, BoundaryAction::Drop);
                assert!(approved.bytes().is_empty());
            } else {
                assert_eq!(result.err().unwrap().error().code(), Code::NoAlternative);
            }
        }
    }
}

#[test]
fn unsupported_rdf_and_numeric_values_cannot_authorize_a_rendering_subset() {
    let cases = [
        xml("t:Orientation=\"6\"", "<t:Orientation>3</t:Orientation>"),
        xml("t:Orientation=\"0\"", ""),
        xml("t:Orientation=\"9\"", ""),
        xml("t:Orientation=\"NaN\"", ""),
        xml("t:Orientation=\"fixture@example.test\"", ""),
        xml("", "<t:Orientation rdf:resource=\"urn:private\"/>"),
        xml(
            "",
            "<t:Orientation><rdf:Seq><rdf:li>6</rdf:li><rdf:li>3</rdf:li></rdf:Seq></t:Orientation>",
        ),
        xml("private:Orientation=\"6\"", ""),
    ];
    for bytes in cases {
        let packet = prepare(&bytes, Obligation::Orientation, TranscodeRoute::Rewrite);
        assert_eq!(packet.inspection(), Inspection::SyntaxOnly);
        let failure = packet
            .resolve(
                &mut engine(),
                Context::default(),
                &mut first,
                Some(&NoPulse),
            )
            .err()
            .unwrap();
        assert_eq!(failure.error().code(), Code::NoAlternative);
    }
}

#[test]
fn opaque_icc_reconstruction_and_nested_required_payloads_are_conflicts() {
    for carrier in [
        Carrier::Icc,
        Carrier::Reconstruction,
        Carrier::NestedObject,
        Carrier::UnknownCritical,
    ] {
        for route in ROUTES {
            let error = Prepared::opaque(
                b"private",
                carrier,
                Obligation::UnknownCritical,
                STRICT,
                route,
            )
            .resolve(
                &mut engine(),
                Context::default(),
                &mut first,
                Some(&NoPulse),
            )
            .err()
            .unwrap();
            assert_eq!(error.error().code(), Code::NoAlternative);
            let facts = error.error().details().unwrap().facts;
            assert!(matches!(
                facts,
                Facts::Boundary(BoundaryFacts {
                    trigger: BoundaryTrigger::Disclosure {
                        required: true,
                        inspection: Inspection::Unavailable,
                        ..
                    },
                    ..
                })
            ));
        }
    }
}

#[test]
fn an_explicit_weaker_contract_is_reported_and_cannot_be_mistaken_for_strict() {
    let approved = Prepared::opaque(
        b"private",
        Carrier::Xmp,
        Obligation::ExactBytes,
        Privacy::PreserveUninspected,
        TranscodeRoute::Copy,
    )
    .resolve(
        &mut engine(),
        Context::default(),
        &mut first,
        Some(&NoPulse),
    )
    .unwrap();
    assert_eq!(approved.privacy, Privacy::PreserveUninspected);
    assert_eq!(approved.inspection, Inspection::Unavailable);
    assert_eq!(approved.bytes(), b"private");
}

#[test]
fn outgoing_bytes_are_bound_to_the_inspected_packet_not_a_separate_boolean() {
    let canonical = orientation_packet(6);
    let private = xml(
        "t:Orientation=\"6\" private:contact=\"fixture@example.test\"",
        "",
    );
    let approved = prepare(&canonical, Obligation::ExactBytes, TranscodeRoute::Rewrite)
        .resolve(
            &mut engine(),
            Context::default(),
            &mut first,
            Some(&NoPulse),
        )
        .unwrap();
    // Passing a different packet requires a new check; copying a previous
    // receipt cannot substitute different bytes into Approved's private body.
    assert_eq!(approved.bytes(), canonical);
    assert!(
        prepare(&private, Obligation::ExactBytes, TranscodeRoute::Rewrite)
            .resolve(
                &mut engine(),
                Context::default(),
                &mut first,
                Some(&NoPulse)
            )
            .is_err()
    );
}
