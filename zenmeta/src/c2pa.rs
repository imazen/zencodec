//! Bounded structural inspection of an opaque C2PA manifest store.
//!
//! Container framing belongs to codecs. This module does not validate bindings,
//! signatures, credentials or trust; semantic verification belongs to c2pa-rs.
mod budget;
mod cbor;
mod jumbf;

use alloc::{string::String, vec::Vec};
use core::fmt;

use crate::audit::{Entry, Finding, Report};
use budget::{Budget, LIMIT, Limit};
use cbor::Value;

const SHOWN_BYTES: usize = 256;
const MAX_FLATTEN_DEPTH: usize = 64;
// '#' plus every decimal digit of usize, on every supported target.
const OCCURRENCE_ROOM: usize = 1 + usize::BITS as usize;

struct Walk {
    entries: Vec<Entry>,
    order: Vec<usize>,
    findings: Vec<Finding>,
    budget: Budget,
    depth: usize,
}

pub(crate) fn inspect(data: &[u8]) -> Report {
    let mut walk = Walk {
        entries: Vec::new(),
        order: Vec::new(),
        // A fixed emergency slot, independent of untrusted input. Every finding
        // insertion keeps a spare slot so exhaustion can always be reported once.
        findings: Vec::with_capacity(1),
        budget: Budget::new(),
        depth: 0,
    };
    let result = if data.len() > LIMIT {
        Err(Limit)
    } else {
        walk.store(data)
    };
    if result.is_err() {
        walk.findings.push(Finding::Limit);
    }
    walk.finish()
}

/// User text cannot impersonate generated array/tag/occurrence/description paths.
struct Segment<'a>(&'a str);
impl fmt::Display for Segment<'_> {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        for c in self.0.chars() {
            match c {
                '%' | '/' | '#' | '[' | ']' | '@' | '(' | ')' => write!(out, "%{:02X}", c as u32)?,
                _ => write!(out, "{c}")?,
            }
        }
        Ok(())
    }
}

struct Fourcc([u8; 4]);
impl fmt::Display for Fourcc {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match core::str::from_utf8(&self.0) {
            Ok(s) if s.chars().all(|c| c.is_ascii_graphic() || c == ' ') => out.write_str(s),
            _ => write!(out, "{:02x?}", self.0),
        }
    }
}

type Result<T = ()> = core::result::Result<T, Limit>;

impl Walk {
    fn text(&mut self, args: fmt::Arguments<'_>) -> Result<String> {
        self.budget.text(args)
    }

    fn entry(&mut self, path: &str, kind: &str, value: fmt::Arguments<'_>) -> Result {
        let mut path = self.text(format_args!("{path}"))?;
        // Final path disambiguation must work even after the budget is exhausted.
        self.budget.charge(OCCURRENCE_ROOM)?;
        path.try_reserve_exact(OCCURRENCE_ROOM).map_err(|_| Limit)?;
        let kind = self.text(format_args!("{kind}"))?;
        let value = self.text(value)?;
        self.budget.reserve(&mut self.entries, 1)?;
        self.budget.reserve(&mut self.order, 1)?;
        self.order.push(self.entries.len());
        self.entries.push(Entry::new(path, kind, value));
        Ok(())
    }

    fn finding(&mut self, finding: Finding) -> Result {
        self.budget.reserve(&mut self.findings, 2)?;
        self.findings.push(finding);
        Ok(())
    }

    fn invalid(&mut self, path: &str, why: &str) -> Result {
        let message = self.text(format_args!("{path}: {why}"))?;
        self.finding(Finding::InvalidJumbf(message))
    }

    fn invalid_cbor(&mut self, path: &str, error: fmt::Arguments<'_>) -> Result {
        let path = self.text(format_args!("{path}"))?;
        let error = self.text(error)?;
        self.finding(Finding::InvalidCbor { path, error })
    }

    fn opaque(&mut self, path: &str, bytes: usize) -> Result {
        let path = self.text(format_args!("{path}"))?;
        self.finding(Finding::Opaque { path, bytes })
    }

