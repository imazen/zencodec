//! Validated directory editing using the same RDF traversal as inspection.

use super::{ContainerItem, ItemSemantic};
use crate::xmp::{Error, Packet, SpannedProperty};
use alloc::{
    string::{String, ToString},
    vec::Vec,
};
use core::ops::Range;

const MAX_ITEMS: usize = 256;

struct Entry {
    item: ContainerItem,
    length: Option<Range<usize>>,
    padding: Option<Range<usize>>,
}

/// A validated GContainer directory and its source packet.
///
/// Supports Google Photos and Dynamic Depth namespaces, including aliases.
/// A directory must start with exactly one primary, and each secondary must
/// have a unique semantic, MIME type and decimal length. At most 256 items
/// are accepted. Unknown fields remain untouched during editing.
pub struct Directory<'a> {
    xml: &'a str,
    entries: Vec<Entry>,
}

impl<'a> Directory<'a> {
    /// Parse a complete namespace-bound XMP packet. A packet without a
    /// directory is valid and has no items. Malformed or ambiguous XML fails.
    pub fn parse(xml: &'a str) -> Result<Self, Error> {
        let packet = Packet::parse(xml)?;
        let mut entries = Vec::new();
        for (container_ns, item_ns) in [
            (
                super::super::gainmap::CONTAINER_NAMESPACE,
                super::super::gainmap::ITEM_NAMESPACE,
            ),
            (
                "http://ns.google.com/photos/dd/1.0/container/",
                "http://ns.google.com/photos/dd/1.0/item/",
            ),
        ] {
            let count = packet
                .doc
                .descendants()
                .filter(|n| n.has_tag_name((container_ns, "Directory")))
                .count();
            if count > 1
                || packet
                    .doc
                    .descendants()
                    .any(|n| n.attribute((container_ns, "Directory")).is_some())
            {
                return Err(Error::Duplicate("Directory".into()));
            }
            let items = packet.resource_sequence_spanned(
                container_ns,
                "Directory",
                container_ns,
                "Item",
                MAX_ITEMS,
            )?;
            if count != 0 && items.is_empty() {
                return Err(Error::Structure("empty or misplaced Directory".into()));
            }
            if !entries.is_empty() && !items.is_empty() {
                return Err(Error::Duplicate("Directory namespaces".into()));
            }
            for fields in items {
                let field = |name| {
                    fields
                        .iter()
                        .find(|f| f.property.namespace() == item_ns && f.property.name() == name)
                };
                let required = |name| {
                    field(name)
                        .map(|f| f.property.value())
                        .filter(|s| !s.is_empty())
                        .ok_or_else(|| Error::Structure(alloc::format!("missing item {name}")))
                };
                let semantic = required("Semantic")?;
                let mime = required("Mime")?;
                if entries
                    .iter()
                    .any(|e: &Entry| e.item.semantic.as_xmp_str() == semantic)
                {
                    return Err(Error::Duplicate("item semantic".into()));
                }
                if (semantic == "Primary") != entries.is_empty() {
                    return Err(Error::Structure(
                        "primary must be the first directory item".into(),
                    ));
                }
                let number = |name| -> Result<Option<usize>, Error> {
                    field(name)
                        .map(|f| {
                            let value = f.property.value();
                            if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
                                return Err(Error::Structure(alloc::format!(
                                    "invalid item {name}"
                                )));
                            }
                            value.parse().map_err(|_| {
                                Error::Structure(alloc::format!("invalid item {name}"))
                            })
                        })
                        .transpose()
                };
                let length = number("Length")?;
                let padding = number("Padding")?;
                if semantic != "Primary" && length.is_none() {
                    return Err(Error::Structure("secondary item requires Length".into()));
                }
                entries.push(Entry {
                    item: ContainerItem {
                        semantic: ItemSemantic::from_xmp(semantic),
                        mime: mime.into(),
                        length,
                        padding,
                    },
                    length: field("Length").map(|f| value_range(xml, f)).transpose()?,
                    padding: field("Padding").map(|f| value_range(xml, f)).transpose()?,
                });
            }
        }
        Ok(Self { xml, entries })
    }

    /// Items in directory order, including the primary.
    pub fn items(&self) -> impl ExactSizeIterator<Item = &ContainerItem> {
        self.entries.iter().map(|e| &e.item)
    }

    /// Replace secondary lengths by semantic and set secondary padding to zero.
    ///
    /// The caller supplies the final payload lengths. Unlisted payloads (such
    /// as MPF thumbnails) are allowed, but every directory secondary must match
    /// exactly one supplied semantic. Primary fields and all bytes outside the
    /// selected numeric values are retained. Element content, including CDATA,
    /// is replaced by decimal text; attribute quote style is retained.
    pub fn rewrite_item_lengths(&self, secondaries: &[(&str, usize)]) -> Result<String, Error> {
        let mut edits = Vec::new();
        for entry in &self.entries {
            if entry.item.semantic == ItemSemantic::Primary {
                continue;
            }
            let mut matches = secondaries
                .iter()
                .filter(|(semantic, _)| *semantic == entry.item.semantic.as_xmp_str());
            let (_, length) = matches
                .next()
                .ok_or_else(|| Error::Structure("directory item has no carried payload".into()))?;
            if matches.next().is_some() {
                return Err(Error::Duplicate("carried item semantic".into()));
            }
            edits.push((
                entry
                    .length
                    .clone()
                    .ok_or_else(|| Error::Structure("secondary Length".into()))?,
                length.to_string(),
            ));
            if let Some(range) = &entry.padding {
                edits.push((range.clone(), "0".into()));
            }
        }
        edits.sort_unstable_by_key(|(r, _)| r.start);
        let mut size = self.xml.len();
        let mut last = 0;
        for (range, replacement) in &edits {
            if range.start < last || range.end > self.xml.len() || range.start > range.end {
                return Err(Error::Structure("overlapping directory edits".into()));
            }
            size = size
                .checked_sub(range.len())
                .and_then(|s| s.checked_add(replacement.len()))
                .ok_or(Error::Limit)?;
            last = range.end;
        }
        if size > super::super::gainmap::MAX_XMP_LENGTH {
            return Err(Error::Limit);
        }
        let mut out = String::new();
        out.try_reserve_exact(size).map_err(|_| Error::Limit)?;
        last = 0;
        for (range, replacement) in edits {
            out.push_str(&self.xml[last..range.start]);
            out.push_str(&replacement);
            last = range.end;
        }
        out.push_str(&self.xml[last..]);
        Ok(out)
    }
}

