//! Minimal DER reader for the X.509 certificates and RFC 3161 timestamp tokens a C2PA
//! claim signature carries. It reads names, serials, validity, algorithms and the
//! subject-alternative-name entries; it does not verify anything.
use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};

#[derive(Clone, Copy, Debug)]
pub(crate) struct Tlv<'a> {
    /// Identifier octet (class, constructed bit and low tag number).
    pub id: u8,
    pub value: &'a [u8],
}

impl Tlv<'_> {
    pub(crate) fn is(&self, id: u8) -> bool {
        self.id == id
    }
}

/// Read one TLV, returning it and the rest. High tag numbers and indefinite lengths
/// (BER only) are rejected.
pub(crate) fn tlv(data: &[u8]) -> Option<(Tlv<'_>, &[u8])> {
    let (&id, rest) = data.split_first()?;
    if id & 0x1f == 0x1f {
        return None;
    }
    let (&l0, mut rest) = rest.split_first()?;
    let len = if l0 < 0x80 {
        usize::from(l0)
    } else {
        let n = usize::from(l0 & 0x7f);
        if n == 0 || n > 4 || rest.len() < n {
            return None;
        }
        let mut len = 0usize;
        for &b in &rest[..n] {
            len = (len << 8) | usize::from(b);
        }
        rest = &rest[n..];
        len
    };
    if rest.len() < len {
        return None;
    }
    Some((
        Tlv {
            id,
            value: &rest[..len],
        },
        &rest[len..],
    ))
}

/// Every TLV in `data`, or `None` if any is malformed.
pub(crate) fn seq(data: &[u8]) -> Option<Vec<Tlv<'_>>> {
    let mut out = Vec::new();
    let mut rest = data;
    while !rest.is_empty() {
        let (t, r) = tlv(rest)?;
        out.push(t);
        rest = r;
    }
    Some(out)
}

pub(crate) fn oid(bytes: &[u8]) -> String {
    let mut parts: Vec<u64> = Vec::new();
    let mut acc = 0u64;
    for &b in bytes {
        acc = (acc << 7) | u64::from(b & 0x7f);
        if b & 0x80 == 0 {
            if parts.is_empty() {
                let first = (acc / 40).min(2);
                parts.push(first);
                parts.push(acc - first * 40);
            } else {
                parts.push(acc);
            }
            acc = 0;
        }
    }
    parts
        .iter()
        .map(|p| p.to_string())
        .collect::<Vec<_>>()
        .join(".")
}

pub(crate) fn oid_name(oid: &str) -> Option<&'static str> {
    Some(match oid {
        "2.5.4.3" => "CN",
        "2.5.4.4" => "SN",
        "2.5.4.5" => "serialNumber",
        "2.5.4.6" => "C",
        "2.5.4.7" => "L",
        "2.5.4.8" => "ST",
        "2.5.4.9" => "street",
        "2.5.4.10" => "O",
        "2.5.4.11" => "OU",
        "2.5.4.12" => "title",
        "2.5.4.42" => "GN",
        "2.5.4.97" => "organizationIdentifier",
        "1.2.840.113549.1.9.1" => "emailAddress",
        "0.9.2342.19200300.100.1.1" => "UID",
        "0.9.2342.19200300.100.1.25" => "DC",
        "1.2.840.10045.2.1" => "ecPublicKey",
        "1.2.840.10045.3.1.7" => "P-256",
        "1.3.132.0.34" => "P-384",
        "1.3.132.0.35" => "P-521",
        "1.2.840.113549.1.1.1" => "rsaEncryption",
        "1.2.840.113549.1.1.10" => "RSASSA-PSS",
        "1.2.840.113549.1.1.11" => "sha256WithRSAEncryption",
        "1.2.840.113549.1.1.12" => "sha384WithRSAEncryption",
        "1.2.840.113549.1.1.13" => "sha512WithRSAEncryption",
        "1.2.840.10045.4.3.2" => "ecdsa-with-SHA256",
        "1.2.840.10045.4.3.3" => "ecdsa-with-SHA384",
        "1.2.840.10045.4.3.4" => "ecdsa-with-SHA512",
        "1.3.101.112" => "Ed25519",
        "2.16.840.1.101.3.4.2.1" => "sha256",
        "2.16.840.1.101.3.4.2.2" => "sha384",
        "2.16.840.1.101.3.4.2.3" => "sha512",
        "2.5.29.14" => "subjectKeyIdentifier",
        "2.5.29.15" => "keyUsage",
        "2.5.29.17" => "subjectAltName",
        "2.5.29.19" => "basicConstraints",
        "2.5.29.31" => "cRLDistributionPoints",
        "2.5.29.32" => "certificatePolicies",
        "2.5.29.35" => "authorityKeyIdentifier",
        "2.5.29.37" => "extKeyUsage",
        "1.3.6.1.5.5.7.1.1" => "authorityInfoAccess",
        _ => return None,
    })
}

