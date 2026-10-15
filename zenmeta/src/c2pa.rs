//! C2PA manifest-store inspection, reported through [`crate::audit::Report::c2pa`].
//!
//! The input is a complete manifest store: one JUMBF superbox of type `c2pa`, as a PNG
//! `caBX` chunk carries it or as reassembled from JPEG APP11 packets (reassembly and
//! other container framing stay with the codecs). Every manifest's claim and
//! assertions are flattened from CBOR into entries; the claim signature's COSE
//! headers are decoded, including each X.509 certificate's names, serial, validity
//! and subject-alternative names, and each RFC 3161 timestamp token's time, policy
//! and certificates. Nothing is verified: hashes, signatures and certificate chains
//! are reported, not checked.
//!
//! Content kept but not interpreted is a [`Finding::Opaque`]: embedded files and
//! thumbnails, unknown boxes, byte strings over 256 bytes that are not all zero, and
//! any certificate or token this reader cannot parse.
mod cbor;
mod der;
mod jumbf;

use alloc::{
    collections::BTreeMap,
    format,
    string::{String, ToString},
    vec::Vec,
};

use crate::audit::{Entry, Finding, Report};
use cbor::Value;

/// Largest manifest store inspected, and the most text the report may hold.
const LIMIT: usize = 16 * 1024 * 1024;
/// Byte strings up to this length are printed; longer ones are summarized.
const SHOWN_BYTES: usize = 256;

/// Deepest CBOR flattening, across embedded-CBOR (tag 24) re-parses: each re-parse
/// starts a fresh parser budget, so the walk keeps its own.
const MAX_FLATTEN_DEPTH: usize = 64;

struct Walk {
    entries: Vec<Entry>,
    findings: Vec<Finding>,
    budget: usize,
    over: bool,
    depth: usize,
}

pub(crate) fn inspect(data: &[u8]) -> Report {
    let mut w = Walk {
        entries: Vec::new(),
        findings: Vec::new(),
        budget: 0,
        over: false,
        depth: 0,
    };
    if data.len() > LIMIT {
        w.findings.push(Finding::Limit);
        return Report::from_parts(w.entries, w.findings);
    }
    match jumbf::boxes(data) {
        Ok(top) => {
            if top.len() != 1 || &top[0].ty != b"jumb" {
                w.invalid("c2pa", "a manifest store is exactly one jumb superbox");
            } else {
                w.superbox("c2pa", top[0].payload, 0, Some(*b"c2pa"));
            }
        }
        Err(e) => w.invalid("c2pa", e),
    }
    let mut seen = BTreeMap::<String, usize>::new();
    for e in &mut w.entries {
        let n = seen.entry(e.path().to_string()).or_default();
        *n += 1;
        if *n > 1 {
            e.set_path(format!("{}#{}", e.path(), n));
        }
    }
    w.entries.sort_by(|a, b| a.path().cmp(b.path()));
    Report::from_parts(w.entries, w.findings)
}

/// A path segment: labels and keys are free text, so `/` is escaped.
fn seg(s: &str) -> String {
    s.replace('%', "%25").replace('/', "%2F")
}

fn fourcc(t: &[u8; 4]) -> String {
    match core::str::from_utf8(t) {
        Ok(s) if s.chars().all(|c| c.is_ascii_graphic() || c == ' ') => s.to_string(),
        _ => format!("{t:02x?}"),
    }
}

impl Walk {
    fn push(&mut self, path: String, kind: &str, value: String) {
        if self.over {
            return;
        }
        self.budget = self
            .budget
            .saturating_add(path.len() + kind.len() + value.len() + 64);
        if self.budget > LIMIT {
            self.over = true;
            self.findings.push(Finding::Limit);
            return;
        }
        self.entries.push(Entry::new(path, kind.to_string(), value));
    }

    fn invalid(&mut self, path: &str, why: &str) {
        self.findings
            .push(Finding::InvalidJumbf(format!("{path}: {why}")));
    }

    fn opaque(&mut self, path: &str, bytes: usize) {
        self.findings.push(Finding::Opaque {
            path: path.to_string(),
            bytes,
        });
    }

