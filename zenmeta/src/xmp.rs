//! Namespace-aware XMP inspection. XML prefixes are aliases, never identities.
//!
//! This is a bounded XML reader with selected RDF property access, not a complete
//! RDF reasoner or an Adobe alias-normalization engine. Unknown structures remain
//! visible in the audit snapshot. DTDs/external entities are rejected. Reading
//! metadata never reads, scans or allocates image pixels.
use alloc::{
    collections::BTreeMap,
    format,
    string::{String, ToString},
    vec::Vec,
};
use roxmltree::{Document, Node, ParsingOptions};

pub(crate) const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";

/// Invalid, ambiguous, unsupported or over-budget metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// Byte, node or nesting budget exceeded.
    Limit,
    /// XML syntax error (including forbidden DTDs).
    Xml(String),
    /// Multiple declarations of a property; choosing one could change rendering.
    Duplicate(String),
    /// Selected property uses an unsupported RDF shape.
    Structure(String),
}
impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "XMP: {self:?}")
    }
}
impl core::error::Error for Error {}

/// One XML field in a namespace-resolved structural snapshot.
/// Paths include sibling occurrence indexes, preserving duplicates and arrays.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Field {
    pub(crate) path: String,
    pub(crate) value: String,
}
impl Field {
    /// Expanded namespace path; prefix spelling is immaterial.
    pub fn path(&self) -> &str {
        &self.path
    }
    /// Decoded attribute/text value, or an empty value for an element marker.
    pub fn value(&self) -> &str {
        &self.value
    }
}

/// A namespace-qualified scalar in an RDF resource structure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Property {
    namespace: String,
    name: String,
    value: String,
}
impl Property {
    /// Namespace URI, independent of prefix spelling.
    pub fn namespace(&self) -> &str {
        &self.namespace
    }
    /// Local property name.
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Decoded scalar value.
    pub fn value(&self) -> &str {
        &self.value
    }
}

/// Bounded parsed XMP packet, borrowing its source XML.
#[derive(Debug)]
pub struct Packet<'a> {
    doc: Document<'a>,
}
impl<'a> Packet<'a> {
    /// Parse at most 16 MiB / 65,536 XML nodes / 64 nested elements.
    /// Unknown XML is inspectable, but selected property access is strict.
    pub fn parse(xml: &'a str) -> Result<Self, Error> {
        if xml.len() > 16 * 1024 * 1024 {
            return Err(Error::Limit);
        }
        let doc = Document::parse_with_options(
            xml,
            ParsingOptions {
                allow_dtd: false,
                nodes_limit: 65_536,
                ..Default::default()
            },
        )
        .map_err(|e| Error::Xml(e.to_string()))?;
        if doc
            .descendants()
            .any(|n| n.ancestors().take(66).count() > 65)
        {
            return Err(Error::Limit);
        }
        Ok(Self { doc })
    }

    /// Read a property of the document's empty-subject RDF Description.
    /// Accepts a scalar attribute/element or an rdf:Seq array of plain values.
    /// Duplicate declarations, alternate subjects, qualifiers and mixed content
    /// refuse instead of selecting a possibly different rendering interpretation.
    pub fn property(&self, namespace: &str, name: &str) -> Result<Option<Vec<String>>, Error> {
        let mut result = None;
        for d in self.descriptions() {
            let attribute = d.attribute((namespace, name));
            let children: Vec<_> = d
                .children()
                .filter(|n| n.has_tag_name((namespace, name)))
                .collect();
            if attribute.is_none() && children.is_empty() {
                continue;
            }
            empty_subject(d)?;
            if let Some(value) = attribute
                && result.replace(alloc::vec![value.into()]).is_some()
            {
                return Err(Error::Duplicate(name.into()));
            }
            for n in children {
                if result.replace(property_values(n)?).is_some() {
                    return Err(Error::Duplicate(name.into()));
                }
            }
        }
        Ok(result)
    }

