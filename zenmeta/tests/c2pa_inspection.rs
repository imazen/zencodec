#![cfg(feature = "c2pa")]
//! A manifest store built here byte by byte: JUMBF superboxes, CBOR claim and
//! assertions, a COSE_Sign1 signature with one X.509 certificate, and a thumbnail.
use zenmeta::audit::{Finding, Report};

// ── CBOR ──────────────────────────────────────────────────────────────────────────
fn head(major: u8, n: u64) -> Vec<u8> {
    let m = major << 5;
    match n {
        0..=23 => vec![m | n as u8],
        24..=0xff => vec![m | 24, n as u8],
        0x100..=0xffff => [vec![m | 25], (n as u16).to_be_bytes().to_vec()].concat(),
        0x1_0000..=0xffff_ffff => [vec![m | 26], (n as u32).to_be_bytes().to_vec()].concat(),
        _ => [vec![m | 27], n.to_be_bytes().to_vec()].concat(),
    }
}
fn uint(n: u64) -> Vec<u8> {
    head(0, n)
}
fn nint(n: i64) -> Vec<u8> {
    head(1, (-1 - n) as u64)
}
fn text(s: &str) -> Vec<u8> {
    [head(3, s.len() as u64), s.as_bytes().to_vec()].concat()
}
fn bytes(b: &[u8]) -> Vec<u8> {
    [head(2, b.len() as u64), b.to_vec()].concat()
}
fn array(items: &[Vec<u8>]) -> Vec<u8> {
    [head(4, items.len() as u64), items.concat()].concat()
}
fn map(pairs: &[(Vec<u8>, Vec<u8>)]) -> Vec<u8> {
    let mut out = head(5, pairs.len() as u64);
    for (k, v) in pairs {
        out.extend_from_slice(k);
        out.extend_from_slice(v);
    }
    out
}
fn tag(t: u64, v: Vec<u8>) -> Vec<u8> {
    [head(6, t), v].concat()
}

// ── JUMBF ─────────────────────────────────────────────────────────────────────────
fn bx(ty: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    [
        ((payload.len() + 8) as u32).to_be_bytes().to_vec(),
        ty.to_vec(),
        payload.to_vec(),
    ]
    .concat()
}
fn c2pa_uuid(t: &[u8; 4]) -> Vec<u8> {
    [
        t.to_vec(),
        vec![
            0x00, 0x11, 0x00, 0x10, 0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71,
        ],
    ]
    .concat()
}
fn superbox(uuid: Vec<u8>, label: &str, content: &[Vec<u8>]) -> Vec<u8> {
    let jumd = [uuid, vec![0x03], label.as_bytes().to_vec(), vec![0]].concat();
    bx(b"jumb", &[bx(b"jumd", &jumd), content.concat()].concat())
}

// ── DER ───────────────────────────────────────────────────────────────────────────
fn der(id: u8, v: &[u8]) -> Vec<u8> {
    let mut out = vec![id];
    if v.len() < 0x80 {
        out.push(v.len() as u8);
    } else {
        out.extend_from_slice(&[0x82, (v.len() >> 8) as u8, v.len() as u8]);
    }
    out.extend_from_slice(v);
    out
}
fn name(attrs: &[(&[u8], &str)]) -> Vec<u8> {
    let rdns: Vec<u8> = attrs
        .iter()
        .flat_map(|(oid, val)| {
            der(
                0x31,
                &der(0x30, &[der(0x06, oid), der(0x0c, val.as_bytes())].concat()),
            )
        })
        .collect();
    der(0x30, &rdns)
}
const CN: &[u8] = &[0x55, 0x04, 0x03];
const O: &[u8] = &[0x55, 0x04, 0x0a];
const EMAIL: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x09, 0x01];
const ECDSA_SHA256: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02];
fn certificate() -> Vec<u8> {
    let alg = der(0x30, &der(0x06, ECDSA_SHA256));
    let san = der(0x30, &der(0x81, b"signer@example.test"));
    let ext = der(
        0x30,
        &[der(0x06, &[0x55, 0x1d, 0x11]), der(0x04, &san)].concat(),
    );
    let tbs = der(
        0x30,
        &[
            der(0xa0, &der(0x02, &[2])),
            der(0x02, &[0x1f, 0x2e]),
            alg.clone(),
            name(&[(CN, "Example CA"), (O, "Example Org")]),
            der(
                0x30,
                &[der(0x17, b"250101000000Z"), der(0x17, b"270101000000Z")].concat(),
            ),
            name(&[(CN, "Example Signer"), (EMAIL, "signer@example.test")]),
            der(
                0x30,
                &[
                    der(
                        0x30,
                        &[der(0x06, &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01])].concat(),
                    ),
                    der(0x03, &[0, 4, 1]),
                ]
                .concat(),
            ),
            der(0xa3, &der(0x30, &ext)),
        ]
        .concat(),
    );
    der(0x30, &[tbs, alg, der(0x03, &[0, 1, 2])].concat())
}