// The XML parser already validated each source range. Locate only its value
// boundary: Attribute::range_value truncates unusually long names/whitespace
// in roxmltree 0.21, so use its full, non-truncated attribute range instead.
fn value_range(xml: &str, field: &SpannedProperty) -> Result<Range<usize>, Error> {
    let source = &xml[field.source.clone()];
    let invalid = || Error::Structure("directory value span".into());
    if field.attribute {
        let first = source
            .bytes()
            .position(|b| b == b'\'' || b == b'"')
            .ok_or_else(invalid)?;
        if source.as_bytes().last() != source.as_bytes().get(first) {
            return Err(invalid());
        }
        Ok(field.source.start + first + 1..field.source.end - 1)
    } else {
        let mut quote = None;
        let mut begin = None;
        for (i, b) in source.bytes().enumerate() {
            if let Some(q) = quote {
                if b == q {
                    quote = None;
                }
            } else if b == b'\'' || b == b'"' {
                quote = Some(b);
            } else if b == b'>' {
                begin = Some(i + 1);
                break;
            }
        }
        let start = begin.ok_or_else(invalid)?;
        let end = source.rfind("</").ok_or_else(invalid)?;
        if start > end {
            return Err(invalid());
        }
        Ok(field.source.start + start..field.source.start + end)
    }
}