    fn descriptions(&self) -> impl Iterator<Item = Node<'_, 'a>> {
        self.doc.descendants().filter(|n| {
            n.has_tag_name((RDF, "Description"))
                && n.parent().is_some_and(|p| {
                    p.has_tag_name((RDF, "RDF"))
                        && (p == self.doc.root_element()
                            || p.parent().is_some_and(|root| {
                                root == self.doc.root_element()
                                    && root.has_tag_name(("adobe:ns:meta/", "xmpmeta"))
                            }))
                })
        })
    }

    /// Read a sequence of resource structures (e.g. GContainer Directory).
    /// Each item contains namespace-qualified scalar properties. Unknown scalar
    /// fields are returned too; the caller chooses its schema, never its prefix.
    pub fn resource_sequence(
        &self,
        namespace: &str,
        name: &str,
        item_namespace: &str,
        item_name: &str,
    ) -> Result<Vec<Vec<Property>>, Error> {
        if self
            .descriptions()
            .any(|d| d.attribute((namespace, name)).is_some())
        {
            return Err(Error::Structure(name.into()));
        }
        let nodes: Vec<_> = self
            .descriptions()
            .flat_map(|d| d.children())
            .filter(|n| n.has_tag_name((namespace, name)))
            .collect();
        if nodes.len() > 1 {
            return Err(Error::Duplicate(name.into()));
        }
        let Some(node) = nodes.first() else {
            return Ok(Vec::new());
        };
        empty_subject(node.parent().ok_or_else(|| Error::Structure(name.into()))?)?;
        structural_node(*node, false)?;
        let seq = only_child(*node)
            .filter(|n| n.has_tag_name((RDF, "Seq")))
            .ok_or_else(|| Error::Structure(name.into()))?;
        structural_node(seq, false)?;
        let mut items = Vec::new();
        for li in seq.children().filter(Node::is_element) {
            if !li.has_tag_name((RDF, "li")) {
                return Err(Error::Structure(name.into()));
            }
            structural_node(li, true)?;
            let item = only_child(li).ok_or_else(|| Error::Structure(name.into()))?;
            if !item.has_tag_name((item_namespace, item_name))
                || item
                    .children()
                    .any(|c| c.is_text() && c.text().is_some_and(|s| !s.trim().is_empty()))
            {
                return Err(Error::Structure(name.into()));
            }
            let mut fields = Vec::new();
            for a in item.attributes() {
                if a.namespace() == Some(RDF) {
                    return Err(Error::Structure(name.into()));
                }
                fields.push(Property {
                    namespace: a.namespace().unwrap_or("").into(),
                    name: a.name().into(),
                    value: a.value().into(),
                });
            }
            for child in item.children().filter(Node::is_element) {
                fields.push(Property {
                    namespace: child.tag_name().namespace().unwrap_or("").into(),
                    name: child.tag_name().name().into(),
                    value: plain_text(child)?,
                });
            }
            if fields.len() > 256 || items.len() >= 4096 {
                return Err(Error::Limit);
            }
            for (i, a) in fields.iter().enumerate() {
                if fields[..i]
                    .iter()
                    .any(|b| a.namespace == b.namespace && a.name == b.name)
                {
                    return Err(Error::Duplicate(a.name.clone()));
                }
            }
            items.push(fields);
        }
        Ok(items)
    }