fn named(oid: &str) -> String {
    match oid_name(oid) {
        Some(n) => format!("{n} ({oid})"),
        None => oid.to_string(),
    }
}

/// A directory string (UTF8/Printable/IA5/Teletex/BMP/Universal), escaped.
fn text(t: &Tlv<'_>) -> String {
    match t.id {
        0x1e => {
            let units: Vec<u16> = t
                .value
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_be_bytes(*c))
                .collect();
            format!("{:?}", String::from_utf16_lossy(&units))
        }
        0x1c => {
            let s: String = t
                .value
                .as_chunks::<4>()
                .0
                .iter()
                .map(|c| char::from_u32(u32::from_be_bytes(*c)).unwrap_or('\u{fffd}'))
                .collect();
            format!("{s:?}")
        }
        _ => match core::str::from_utf8(t.value) {
            Ok(s) => format!("{s:?}"),
            Err(_) => format!("bytes={:02x?}", t.value),
        },
    }
}

/// `(path suffix, kind, value)` triples, appended by the readers below.
pub(crate) type Out = Vec<(String, &'static str, String)>;

/// Name ::= SEQUENCE OF RelativeDistinguishedName, each a SET of (OID, string).
fn name(prefix: &str, value: &[u8], out: &mut Out) -> Option<()> {
    for (i, rdn) in seq(value)?.iter().enumerate() {
        for atv in seq(rdn.value)? {
            let parts = seq(atv.value)?;
            let (o, v) = (parts.first()?, parts.get(1)?);
            let o = oid(o.value);
            out.push((
                format!("{prefix}/{i}/{}", oid_name(&o).unwrap_or(&o)),
                "X.509 name",
                text(v),
            ));
        }
    }
    Some(())
}

/// GeneralNames, as subjectAltName and the RFC 3161 `tsa` field use them.
fn general_names(prefix: &str, names: &[Tlv<'_>], out: &mut Out) {
    for (i, g) in names.iter().enumerate() {
        let (kind, value) = match g.id & 0x1f {
            1 => ("rfc822Name", text(g)),
            2 => ("dNSName", text(g)),
            6 => ("URI", text(g)),
            7 => ("iPAddress", format!("{:?}", g.value)),
            4 => {
                let mut sub = Out::new();
                if seq(g.value)
                    .and_then(|d| d.first().map(|n| name("", n.value, &mut sub)))
                    .flatten()
                    .is_some()
                {
                    for (p, k, v) in sub {
                        out.push((format!("{prefix}[{i}]/directoryName{p}"), k, v));
                    }
                    continue;
                }
                ("directoryName", format!("bytes={:02x?}", g.value))
            }
            n => {
                out.push((
                    format!("{prefix}[{i}]/[{n}]"),
                    "X.509 general name",
                    format!("bytes(len={})", g.value.len()),
                ));
                continue;
            }
        };
        out.push((format!("{prefix}[{i}]/{kind}"), "X.509 general name", value));
    }
}

fn algorithm(prefix: &str, value: &[u8], out: &mut Out) {
    if let Some(parts) = seq(value) {
        if let Some(o) = parts.first().filter(|o| o.is(0x06)) {
            out.push((prefix.to_string(), "X.509 algorithm", named(&oid(o.value))));
        }
        if let Some(p) = parts.get(1).filter(|p| p.is(0x06)) {
            out.push((
                format!("{prefix}/parameters"),
                "X.509 algorithm",
                named(&oid(p.value)),
            ));
        }
    }
}

/// One certificate's identifying fields. `None` if the structure is not X.509.
pub(crate) fn certificate(prefix: &str, der: &[u8], out: &mut Out) -> Option<()> {
    let (cert, rest) = tlv(der)?;
    if !cert.is(0x30) || !rest.is_empty() {
        return None;
    }
    let parts = seq(cert.value)?;
    let tbs = seq(parts.first().filter(|t| t.is(0x30))?.value)?;
    let mut f = tbs.iter().peekable();
    if let Some(v) = f.next_if(|t| t.is(0xa0)) {
        let ver = seq(v.value)?
            .first()
            .map(|i| i.value.last().copied().unwrap_or(0))
            .unwrap_or(0);
        out.push((
            format!("{prefix}/version"),
            "X.509",
            format!("v{}", u32::from(ver) + 1),
        ));
    }
    let serial = f.next().filter(|t| t.is(0x02))?;
    out.push((
        format!("{prefix}/serial"),
        "X.509",
        format!("{:02x?}", serial.value),
    ));
    algorithm(
        &format!("{prefix}/signature"),
        f.next().filter(|t| t.is(0x30))?.value,
        out,
    );
    name(
        &format!("{prefix}/issuer"),
        f.next().filter(|t| t.is(0x30))?.value,
        out,
    )?;
    let validity = seq(f.next().filter(|t| t.is(0x30))?.value)?;
    for (t, label) in validity.iter().zip(["notBefore", "notAfter"]) {
        out.push((format!("{prefix}/validity/{label}"), "X.509 time", text(t)));
    }
    name(
        &format!("{prefix}/subject"),
        f.next().filter(|t| t.is(0x30))?.value,
        out,
    )?;
    if let Some(spki) = f.next().filter(|t| t.is(0x30))
        && let Some(alg) = seq(spki.value)?.first()
    {
        algorithm(&format!("{prefix}/key"), alg.value, out);
    }
    for t in f {
        if t.is(0xa3) {
            let exts = seq(seq(t.value)?.first()?.value)?;
            for (i, ext) in exts.iter().enumerate() {
                let e = seq(ext.value)?;
                let o = oid(e.first()?.value);
                let critical = e
                    .get(1)
                    .is_some_and(|c| c.is(0x01) && c.value.first() == Some(&0xff));
                let body = e.iter().find(|x| x.is(0x04))?;
                let p = format!("{prefix}/extension[{i}]");
                out.push((
                    p.clone(),
                    "X.509 extension",
                    format!(
                        "{}{}, {} bytes",
                        named(&o),
                        if critical { ", critical" } else { "" },
                        body.value.len()
                    ),
                ));
                if o == "2.5.29.17"
                    && let Some(names) = tlv(body.value).and_then(|(s, _)| seq(s.value))
                {
                    general_names(&format!("{p}/subjectAltName"), &names, out);
                }
            }
        }
    }
    Some(())
}

/// An RFC 3161 TimeStampToken: TSTInfo fields and the certificates it carries.
/// `None` if the CMS framing or the TSTInfo is not as RFC 3161 defines it.
pub(crate) fn timestamp_token(prefix: &str, der: &[u8], out: &mut Out) -> Option<()> {
    let (ci, _) = tlv(der)?;
    let ci = seq(ci.value)?;
    if oid(ci.first()?.value) != "1.2.840.113549.1.7.2" {
        return None;
    }
    let signed = seq(seq(ci.get(1).filter(|t| t.is(0xa0))?.value)?.first()?.value)?;
    let encap = seq(signed.get(2)?.value)?;
    let econtent = seq(encap.get(1).filter(|t| t.is(0xa0))?.value)?;
    let tst = seq(tlv(econtent.first().filter(|t| t.is(0x04))?.value)?.0.value)?;
    let mut it = tst.iter();
    let _version = it.next()?;
    out.push((
        format!("{prefix}/tstInfo/policy"),
        "RFC 3161",
        oid(it.next()?.value),
    ));
    let imprint = seq(it.next()?.value)?;
    if let Some(alg) = imprint.first() {
        algorithm(
            &format!("{prefix}/tstInfo/messageImprint/algorithm"),
            alg.value,
            out,
        );
    }
    out.push((
        format!("{prefix}/tstInfo/serialNumber"),
        "RFC 3161",
        format!("{:02x?}", it.next()?.value),
    ));
    out.push((
        format!("{prefix}/tstInfo/genTime"),
        "RFC 3161",
        text(it.next()?),
    ));
    for t in it {
        if t.id == 0xa0 {
            if let Some(names) = seq(t.value) {
                general_names(&format!("{prefix}/tstInfo/tsa"), &names, out);
            }
        } else if t.id == 0x02 {
            out.push((
                format!("{prefix}/tstInfo/nonce"),
                "RFC 3161",
                format!("{:02x?}", t.value),
            ));
        }
    }
    let mut certs = 0;
    for t in signed.iter().skip(3) {
        if t.is(0xa0) {
            for (i, c) in seq(t.value)?.iter().enumerate() {
                let mut whole = Vec::with_capacity(c.value.len() + 6);
                encode_tlv(c.id, c.value, &mut whole);
                certificate(&format!("{prefix}/certificate[{i}]"), &whole, out)?;
                certs += 1;
            }
        } else if t.is(0x31) {
            let signers = seq(t.value)?;
            out.push((
                format!("{prefix}/signerInfos"),
                "RFC 3161",
                format!("{} signer(s), not interpreted", signers.len()),
            ));
        }
    }
    out.push((
        format!("{prefix}/certificates"),
        "RFC 3161",
        certs.to_string(),
    ));
    Some(())
}

fn encode_tlv(id: u8, value: &[u8], out: &mut Vec<u8>) {
    out.push(id);
    let n = value.len();
    if n < 0x80 {
        out.push(n as u8);
    } else {
        let bytes = (n as u32).to_be_bytes();
        let skip = bytes.iter().take_while(|&&b| b == 0).count();
        out.push(0x80 | (4 - skip) as u8);
        out.extend_from_slice(&bytes[skip..]);
    }
    out.extend_from_slice(value);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oids_decode() {
        assert_eq!(oid(&[0x55, 0x04, 0x03]), "2.5.4.3");
        assert_eq!(
            oid(&[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x09, 0x01]),
            "1.2.840.113549.1.9.1"
        );
        assert_eq!(oid(&[0x2b, 0x65, 0x70]), "1.3.101.112");
    }

    #[test]
    fn lengths_beyond_the_input_are_rejected() {
        assert!(tlv(&[0x30, 0x84, 0xff, 0xff, 0xff, 0xff]).is_none());
        assert!(tlv(&[0x30, 0x80]).is_none());
        assert!(tlv(&[0x1f, 0x01, 0x00]).is_none());
        let mut buf = Vec::new();
        encode_tlv(0x04, &[7u8; 300], &mut buf);
        let (t, rest) = tlv(&buf).unwrap();
        assert_eq!((t.id, t.value.len(), rest.len()), (0x04, 300, 0));
    }
}