fn store() -> Vec<u8> {
    let actions = map(&[(
        text("actions"),
        array(&[map(&[
            (text("action"), text("c2pa.created")),
            (text("softwareAgent"), text("Example Generator 1.0")),
        ])]),
    )]);
    let hash = map(&[
        (text("alg"), text("sha256")),
        (text("hash"), bytes(&[0xab; 32])),
        (text("pad"), bytes(&[0; 300])),
    ]);
    let thumb = superbox(
        [
            0x40, 0xcb, 0x0c, 0x32, 0xbb, 0x8a, 0x48, 0x9d, 0xa7, 0x0b, 0x2a, 0xd6, 0xf4, 0x7f,
            0x43, 0x69,
        ]
        .to_vec(),
        "c2pa.thumbnail.claim.jpeg",
        &[
            bx(b"bfdb", b"\x00image/jpeg\x00"),
            bx(b"bidb", &[0xff, 0xd8, 0xff, 0xd9]),
        ],
    );
    let assertions = superbox(
        c2pa_uuid(b"c2as"),
        "c2pa.assertions",
        &[
            superbox(
                c2pa_uuid(b"cbor"),
                "c2pa.actions.v2",
                &[bx(b"cbor", &actions)],
            ),
            superbox(c2pa_uuid(b"cbor"), "c2pa.hash.data", &[bx(b"cbor", &hash)]),
            thumb,
        ],
    );
    let claim = map(&[
        (text("instanceID"), text("xmp:iid:0000")),
        (
            text("claim_generator_info"),
            array(&[map(&[(text("name"), text("Example Generator"))])]),
        ),
        (
            text("created_assertions"),
            array(&[map(&[(
                text("url"),
                text("self#jumbf=c2pa.assertions/c2pa.actions.v2"),
            )])]),
        ),
    ]);
    let protected = map(&[(uint(1), nint(-7)), (uint(33), bytes(&certificate()))]);
    let cose = tag(
        18,
        array(&[
            bytes(&protected),
            map(&[(text("pad"), bytes(&[0; 8]))]),
            vec![0xf6],
            bytes(&[7; 64]),
        ]),
    );
    let manifest = superbox(
        c2pa_uuid(b"c2ma"),
        "urn:uuid:00000000-0000-4000-8000-000000000001",
        &[
            assertions,
            superbox(c2pa_uuid(b"c2cl"), "c2pa.claim.v2", &[bx(b"cbor", &claim)]),
            superbox(c2pa_uuid(b"c2cs"), "c2pa.signature", &[bx(b"cbor", &cose)]),
        ],
    );
    superbox(c2pa_uuid(b"c2pa"), "c2pa", &[manifest])
}

fn value<'a>(r: &'a Report, path: &str) -> &'a str {
    r.entries()
        .iter()
        .find(|e| e.path() == path)
        .unwrap_or_else(|| {
            panic!(
                "no entry {path}; have {:#?}",
                r.entries().iter().map(|e| e.path()).collect::<Vec<_>>()
            )
        })
        .value()
}

const M: &str = "c2pa/urn:uuid:00000000-0000-4000-8000-000000000001";

#[test]
fn claims_and_assertions_are_reported_with_opaque_certificates() {
    let r = Report::c2pa(&store());
    assert_eq!(value(&r, "c2pa/@type"), "c2pa");
    assert_eq!(value(&r, &format!("{M}/@type")), "c2ma");
    assert_eq!(
        value(
            &r,
            &format!("{M}/c2pa.assertions/c2pa.actions.v2/cbor/actions[0]/action")
        ),
        "\"c2pa.created\""
    );
    assert_eq!(
        value(
            &r,
            &format!("{M}/c2pa.assertions/c2pa.actions.v2/cbor/actions[0]/softwareAgent")
        ),
        "\"Example Generator 1.0\""
    );
    assert_eq!(
        value(&r, &format!("{M}/c2pa.assertions/c2pa.hash.data/cbor/pad")),
        "bytes(len=300, all zero)"
    );
    assert_eq!(
        value(
            &r,
            &format!("{M}/c2pa.claim.v2/cbor/claim_generator_info[0]/name")
        ),
        "\"Example Generator\""
    );
    let sig = format!("{M}/c2pa.signature/cose");
    assert_eq!(value(&r, &format!("{sig}/protected/alg")), "-7 (ES256)");
    assert_eq!(
        value(&r, &format!("{sig}/protected/x5chain[0]")),
        format!("bytes(len={})", certificate().len())
    );
    assert!(
        !r.entries()
            .iter()
            .any(|e| e.value().contains("Example Signer"))
    );
    assert_eq!(value(&r, &format!("{sig}/payload")), "detached");
    assert_eq!(value(&r, &format!("{sig}/signature")), "bytes(len=64)");
    // Both the thumbnail and certificate have explicit coverage findings.
    assert_eq!(
        r.findings(),
        &[
            Finding::Opaque {
                path: format!("{M}/c2pa.assertions/c2pa.thumbnail.claim.jpeg/bidb"),
                bytes: 4
            },
            Finding::Opaque {
                path: format!("{sig}/protected/x5chain[0]"),
                bytes: certificate().len()
            }
        ]
    );
    // Paths are unique, so diffs pair entries one to one.
    let mut paths: Vec<_> = r.entries().iter().map(|e| e.path()).collect();
    paths.dedup();
    assert_eq!(paths.len(), r.entries().len());
}

