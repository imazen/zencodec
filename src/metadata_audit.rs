//! Explicit metadata inspection and structural diffing.
//!
//! Audits do not filter or certify privacy. Unknown/opaque content is reported,
//! never interpreted as harmless. EXIF entries use the borrowing iterator;
//! XML is optional (`xmp` feature). No image pixels are touched.
use crate::exif::{ByteOrder, Exif};
use alloc::{
    collections::BTreeMap,
    format,
    string::{String, ToString},
    vec::Vec,
};

/// An inspected field. Values use escaped text or typed numeric/hex notation.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Entry {
    path: String,
    kind: String,
    value: String,
}
impl Entry {
    /// Directory/namespace path, including occurrence index for duplicate tags.
    pub fn path(&self) -> &str {
        &self.path
    }
    /// Carrier and field type; unknown types retain their numeric identifier.
    pub fn kind(&self) -> &str {
        &self.kind
    }
    /// Decoded numeric values or losslessly escaped text/hex bytes.
    pub fn value(&self) -> &str {
        &self.value
    }
}
/// Inspection limitation or content requiring an explicit retention decision.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Finding {
    /// EXIF header/directory could not be parsed.
    InvalidExif,
    /// The forgiving EXIF parser can skip malformed entries and does not model
    /// arbitrary SubIFDs. Retain the source for byte-level forensic comparison.
    ExifCoverageLimited,
    /// Opaque vendor data may contain identity/location and rendering parameters.
    MakerNote { path: String, bytes: usize },
    /// Input exceeds the inspection budget.
    Limit,
    /// XMP XML is invalid or exceeds its parsing budget.
    InvalidXmp(String),
}
/// Read-only metadata snapshot with explicit coverage findings.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    entries: Vec<Entry>,
    findings: Vec<Finding>,
}
impl Report {
    /// Inspect EXIF/TIFF up to 16 MiB. Preserve unknown tags and duplicate entries.
    pub fn exif(bytes: &[u8]) -> Self {
        let mut out = Self::default();
        if bytes.len() > 16 * 1024 * 1024 {
            out.findings.push(Finding::Limit);
            return out;
        }
        let Some(exif) = Exif::parse(bytes) else {
            out.findings.push(Finding::InvalidExif);
            return out;
        };
        out.findings.push(Finding::ExifCoverageLimited);
        let mut occurrences = BTreeMap::<String, usize>::new();
        let mut budget = 0usize;
        for e in exif.entries() {
            budget = budget
                .saturating_add(e.value.len().saturating_mul(24))
                .saturating_add(256);
            if budget > 16 * 1024 * 1024 {
                out.findings.push(Finding::Limit);
                break;
            }
            let key = format!("exif/{:?}/0x{:04x}", e.ifd, e.tag);
            let index = occurrences.entry(key.clone()).or_default();
            let path = format!("{key}[{index}]");
            *index += 1;
            if e.tag == 0x927c {
                out.findings.push(Finding::MakerNote {
                    path: path.clone(),
                    bytes: e.value.len(),
                });
            }
            out.entries.push(Entry {
                path,
                kind: format!("TIFF type={} count={}", e.kind, e.count),
                value: display_value(e.kind, e.value, exif.byte_order()),
            });
        }
        out.entries.sort_by(|a, b| a.path.cmp(&b.path));
        out
    }
    /// Inspect every XML field, including unknown namespaces and annotations.
    /// Does not canonicalize all RDF equivalents or interpret vendor schemas.
    #[cfg(feature = "xmp")]
    pub fn xmp(xml: &str) -> Self {
        match crate::xmp::Packet::parse(xml).and_then(|p| p.fields()) {
            Ok(fields) => Self {
                entries: fields
                    .into_iter()
                    .map(|f| Entry {
                        path: f.path,
                        kind: "XML".into(),
                        value: f.value,
                    })
                    .collect(),
                findings: Vec::new(),
            },
            Err(e) => Self {
                entries: Vec::new(),
                findings: alloc::vec![Finding::InvalidXmp(e.to_string())],
            },
        }
    }
    /// Sorted fields. Absence does not prove a malformed source contained no data.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }
    /// Coverage and opaque-data findings; inspect these along with field diffs.
    pub fn findings(&self) -> &[Finding] {
        &self.findings
    }
    /// Compare field paths/types/values. Findings are deliberately separate so an
    /// empty field diff is never presented as proof of equal parse coverage.
    pub fn diff<'a>(&'a self, after: &'a Self) -> Vec<Change<'a>> {
        let before: BTreeMap<_, _> = self.entries.iter().map(|e| (e.path(), e)).collect();
        let next: BTreeMap<_, _> = after.entries.iter().map(|e| (e.path(), e)).collect();
        let mut changes = Vec::new();
        for (path, e) in &before {
            match next.get(path) {
                None => changes.push(Change::Removed(e)),
                Some(n) if e != n => changes.push(Change::Modified {
                    before: e,
                    after: n,
                }),
                _ => {}
            }
        }
        for (path, e) in &next {
            if !before.contains_key(path) {
                changes.push(Change::Added(e));
            }
        }
        changes
    }
}
/// A structural field change; unchanged values allocate no copy.
#[derive(Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Change<'a> {
    Added(&'a Entry),
    Removed(&'a Entry),
    Modified { before: &'a Entry, after: &'a Entry },
}
fn display_value(kind: u16, bytes: &[u8], order: ByteOrder) -> String {
    if kind == 2 || kind == 129 {
        return match core::str::from_utf8(bytes) {
            Ok(text) => format!("{text:?}"),
            Err(_) => format!("bytes={bytes:02x?}"),
        };
    }
    let width = match kind {
        1 | 6 => 1,
        3 | 8 => 2,
        4 | 9 | 11 | 13 => 4,
        5 | 10 | 12 => 8,
        _ => return format!("{bytes:02x?}"),
    };
    let number = |b: &[u8]| -> u64 {
        match order {
            ByteOrder::Little => b
                .iter()
                .enumerate()
                .fold(0, |v, (i, b)| v | ((*b as u64) << (i * 8))),
            ByteOrder::Big => b.iter().fold(0, |v, b| (v << 8) | *b as u64),
        }
    };
    let values: Vec<_> = bytes
        .chunks_exact(width)
        .map(|b| match kind {
            6 => (number(b) as i8).to_string(),
            8 => (number(b) as i16).to_string(),
            9 => (number(b) as i32).to_string(),
            5 => format!("{}/{}", number(&b[..4]), number(&b[4..])),
            10 => format!("{}/{}", number(&b[..4]) as i32, number(&b[4..]) as i32),
            11 => format!(
                "{:?} [bits={:08x}]",
                f32::from_bits(number(b) as u32),
                number(b)
            ),
            12 => format!("{:?} [bits={:016x}]", f64::from_bits(number(b)), number(b)),
            _ => number(b).to_string(),
        })
        .collect();
    format!("[{}]", values.join(", "))
}