    /// A superbox: its description, then its content boxes. `expect` is the type the
    /// parent requires (only the store's type is enforced).
    fn superbox(&mut self, path: &str, payload: &[u8], depth: usize, expect: Option<[u8; 4]>) {
        if depth > jumbf::MAX_DEPTH {
            self.invalid(path, "superboxes nest too deeply");
            return;
        }
        let parts = match jumbf::boxes(payload) {
            Ok(p) => p,
            Err(e) => return self.invalid(path, e),
        };
        let Some((first, content)) = parts.split_first().filter(|(f, _)| &f.ty == b"jumd") else {
            return self.invalid(path, "superbox does not start with a description box");
        };
        let d = match jumbf::description(first.payload) {
            Ok(d) => d,
            Err(e) => return self.invalid(path, e),
        };
        let ty = jumbf::c2pa_type(&d.uuid);
        if expect.is_some() && ty != expect {
            return self.invalid(path, "not a C2PA manifest store");
        }
        let type_name = match ty {
            Some(t) => fourcc(&t),
            None if d.uuid == jumbf::EMBEDDED_FILE => "embedded file".to_string(),
            None => format!("uuid {:02x?}", d.uuid),
        };
        self.push(format!("{path}/@type"), "JUMBF description", type_name);
        if let Some(l) = &d.label {
            self.push(
                format!("{path}/@label"),
                "JUMBF description",
                format!("{l:?}"),
            );
        }
        if let Some(id) = d.id {
            self.push(format!("{path}/@id"), "JUMBF description", id.to_string());
        }
        if let Some(h) = d.hash {
            self.push(
                format!("{path}/@hash"),
                "JUMBF description",
                format!("{h:02x?}"),
            );
        }
        if let Some((t, len)) = d.private {
            self.push(
                format!("{path}/@private"),
                "JUMBF description",
                format!("{} box, {len} bytes", fourcc(&t)),
            );
        }
        let mut children = BTreeMap::<String, usize>::new();
        for b in content {
            match &b.ty {
                b"jumb" => {
                    let label = jumbf::boxes(b.payload)
                        .ok()
                        .and_then(|p| {
                            p.first()
                                .filter(|f| &f.ty == b"jumd")
                                .and_then(|f| jumbf::description(f.payload).ok())
                        })
                        .and_then(|d| d.label);
                    let name = seg(label.as_deref().unwrap_or("jumb"));
                    let n = children.entry(name.clone()).or_default();
                    let child = if *n == 0 {
                        format!("{path}/{name}")
                    } else {
                        format!("{path}/{name}[{n}]")
                    };
                    *n += 1;
                    self.superbox(&child, b.payload, depth + 1, None);
                }
                b"cbor" => match cbor::parse(b.payload) {
                    Ok(v) if ty == Some(*b"c2cs") => self.cose(&format!("{path}/cose"), &v),
                    Ok(v) => self.cbor(&format!("{path}/cbor"), &v),
                    Err(e) => self.findings.push(Finding::InvalidCbor {
                        path: format!("{path}/cbor"),
                        error: format!("{e:?}"),
                    }),
                },
                b"json" => match core::str::from_utf8(b.payload) {
                    Ok(s) => self.push(format!("{path}/json"), "JSON text", format!("{s:?}")),
                    Err(_) => self.opaque(&format!("{path}/json"), b.payload.len()),
                },
                b"bfdb" => self.file_description(&format!("{path}/bfdb"), b.payload),
                b"bidb" => {
                    self.push(
                        format!("{path}/bidb"),
                        "JUMBF binary data",
                        format!("bytes(len={})", b.payload.len()),
                    );
                    self.opaque(&format!("{path}/bidb"), b.payload.len());
                }
                other => {
                    let p = format!("{path}/{}", seg(&fourcc(other)));
                    self.push(
                        p.clone(),
                        "JUMBF box",
                        format!("bytes(len={})", b.payload.len()),
                    );
                    self.opaque(&p, b.payload.len());
                }
            }
        }
    }