#[test]
fn opaque_certificate_contents_require_original_bytes_for_comparison() {
    let a = store();
    let b = {
        let s = String::from_utf8_lossy(&a).into_owned();
        assert!(s.contains("Example Signer"));
        let mut b = a.clone();
        let at = a.windows(14).position(|w| w == b"Example Signer").unwrap();
        b[at..at + 14].copy_from_slice(b"Another Signer");
        b
    };
    let (ra, rb) = (Report::c2pa(&a), Report::c2pa(&b));
    let changes = ra.diff(&rb);
    assert_ne!(a, b);
    assert!(
        changes.is_empty(),
        "opaque content is deliberately not interpreted"
    );
    assert!(ra.findings().iter().any(|f| matches!(f,
        Finding::Opaque { path, bytes } if path.ends_with("/x5chain[0]") && *bytes == certificate().len()
    )));
    assert_eq!(ra.findings(), rb.findings());
}

#[test]
fn malformed_and_hostile_stores_are_findings_not_panics() {
    // Not a manifest store.
    let other = superbox(c2pa_uuid(b"c2ma"), "x", &[]);
    assert!(matches!(
        Report::c2pa(&other).findings(),
        [Finding::InvalidJumbf(_)]
    ));
    // Truncated.
    let s = store();
    assert!(matches!(
        Report::c2pa(&s[..s.len() - 1]).findings(),
        [Finding::InvalidJumbf(_)]
    ));
    // Superboxes nested past the limit.
    let mut deep = superbox(c2pa_uuid(b"cbor"), "leaf", &[]);
    for _ in 0..40 {
        deep = superbox(c2pa_uuid(b"c2as"), "n", &[deep]);
    }
    let deep = superbox(c2pa_uuid(b"c2pa"), "c2pa", &[deep]);
    assert!(
        Report::c2pa(&deep)
            .findings()
            .iter()
            .any(|f| matches!(f, Finding::InvalidJumbf(m) if m.contains("deeply")))
    );
    // Embedded CBOR (tag 24) nested 10,000 times in an assertion: bounded, no stack overflow.
    let mut nested = uint(1);
    for _ in 0..10_000 {
        nested = tag(24, bytes(&nested));
    }
    let a = superbox(c2pa_uuid(b"cbor"), "x", &[bx(b"cbor", &nested)]);
    let s = superbox(
        c2pa_uuid(b"c2pa"),
        "c2pa",
        &[superbox(c2pa_uuid(b"c2ma"), "m", &[a])],
    );
    assert!(
        Report::c2pa(&s)
            .findings()
            .iter()
            .any(|f| matches!(f, Finding::InvalidCbor { .. }))
    );
    // Garbage certificates are opaque, not errors.
    let cose = tag(
        18,
        array(&[
            bytes(&map(&[(uint(33), bytes(&[0x30, 0x03, 1, 2, 3]))])),
            map(&[]),
            vec![0xf6],
            bytes(&[1]),
        ]),
    );
    let s = superbox(
        c2pa_uuid(b"c2pa"),
        "c2pa",
        &[superbox(
            c2pa_uuid(b"c2ma"),
            "m",
            &[superbox(
                c2pa_uuid(b"c2cs"),
                "c2pa.signature",
                &[bx(b"cbor", &cose)],
            )],
        )],
    );
    assert!(
        Report::c2pa(&s)
            .findings()
            .iter()
            .any(|f| matches!(f, Finding::Opaque { path, .. } if path.ends_with("x5chain[0]")))
    );
    assert_eq!(
        Report::c2pa(&[]).findings(),
        &[Finding::InvalidJumbf(
            "c2pa: a manifest store is exactly one jumb superbox".into()
        )]
    );
}

fn assertion_store(value: &[u8]) -> Vec<u8> {
    superbox(c2pa_uuid(b"c2pa"), "c2pa", &[bx(b"cbor", value)])
}