    fn finish(mut self) -> Report {
        // Sort indices first, retaining source occurrence order for equal paths.
        // Both the indices and suffix capacity were admitted with each entry.
        self.order.sort_unstable_by(|&a, &b| {
            self.entries[a]
                .path()
                .cmp(self.entries[b].path())
                .then(a.cmp(&b))
        });
        let mut start = 0;
        while start < self.order.len() {
            let first = self.order[start];
            let mut end = start + 1;
            while end < self.order.len()
                && self.entries[self.order[end]].path() == self.entries[first].path()
            {
                end += 1;
            }
            for n in start + 1..end {
                self.entries[self.order[n]].append_occurrence(n - start + 1);
            }
            start = end;
        }
        self.entries.sort_unstable_by(|a, b| a.path().cmp(b.path()));
        Report::from_parts(self.entries, self.findings)
    }

    fn store(&mut self, data: &[u8]) -> Result {
        let mut boxes = jumbf::Boxes::new(data);
        match boxes.next() {
            Some(Ok(root)) if root.ty == *b"jumb" && boxes.next().is_none() => {
                self.superbox("c2pa", root.payload, 0, Some(*b"c2pa"))
            }
            Some(Err(error)) => self.invalid("c2pa", error),
            _ => self.invalid("c2pa", "a manifest store is exactly one jumb superbox"),
        }
    }

    fn superbox(
        &mut self,
        path: &str,
        payload: &[u8],
        depth: usize,
        expect: Option<[u8; 4]>,
    ) -> Result {
        self.budget.item()?;
        if depth > jumbf::MAX_DEPTH {
            return self.invalid(path, "superboxes nest too deeply");
        }
        let mut boxes = jumbf::Boxes::new(payload);
        let description = match boxes.next() {
            Some(Ok(first)) if first.ty == *b"jumd" => match jumbf::description(first.payload) {
                Ok(d) => d,
                Err(error) => return self.invalid(path, error),
            },
            Some(Err(error)) => return self.invalid(path, error),
            _ => return self.invalid(path, "superbox does not start with a description box"),
        };
        let ty = jumbf::c2pa_type(&description.uuid);
        if expect.is_some() && ty != expect {
            return self.invalid(path, "not a C2PA manifest store");
        }
        let p = self.text(format_args!("{path}/@type"))?;
        match ty {
            Some(t) => self.entry(&p, "JUMBF description", format_args!("{}", Fourcc(t)))?,
            None if description.uuid == jumbf::EMBEDDED_FILE => {
                self.entry(&p, "JUMBF description", format_args!("embedded file"))?;
            }
            None => self.entry(
                &p,
                "JUMBF description",
                format_args!("uuid {:02x?}", description.uuid),
            )?,
        }
        if let Some(label) = description.label {
            let p = self.text(format_args!("{path}/@label"))?;
            self.entry(&p, "JUMBF description", format_args!("{label:?}"))?;
        }
        if let Some(id) = description.id {
            let p = self.text(format_args!("{path}/@id"))?;
            self.entry(&p, "JUMBF description", format_args!("{id}"))?;
        }
        if let Some(hash) = description.hash {
            let p = self.text(format_args!("{path}/@hash"))?;
            self.entry(&p, "JUMBF description", format_args!("{hash:02x?}"))?;
        }
        if let Some((t, len)) = description.private {
            let p = self.text(format_args!("{path}/@private"))?;
            self.entry(
                &p,
                "JUMBF description",
                format_args!("{} box, {len} bytes", Fourcc(t)),
            )?;
        }
        for child in boxes {
            self.budget.item()?;
            let child = match child {
                Ok(child) => child,
                Err(error) => return self.invalid(path, error),
            };
            match &child.ty {
                b"jumb" => {
                    let label = jumbf::Boxes::new(child.payload)
                        .next()
                        .and_then(core::result::Result::ok)
                        .filter(|first| first.ty == *b"jumd")
                        .and_then(|first| jumbf::description(first.payload).ok())
                        .and_then(|d| d.label)
                        .unwrap_or("jumb");
                    let p = self.text(format_args!("{path}/{}", Segment(label)))?;
                    self.superbox(&p, child.payload, depth + 1, None)?;
                }
                b"cbor" => {
                    let p = self.text(format_args!(
                        "{path}/{}",
                        if ty == Some(*b"c2cs") { "cose" } else { "cbor" }
                    ))?;
                    match cbor::parse_with_budget(child.payload, &mut self.budget) {
                        Ok(v) if ty == Some(*b"c2cs") => self.cose(&p, &v)?,
                        Ok(v) => self.cbor(&p, &v)?,
                        Err(cbor::Error::Budget) => return Err(Limit),
                        Err(error) => self.invalid_cbor(&p, format_args!("{error:?}"))?,
                    }
                }
                b"json" => {
                    let p = self.text(format_args!("{path}/json"))?;
                    match core::str::from_utf8(child.payload) {
                        Ok(s) => self.entry(&p, "JSON text", format_args!("{s:?}"))?,
                        Err(_) => self.opaque(&p, child.payload.len())?,
                    }
                }
                b"bfdb" => {
                    let p = self.text(format_args!("{path}/bfdb"))?;
                    self.file_description(&p, child.payload)?;
                }
                other => {
                    let name = self.text(format_args!("{}", Fourcc(*other)))?;
                    let p = self.text(format_args!("{path}/{}", Segment(&name)))?;
                    let kind = if other == b"bidb" {
                        "JUMBF binary data"
                    } else {
                        "JUMBF box"
                    };
                    self.entry(&p, kind, format_args!("bytes(len={})", child.payload.len()))?;
                    self.opaque(&p, child.payload.len())?;
                }
            }
        }
        Ok(())
    }