    /// Embedded-file description: toggles, NUL-terminated media type, optional name.
    fn file_description(&mut self, path: &str, p: &[u8]) {
        let Some((&toggles, rest)) = p.split_first() else {
            return self.invalid(path, "empty embedded-file description");
        };
        let mut fields = rest.split(|&b| b == 0);
        let media = fields.next().unwrap_or_default();
        self.push(
            format!("{path}/media_type"),
            "JUMBF embedded file",
            format!("{:?}", String::from_utf8_lossy(media)),
        );
        if toggles & 0x01 != 0 {
            let name = fields.next().unwrap_or_default();
            self.push(
                format!("{path}/file_name"),
                "JUMBF embedded file",
                format!("{:?}", String::from_utf8_lossy(name)),
            );
        }
    }

    fn cbor(&mut self, path: &str, v: &Value) {
        if self.depth >= MAX_FLATTEN_DEPTH {
            return self.findings.push(Finding::InvalidCbor {
                path: path.to_string(),
                error: "nested too deeply".into(),
            });
        }
        self.depth += 1;
        self.cbor_value(path, v);
        self.depth -= 1;
    }

    fn cbor_value(&mut self, path: &str, v: &Value) {
        match v {
            Value::Map(pairs) => {
                if pairs.is_empty() {
                    self.push(path.to_string(), "CBOR map", "{}".into());
                }
                for (k, v) in pairs {
                    let key = match k {
                        Value::Text(t) => seg(t),
                        Value::Uint(_) | Value::Nint(_) => {
                            k.as_i64().map_or_else(|| "int".into(), |n| n.to_string())
                        }
                        other => format!("{other:?}"),
                    };
                    self.cbor(&format!("{path}/{key}"), v);
                }
            }
            Value::Array(items) => {
                if items.is_empty() {
                    self.push(path.to_string(), "CBOR array", "[]".into());
                }
                for (i, v) in items.iter().enumerate() {
                    self.cbor(&format!("{path}[{i}]"), v);
                }
            }
            Value::Tag(24, inner) => match inner.as_bytes().map(cbor::parse) {
                Some(Ok(nested)) => self.cbor(&format!("{path}/tag(24)"), &nested),
                _ => self.cbor(&format!("{path}/tag(24)"), inner),
            },
            Value::Tag(t, inner) => self.cbor(&format!("{path}/tag({t})"), inner),
            Value::Bytes(b) => self.bytes(path, "CBOR bytes", b),
            Value::Text(t) => self.push(path.to_string(), "CBOR text", format!("{t:?}")),
            Value::Uint(_) | Value::Nint(_) => {
                let s = match v {
                    Value::Uint(n) => n.to_string(),
                    Value::Nint(n) => format!("-{}", u128::from(*n) + 1),
                    _ => unreachable!(),
                };
                self.push(path.to_string(), "CBOR int", s)
            }
            Value::Float(f) => self.push(path.to_string(), "CBOR float", format!("{f:?}")),
            Value::Bool(b) => self.push(path.to_string(), "CBOR simple", b.to_string()),
            Value::Null => self.push(path.to_string(), "CBOR simple", "null".into()),
            Value::Undefined => self.push(path.to_string(), "CBOR simple", "undefined".into()),
            Value::Simple(n) => self.push(path.to_string(), "CBOR simple", format!("simple({n})")),
        }
    }

    fn bytes(&mut self, path: &str, kind: &str, b: &[u8]) {
        if b.iter().all(|&x| x == 0) && b.len() > SHOWN_BYTES {
            self.push(
                path.to_string(),
                kind,
                format!("bytes(len={}, all zero)", b.len()),
            );
        } else if b.len() <= SHOWN_BYTES {
            self.push(path.to_string(), kind, format!("{b:02x?}"));
        } else {
            self.push(path.to_string(), kind, format!("bytes(len={})", b.len()));
            self.opaque(path, b.len());
        }
    }