#[test]
fn duplicate_and_typed_keys_remain_distinct_in_diffs() {
    let fields = |duplicate| {
        map(&[
            (text("foo"), uint(1)),
            (text("foo"), uint(duplicate)),
            (text("foo#2"), uint(3)),
            (text("1"), uint(4)),
            (uint(1), uint(5)),
            (text("a[0]"), uint(6)),
            (text("a"), array(&[uint(7)])),
            (text("@uint(1)"), uint(8)),
            (uint(u64::MAX), uint(9)),
            (uint(u64::MAX - 1), uint(10)),
            (text("Null"), uint(11)),
            (vec![0xf6], uint(12)),
            (array(&[uint(1)]), uint(13)),
        ])
    };
    let before = Report::c2pa(&assertion_store(&fields(2)));
    let after = Report::c2pa(&assertion_store(&fields(22)));
    assert!(before.findings().is_empty(), "{:?}", before.findings());
    let paths: Vec<_> = before.entries().iter().map(|entry| entry.path()).collect();
    let mut distinct = paths.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(distinct.len(), paths.len(), "{paths:#?}");
    assert_eq!(before.diff(&after).len(), 1);
}

#[test]
fn opaque_findings_stop_at_one_terminal_limit() {
    let children = bx(b"xxxx", &[]).repeat(200_000);
    let input = superbox(c2pa_uuid(b"c2pa"), "c2pa", &[children]);
    let report = Report::c2pa(&input);
    assert_eq!(report.findings().last(), Some(&Finding::Limit));
    assert_eq!(
        report
            .findings()
            .iter()
            .filter(|f| **f == Finding::Limit)
            .count(),
        1
    );
    assert!(report.findings().len() < 200_000);
    let mut paths: Vec<_> = report.entries().iter().map(|entry| entry.path()).collect();
    let count = paths.len();
    paths.sort_unstable();
    paths.dedup();
    assert_eq!(paths.len(), count, "partial reports also need unique paths");
}

#[test]
fn tag24_reparses_share_the_allocation_budget() {
    let mut nested = bytes(&vec![0xab; 1024 * 1024]);
    for _ in 0..24 {
        nested = tag(24, bytes(&nested));
    }
    let report = Report::c2pa(&assertion_store(&nested));
    assert_eq!(report.findings().last(), Some(&Finding::Limit));
    assert_eq!(
        report
            .findings()
            .iter()
            .filter(|f| **f == Finding::Limit)
            .count(),
        1
    );
}

#[test]
fn sibling_labels_cannot_impersonate_occurrence_suffixes() {
    let child = |label: &str, n| superbox(c2pa_uuid(b"cbor"), label, &[bx(b"cbor", &uint(n))]);
    let input = superbox(
        c2pa_uuid(b"c2pa"),
        "c2pa",
        &[
            child("same", 1),
            child("same", 2),
            child("same[1]", 3),
            child("same#2", 4),
        ],
    );
    let report = Report::c2pa(&input);
    assert!(report.findings().is_empty());
    let mut paths: Vec<_> = report.entries().iter().map(|entry| entry.path()).collect();
    let count = paths.len();
    paths.sort_unstable();
    paths.dedup();
    assert_eq!(paths.len(), count);
}

#[test]
fn certificates_and_timestamp_der_are_always_opaque() {
    // A truncated OID and a base-128 arc larger than u64: neither is decoded here.
    for der_bytes in [
        &[0x06, 1, 0x80][..],
        &[
            0x06, 12, 0x2a, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f,
        ][..],
    ] {
        let protected = map(&[(uint(33), array(&[bytes(der_bytes)]))]);
        let token = map(&[
            (text("val"), bytes(der_bytes)),
            (text("extra"), text("retained")),
        ]);
        let timestamp = map(&[
            (text("tstTokens"), array(&[token])),
            (text("other"), uint(7)),
        ]);
        let cose = array(&[
            bytes(&protected),
            map(&[(text("sigTst2"), timestamp)]),
            vec![0xf6],
            bytes(&[1]),
        ]);
        let input = superbox(
            c2pa_uuid(b"c2pa"),
            "c2pa",
            &[superbox(
                c2pa_uuid(b"c2cs"),
                "signature",
                &[bx(b"cbor", &cose)],
            )],
        );
        let report = Report::c2pa(&input);
        assert_eq!(
            report.findings(),
            &[
                Finding::Opaque {
                    path: "c2pa/signature/cose/protected/x5chain[0]".into(),
                    bytes: der_bytes.len()
                },
                Finding::Opaque {
                    path: "c2pa/signature/cose/unprotected/sigTst2/tstTokens[0]/val".into(),
                    bytes: der_bytes.len()
                }
            ]
        );
        assert_eq!(
            value(
                &report,
                "c2pa/signature/cose/unprotected/sigTst2/tstTokens[0]/extra"
            ),
            "\"retained\""
        );
        assert_eq!(
            value(&report, "c2pa/signature/cose/unprotected/sigTst2/other"),
            "7"
        );
    }
}