    fn file_description(&mut self, path: &str, payload: &[u8]) -> Result {
        let Some((&toggles, rest)) = payload.split_first() else {
            return self.invalid(path, "empty embedded-file description");
        };
        let mut fields = rest.split(|&b| b == 0);
        for field in ["media_type", "file_name"]
            .iter()
            .take(if toggles & 1 != 0 { 2 } else { 1 })
        {
            let bytes = fields.next().unwrap_or_default();
            let p = self.text(format_args!("{path}/{field}"))?;
            match core::str::from_utf8(bytes) {
                Ok(s) => self.entry(&p, "JUMBF embedded file", format_args!("{s:?}"))?,
                Err(_) => self.opaque(&p, bytes.len())?,
            }
        }
        Ok(())
    }

    fn cbor(&mut self, path: &str, value: &Value) -> Result {
        self.budget.item()?;
        if self.depth >= MAX_FLATTEN_DEPTH {
            return self.invalid_cbor(path, format_args!("nested too deeply"));
        }
        self.depth += 1;
        let result = self.cbor_value(path, value);
        self.depth -= 1;
        result
    }

    fn map_item(&mut self, path: &str, index: usize, key: &Value, value: &Value) -> Result {
        let p = match key {
            Value::Text(text) => self.text(format_args!("{path}/{}", Segment(text)))?,
            Value::Uint(n) => self.text(format_args!("{path}/@uint({n})"))?,
            Value::Nint(n) => self.text(format_args!("{path}/@nint({n})"))?,
            other => {
                let p = self.text(format_args!("{path}/@key({index})/key"))?;
                self.cbor(&p, other)?;
                self.text(format_args!("{path}/@key({index})/value"))?
            }
        };
        self.cbor(&p, value)
    }