    /// Full XML structural snapshot: elements, attributes, text, comments and
    /// processing instructions. Namespace declarations are represented by resolved
    /// names, not prefix spelling. No unknown application property is discarded.
    /// This is structural comparison, not equivalence of arbitrary RDF graphs.
    pub fn fields(&self) -> Result<Vec<Field>, Error> {
        let mut out = Vec::new();
        let mut paths = BTreeMap::<u32, String>::new();
        let mut occurrences = BTreeMap::<(u32, String), usize>::new();
        let mut budget = 0usize;
        for n in self.doc.descendants().filter(|n| !n.is_root()) {
            let parent = n.parent().map(|p| p.id().get()).unwrap_or(0);
            let name = if n.is_element() {
                format!(
                    "{{{}}}{}",
                    n.tag_name().namespace().unwrap_or(""),
                    n.tag_name().name()
                )
            } else if n.is_comment() {
                "#comment".into()
            } else if n.is_pi() {
                "#pi".into()
            } else {
                "#text".into()
            };
            let index = occurrences.entry((parent, name.clone())).or_default();
            let path = format!(
                "{}/{name}[{index}]",
                paths.get(&parent).map(String::as_str).unwrap_or("")
            );
            *index += 1;
            budget = budget.saturating_add(path.len() * 2);
            if budget > 16 * 1024 * 1024 {
                return Err(Error::Limit);
            }
            paths.insert(n.id().get(), path.clone());
            if n.is_element() {
                out.push(Field {
                    path: path.clone(),
                    value: String::new(),
                });
                for a in n.attributes() {
                    budget = budget.saturating_add(
                        path.len()
                            + a.namespace().unwrap_or("").len()
                            + a.name().len()
                            + a.value().len(),
                    );
                    if budget > 16 * 1024 * 1024 {
                        return Err(Error::Limit);
                    }
                    out.push(Field {
                        path: format!("{path}/@{{{}}}{}", a.namespace().unwrap_or(""), a.name()),
                        value: a.value().into(),
                    });
                }
            } else if let Some(pi) = n.pi() {
                out.push(Field {
                    path,
                    value: format!("{} {}", pi.target, pi.value.unwrap_or("")),
                });
            } else if let Some(text) = n.text() {
                // Formatting whitespace between elements is not a property.
                if n.is_comment()
                    || !text.trim().is_empty()
                    || n.parent()
                        .is_some_and(|p| !p.children().any(|c| c.is_element()))
                {
                    out.push(Field {
                        path,
                        value: text.into(),
                    });
                }
            }
        }
        out.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(out)
    }
}
fn only_child<'a, 'input>(n: Node<'a, 'input>) -> Option<Node<'a, 'input>> {
    let mut it = n.children().filter(Node::is_element);
    let first = it.next()?;
    it.next().is_none().then_some(first)
}
fn plain_text(n: Node<'_, '_>) -> Result<String, Error> {
    if n.children().any(|c| c.is_element()) || n.attributes().len() != 0 {
        return Err(Error::Structure(n.tag_name().name().into()));
    }
    Ok(n.children()
        .filter(Node::is_text)
        .filter_map(|c| c.text())
        .collect())
}
fn property_values(n: Node<'_, '_>) -> Result<Vec<String>, Error> {
    if n.children().any(|c| c.is_element()) {
        if n.attributes().len() != 0
            || n.children()
                .any(|c| c.is_text() && c.text().is_some_and(|s| !s.trim().is_empty()))
        {
            return Err(Error::Structure(n.tag_name().name().into()));
        }
        let seq = only_child(n)
            .filter(|c| c.has_tag_name((RDF, "Seq")))
            .ok_or_else(|| Error::Structure(n.tag_name().name().into()))?;
        structural_node(seq, false)?;
        let mut values = Vec::new();
        for li in seq.children().filter(Node::is_element) {
            if !li.has_tag_name((RDF, "li")) {
                return Err(Error::Structure(n.tag_name().name().into()));
            }
            values.push(plain_text(li)?);
        }
        Ok(values)
    } else {
        Ok(alloc::vec![plain_text(n)?])
    }
}

fn structural_node(n: Node<'_, '_>, allow_resource: bool) -> Result<(), Error> {
    if n.attributes().any(|a| {
        !(allow_resource
            && a.namespace() == Some(RDF)
            && a.name() == "parseType"
            && a.value() == "Resource")
    }) || n
        .children()
        .any(|c| c.is_text() && c.text().is_some_and(|s| !s.trim().is_empty()))
    {
        return Err(Error::Structure(n.tag_name().name().into()));
    }
    Ok(())
}

fn empty_subject(n: Node<'_, '_>) -> Result<(), Error> {
    if n.attributes()
        .any(|a| a.namespace() == Some(RDF) && (a.name() != "about" || !a.value().is_empty()))
    {
        return Err(Error::Structure("RDF subject/qualifier".into()));
    }
    Ok(())
}