    /// COSE_Sign1 (RFC 9052): protected and unprotected headers, payload, signature.
    fn cose(&mut self, path: &str, v: &Value) {
        let body = match v {
            Value::Tag(18, inner) => inner.as_ref(),
            other => other,
        };
        let Value::Array(parts) = body else {
            return self.findings.push(Finding::InvalidCbor {
                path: path.to_string(),
                error: "not a COSE_Sign1 array".into(),
            });
        };
        let [protected, unprotected, payload, signature] = parts.as_slice() else {
            return self.findings.push(Finding::InvalidCbor {
                path: path.to_string(),
                error: "COSE_Sign1 needs four parts".into(),
            });
        };
        match protected.as_bytes().map(cbor::parse) {
            Some(Ok(h)) => self.headers(&format!("{path}/protected"), &h),
            Some(Err(e)) => self.findings.push(Finding::InvalidCbor {
                path: format!("{path}/protected"),
                error: format!("{e:?}"),
            }),
            None => self.cbor(&format!("{path}/protected"), protected),
        }
        self.headers(&format!("{path}/unprotected"), unprotected);
        match payload {
            Value::Null => self.push(format!("{path}/payload"), "COSE", "detached".into()),
            other => self.cbor(&format!("{path}/payload"), other),
        }
        match signature.as_bytes() {
            Some(s) => self.push(
                format!("{path}/signature"),
                "COSE",
                format!("bytes(len={})", s.len()),
            ),
            None => self.cbor(&format!("{path}/signature"), signature),
        }
    }

    fn headers(&mut self, path: &str, h: &Value) {
        let Value::Map(pairs) = h else {
            return self.cbor(path, h);
        };
        for (k, v) in pairs {
            match (k.as_i64(), k.as_text()) {
                (Some(1), _) => {
                    let alg = v.as_i64();
                    let name = match alg {
                        Some(-7) => " (ES256)",
                        Some(-35) => " (ES384)",
                        Some(-36) => " (ES512)",
                        Some(-37) => " (PS256)",
                        Some(-38) => " (PS384)",
                        Some(-39) => " (PS512)",
                        Some(-8) => " (EdDSA)",
                        _ => "",
                    };
                    self.push(
                        format!("{path}/alg"),
                        "COSE header",
                        format!("{}{name}", alg.map_or("?".into(), |a| a.to_string())),
                    );
                }
                (Some(33), _) | (_, Some("x5chain")) => self.chain(&format!("{path}/x5chain"), v),
                (_, Some(t @ ("sigTst" | "sigTst2"))) => self.timestamps(&format!("{path}/{t}"), v),
                (Some(n), _) => self.cbor(&format!("{path}/{n}"), v),
                (_, Some(t)) => self.cbor(&format!("{path}/{}", seg(t)), v),
                _ => self.cbor(&format!("{path}/{k:?}"), v),
            }
        }
    }

    fn chain(&mut self, path: &str, v: &Value) {
        let certs: Vec<&[u8]> = match v {
            Value::Bytes(b) => alloc::vec![b.as_slice()],
            Value::Array(items) => items.iter().filter_map(Value::as_bytes).collect(),
            other => return self.cbor(path, other),
        };
        for (i, c) in certs.iter().enumerate() {
            let p = format!("{path}[{i}]");
            let mut out = der::Out::new();
            match der::certificate(&p, c, &mut out) {
                Some(()) => out.into_iter().for_each(|(p, k, v)| self.push(p, k, v)),
                None => {
                    self.push(p.clone(), "X.509", format!("bytes(len={})", c.len()));
                    self.opaque(&p, c.len());
                }
            }
        }
    }

    /// `sigTst`: `{"tstTokens": [{"val": TimeStampToken}, ...]}`.
    fn timestamps(&mut self, path: &str, v: &Value) {
        let Some(Value::Array(tokens)) = v.get_text("tstTokens") else {
            return self.cbor(path, v);
        };
        for (i, t) in tokens.iter().enumerate() {
            let p = format!("{path}/tstTokens[{i}]");
            match t.get_text("val").and_then(Value::as_bytes) {
                Some(der_bytes) => {
                    let mut out = der::Out::new();
                    match der::timestamp_token(&p, der_bytes, &mut out) {
                        Some(()) => out.into_iter().for_each(|(p, k, v)| self.push(p, k, v)),
                        None => {
                            self.push(
                                p.clone(),
                                "RFC 3161",
                                format!("bytes(len={})", der_bytes.len()),
                            );
                            self.opaque(&p, der_bytes.len());
                        }
                    }
                }
                None => self.cbor(&p, t),
            }
        }
    }
}