    fn cbor_value(&mut self, path: &str, value: &Value) -> Result {
        match value {
            Value::Map(pairs) => {
                if pairs.is_empty() {
                    self.entry(path, "CBOR map", format_args!("{{}}"))?;
                }
                for (i, (k, v)) in pairs.iter().enumerate() {
                    self.map_item(path, i, k, v)?;
                }
            }
            Value::Array(items) => {
                if items.is_empty() {
                    self.entry(path, "CBOR array", format_args!("[]"))?;
                }
                for (i, v) in items.iter().enumerate() {
                    let p = self.text(format_args!("{path}[{i}]"))?;
                    self.cbor(&p, v)?;
                }
            }
            Value::Tag(t, inner) => {
                let p = self.text(format_args!("{path}/tag({t})"))?;
                let inner = &inner[0];
                if *t == 24
                    && let Some(bytes) = inner.as_bytes()
                {
                    match cbor::parse_with_budget(bytes, &mut self.budget) {
                        Ok(v) => return self.cbor(&p, &v),
                        Err(cbor::Error::Budget) => return Err(Limit),
                        Err(_) => {}
                    }
                }
                self.cbor(&p, inner)?;
            }
            Value::Bytes(bytes) => self.bytes(path, "CBOR bytes", bytes)?,
            Value::Text(text) => self.entry(path, "CBOR text", format_args!("{text:?}"))?,
            Value::Uint(n) => self.entry(path, "CBOR int", format_args!("{n}"))?,
            Value::Nint(n) => {
                self.entry(path, "CBOR int", format_args!("-{}", u128::from(*n) + 1))?
            }
            Value::Float(f) => self.entry(path, "CBOR float", format_args!("{f:?}"))?,
            Value::Bool(b) => self.entry(path, "CBOR simple", format_args!("{b}"))?,
            Value::Null => self.entry(path, "CBOR simple", format_args!("null"))?,
            Value::Undefined => self.entry(path, "CBOR simple", format_args!("undefined"))?,
            Value::Simple(n) => self.entry(path, "CBOR simple", format_args!("simple({n})"))?,
        }
        Ok(())
    }

    fn bytes(&mut self, path: &str, kind: &str, bytes: &[u8]) -> Result {
        if bytes.len() > SHOWN_BYTES && bytes.iter().all(|&b| b == 0) {
            self.entry(
                path,
                kind,
                format_args!("bytes(len={}, all zero)", bytes.len()),
            )
        } else if bytes.len() <= SHOWN_BYTES {
            self.entry(path, kind, format_args!("{bytes:02x?}"))
        } else {
            self.entry(path, kind, format_args!("bytes(len={})", bytes.len()))?;
            self.opaque(path, bytes.len())
        }
    }

    fn cose(&mut self, path: &str, value: &Value) -> Result {
        let body = match value {
            Value::Tag(18, inner) => &inner[0],
            other => other,
        };
        let Value::Array(parts) = body else {
            return self.invalid_cbor(path, format_args!("not a COSE_Sign1 array"));
        };
        let [protected, unprotected, payload, signature] = parts.as_slice() else {
            return self.invalid_cbor(path, format_args!("COSE_Sign1 needs four parts"));
        };
        let p = self.text(format_args!("{path}/protected"))?;
        match protected.as_bytes() {
            Some(bytes) => match cbor::parse_with_budget(bytes, &mut self.budget) {
                Ok(h) => self.headers(&p, &h)?,
                Err(cbor::Error::Budget) => return Err(Limit),
                Err(error) => self.invalid_cbor(&p, format_args!("{error:?}"))?,
            },
            None => self.cbor(&p, protected)?,
        }
        let p = self.text(format_args!("{path}/unprotected"))?;
        self.headers(&p, unprotected)?;
        let p = self.text(format_args!("{path}/payload"))?;
        match payload {
            Value::Null => self.entry(&p, "COSE", format_args!("detached"))?,
            other => self.cbor(&p, other)?,
        }
        let p = self.text(format_args!("{path}/signature"))?;
        match signature.as_bytes() {
            Some(bytes) => self.entry(&p, "COSE", format_args!("bytes(len={})", bytes.len())),
            None => self.cbor(&p, signature),
        }
    }

    fn headers(&mut self, path: &str, headers: &Value) -> Result {
        let Value::Map(pairs) = headers else {
            return self.cbor(path, headers);
        };
        for (i, (k, v)) in pairs.iter().enumerate() {
            self.budget.item()?;
            match (k.as_i64(), k.as_text()) {
                (Some(1), _) => {
                    let p = self.text(format_args!("{path}/alg"))?;
                    let name = match v.as_i64() {
                        Some(-7) => " (ES256)",
                        Some(-35) => " (ES384)",
                        Some(-36) => " (ES512)",
                        Some(-37) => " (PS256)",
                        Some(-38) => " (PS384)",
                        Some(-39) => " (PS512)",
                        Some(-8) => " (EdDSA)",
                        _ => "",
                    };
                    match v.as_i64() {
                        Some(alg) => self.entry(&p, "COSE header", format_args!("{alg}{name}"))?,
                        None => self.cbor(&p, v)?,
                    }
                }
                (Some(33), _) => {
                    let p = self.text(format_args!("{path}/x5chain"))?;
                    self.chain(&p, v)?;
                }
                (_, Some("x5chain")) => {
                    let p = self.text(format_args!("{path}/@text/x5chain"))?;
                    self.chain(&p, v)?;
                }
                (_, Some(t @ ("sigTst" | "sigTst2"))) => {
                    let p = self.text(format_args!("{path}/{t}"))?;
                    self.timestamps(&p, v)?;
                }
                (_, Some(text)) => {
                    let p = self.text(format_args!("{path}/@text/{}", Segment(text)))?;
                    self.cbor(&p, v)?;
                }
                _ => self.map_item(path, i, k, v)?,
            }
        }
        Ok(())
    }

    fn chain(&mut self, path: &str, value: &Value) -> Result {
        match value {
            Value::Bytes(bytes) => {
                let p = self.text(format_args!("{path}[0]"))?;
                self.certificate(&p, bytes)
            }
            Value::Array(items) => {
                for (i, item) in items.iter().enumerate() {
                    self.budget.item()?;
                    let p = self.text(format_args!("{path}[{i}]"))?;
                    match item.as_bytes() {
                        Some(bytes) => self.certificate(&p, bytes)?,
                        None => self.cbor(&p, item)?,
                    }
                }
                Ok(())
            }
            other => self.cbor(path, other),
        }
    }

    fn certificate(&mut self, path: &str, bytes: &[u8]) -> Result {
        self.entry(path, "X.509", format_args!("bytes(len={})", bytes.len()))?;
        self.opaque(path, bytes.len())
    }

    fn timestamps(&mut self, path: &str, value: &Value) -> Result {
        let Value::Map(pairs) = value else {
            return self.cbor(path, value);
        };
        if pairs.is_empty() {
            return self.cbor(path, value);
        }
        for (i, (key, value)) in pairs.iter().enumerate() {
            self.budget.item()?;
            if key.as_text() == Some("tstTokens")
                && let Value::Array(tokens) = value
                && !tokens.is_empty()
            {
                for (j, token) in tokens.iter().enumerate() {
                    self.budget.item()?;
                    let p = self.text(format_args!("{path}/tstTokens[{j}]"))?;
                    self.timestamp_token(&p, token)?;
                }
            } else {
                self.map_item(path, i, key, value)?;
            }
        }
        Ok(())
    }

    fn timestamp_token(&mut self, path: &str, token: &Value) -> Result {
        let Value::Map(pairs) = token else {
            return self.cbor(path, token);
        };
        if pairs.is_empty() {
            return self.cbor(path, token);
        }
        for (i, (key, value)) in pairs.iter().enumerate() {
            self.budget.item()?;
            if key.as_text() == Some("val")
                && let Some(bytes) = value.as_bytes()
            {
                let p = self.text(format_args!("{path}/val"))?;
                self.entry(&p, "RFC 3161", format_args!("bytes(len={})", bytes.len()))?;
                self.opaque(&p, bytes.len())?;
            } else {
                self.map_item(path, i, key, value)?;
            }
        }
        Ok(())
    }
}
