//! Structural inventory: a byte-exact map of an encoded file's parts.
//!
//! An [`Inventory`] lists every structural part a decoder finds in a file —
//! JPEG marker segments and scan data, PNG and RIFF chunks, ISOBMFF boxes,
//! items and their extents, GIF blocks, TIFF directories — with the part's
//! byte range and what the decoder does with it ([`Disposition`]).
//!
//! The point is completeness, not interpretation. Top-level parts tile the
//! whole input, and a container that declares a [`body`](Part::body) has
//! children that tile that body, so bytes a decoder never reads (unknown
//! chunk types, data after the logical end, slack inside a container) appear
//! as parts instead of being skipped silently. [`Inventory::validate`] checks
//! that coverage mechanically.
//!
//! Use it to audit files for metadata or identifying data that would
//! otherwise slip through a pipeline: it says *where* every byte is and
//! whether anything reads it. It does not judge content. Field-level
//! inspection of EXIF or XMP payloads is a separate concern.
//!
//! Decoders produce inventories through
//! [`DecodeJob::inventory`](crate::decode::DecodeJob::inventory); the default
//! implementation returns `Ok(None)`, and
//! [`DecodeCapabilities::inventory`](crate::decode::DecodeCapabilities::inventory)
//! says whether a codec implements it.
//!
//! ```
//! use zencodec::ImageFormat;
//! use zencodec::inventory::{Disposition, Inventory, Part, PartKind, PartTag};
//!
//! // A 30-byte file: an 8-byte signature, a 12-byte chunk, then 10 bytes the
//! // walker never claimed.
//! let mut inv = Inventory::new(ImageFormat::Png, 30);
//! inv.push(None, Part::new(PartKind::Header, PartTag::None, 0..8, Disposition::Structure))
//!     .unwrap();
//! inv.push(None, Part::new(PartKind::Chunk, PartTag::FourCc(*b"zzZz"), 8..20, Disposition::Unknown))
//!     .unwrap();
//! inv.fill_gaps(None, Disposition::Trailing).unwrap();
//! inv.validate().unwrap();
//! assert_eq!(inv.unconsumed().count(), 2);
//! ```

use alloc::borrow::Cow;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;
use core::ops::Range;

use crate::ImageFormat;

/// Default cap on the number of parts one inventory may hold.
///
/// A crafted file can contain millions of tiny units (a PNG of 12-byte chunks,
/// a GIF of one-byte sub-blocks). A part costs about 140 bytes plus its label
/// and detail strings; 2^20 parts measured 146 MiB peak RSS without strings and
/// 178 MiB with a short label and detail on each. Raise the cap with
/// [`Inventory::with_max_parts`] when auditing trusted, unusually fragmented
/// files; walkers should stop collecting at the cap rather than after it.
pub const DEFAULT_MAX_PARTS: u32 = 1 << 20;

/// Index of a part within its [`Inventory`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PartId(u32);

impl PartId {
    /// The part's position in [`Inventory::parts`].
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// The structural class of a part.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PartKind {
    /// A file signature or fixed header: the PNG signature, the GIF header and
    /// logical screen descriptor, the RIFF header, the BMP file header, a bare
    /// codestream's start marker.
    Header,
    /// A JPEG marker segment (APPn, COM, DQT, DHT, SOFn, SOS header, …) or a
    /// standalone marker (SOI, EOI, RSTn).
    Segment,
    /// Entropy-coded data following a JPEG SOS header, including any RSTn
    /// markers inside it.
    ScanData,
    /// A PNG chunk or a RIFF chunk.
    Chunk,
    /// An ISOBMFF, JPEG XL container or JPEG 2000 box.
    Box,
    /// A HEIF/AVIF item's declaration (its `infe` entry). The item's data is
    /// its [`Extent`](PartKind::Extent) parts inside `mdat` or `idat`, tagged
    /// with the same item ID.
    Item,
    /// One extent of an item's data, tagged with the item ID.
    Extent,
    /// A HEIF item property (a child of `ipco`).
    Property,
    /// A GIF block: an extension, an image descriptor, a colour table, image
    /// data, the trailer.
    Block,
    /// A TIFF image file directory.
    Ifd,
    /// A TIFF field: an IFD entry or its out-of-line value.
    Field,
    /// A named attribute (an OpenEXR header attribute, for example).
    Attribute,
    /// A complete embedded image stream: an MPF secondary image, a thumbnail.
    EmbeddedImage,
    /// A coded unit inside a bitstream: an AV1 OBU, an HEVC NAL unit.
    CodedUnit,
    /// Bytes after the format's logical end (after JPEG EOI, PNG IEND, the GIF
    /// trailer, the RIFF size, the last box).
    Trailer,
    /// Bytes that no other part covers.
    Gap,
}

impl PartKind {
    /// Short lowercase name, for tables and logs.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Header => "header",
            Self::Segment => "segment",
            Self::ScanData => "scan-data",
            Self::Chunk => "chunk",
            Self::Box => "box",
            Self::Item => "item",
            Self::Extent => "extent",
            Self::Property => "property",
            Self::Block => "block",
            Self::Ifd => "ifd",
            Self::Field => "field",
            Self::Attribute => "attribute",
            Self::EmbeddedImage => "embedded-image",
            Self::CodedUnit => "coded-unit",
            Self::Trailer => "trailer",
            Self::Gap => "gap",
        }
    }
}

/// A part's identity within its format.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PartTag {
    /// No natural identity (headers, gaps, trailers).
    None,
    /// A JPEG marker byte: `0xE1` is APP1, `0xFE` is COM.
    Marker(u8),
    /// A four-character code: a PNG chunk type, a RIFF chunk id, an ISOBMFF box,
    /// item or property type.
    FourCc([u8; 4]),
    /// A numeric code: a TIFF tag, a GIF extension label, an AV1 OBU type, an
    /// HEVC NAL unit type, a HEIF item ID.
    Code(u32),
    /// A name: an OpenEXR attribute, an XML element.
    Name(Cow<'static, str>),
}

impl fmt::Display for PartTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => f.write_str("-"),
            Self::Marker(m) => write!(f, "FF{m:02X}"),
            Self::FourCc(cc) => {
                if cc.iter().all(|b| (0x20..0x7f).contains(b)) {
                    for &b in cc {
                        write!(f, "{}", b as char)?;
                    }
                    Ok(())
                } else {
                    write!(f, "0x{:08X}", u32::from_be_bytes(*cc))
                }
            }
            Self::Code(c) => write!(f, "{c:#x}"),
            Self::Name(n) => f.write_str(n),
        }
    }
}

/// Which surfaced metadata a part feeds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MetadataKind {
    /// EXIF, surfaced through
    /// [`EmbeddedMetadata::exif`](crate::decode::EmbeddedMetadata::exif).
    Exif,
    /// XMP, surfaced through
    /// [`EmbeddedMetadata::xmp`](crate::decode::EmbeddedMetadata::xmp).
    Xmp,
    /// An ICC profile, surfaced through
    /// [`SourceColor`](crate::decode::SourceColor).
    Icc,
    /// CICP code points, surfaced through
    /// [`SourceColor`](crate::decode::SourceColor).
    Cicp,
    /// Other colour signalling the decoder applies or reports (PNG `sRGB`,
    /// `gAMA`/`cHRM`, BMP colour space fields).
    Colour,
    /// HDR static metadata (content light level, mastering display).
    HdrStatic,
    /// Gain-map parameters or the gain-map image.
    GainMap,
    /// Orientation, surfaced through [`ImageInfo::orientation`](crate::ImageInfo).
    Orientation,
    /// Pixel density, surfaced through [`ImageInfo::resolution`](crate::ImageInfo).
    Resolution,
    /// Animation control: frame timing, loop count, disposal.
    Animation,
    /// A supplement the decoder reports (thumbnail, depth map, alpha plane).
    Supplement,
}

impl MetadataKind {
    /// Short lowercase name, for tables and logs.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Exif => "exif",
            Self::Xmp => "xmp",
            Self::Icc => "icc",
            Self::Cicp => "cicp",
            Self::Colour => "colour",
            Self::HdrStatic => "hdr-static",
            Self::GainMap => "gain-map",
            Self::Orientation => "orientation",
            Self::Resolution => "resolution",
            Self::Animation => "animation",
            Self::Supplement => "supplement",
        }
    }
}

/// What the decoder does with a part's bytes.
///
/// The first three variants are *consumed*: the decoder reads them to
/// produce pixels or the metadata it reports. Everything else is
/// [unconsumed](Disposition::is_consumed): nothing the caller receives depends
/// on those bytes, so they pass through a decode–re-encode pipeline only if
/// something copies them, and survive a file copy untouched. Auditing those
/// parts is the reason this module exists.
///
/// A disposition must describe what the decoder actually does, not what the
/// format allows: a PNG `tEXt` chunk the decoder never reads is
/// [`Skipped`](Disposition::Skipped) even though the format gives it a meaning.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Disposition {
    /// Container framing the decoder interprets: signatures, chunk and box
    /// headers, index tables (`iloc`, IFD entry tables, the MPF index).
    Structure,
    /// Coded image data the decoder turns into pixels: entropy-coded scans,
    /// `IDAT`, item extents of decoded images, strips and tiles.
    ImageData,
    /// Data the decoder reports to the caller as metadata.
    Metadata(MetadataKind),
    /// Recognised and parsed, but nothing from it reaches the caller
    /// (`pHYs` when no resolution is reported, `tIME`, JFIF density).
    Dropped,
    /// A recognised type the decoder deliberately does not read (thumbnails,
    /// comments, Photoshop resources, MakerNote, secondary images).
    Skipped,
    /// A type the decoder does not recognise (private chunks, unknown boxes,
    /// unrecognised APPn signatures).
    Unknown,
    /// Bytes after the format's logical end.
    Trailing,
    /// Bytes inside a container that no index or structure references (slack
    /// in `mdat`, unreferenced space between TIFF values).
    Unreferenced,
    /// Filler the format defines as meaningless (`free`/`skip` boxes, JPEG fill
    /// bytes, RIFF pad bytes). Editors sometimes leave stale data in it.
    Padding,
    /// Bytes the walker could not interpret: a corrupt length, a truncated unit,
    /// a bad checksum. The walk stopped or resynchronised here.
    Malformed,
}

impl Disposition {
    /// Whether the decoder reads these bytes to produce what the caller
    /// receives: [`Structure`](Self::Structure),
    /// [`ImageData`](Self::ImageData) or [`Metadata`](Self::Metadata).
    pub const fn is_consumed(self) -> bool {
        matches!(self, Self::Structure | Self::ImageData | Self::Metadata(_))
    }

    /// Short lowercase name, for tables and logs.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Structure => "structure",
            Self::ImageData => "image-data",
            Self::Metadata(_) => "metadata",
            Self::Dropped => "dropped",
            Self::Skipped => "skipped",
            Self::Unknown => "unknown",
            Self::Trailing => "trailing",
            Self::Unreferenced => "unreferenced",
            Self::Padding => "padding",
            Self::Malformed => "malformed",
        }
    }
}

impl fmt::Display for Disposition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Metadata(kind) => write!(f, "metadata({})", kind.name()),
            other => f.write_str(other.name()),
        }
    }
}

/// One structural part of a file.
///
/// Build with [`Part::new`] and the `with_*` methods; record it with
/// [`Inventory::push`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Part {
    /// The part this one is nested in, or `None` at the top level.
    parent: Option<PartId>,
    /// Structural class.
    kind: PartKind,
    /// Identity within the format.
    tag: PartTag,
    /// A short identifying label taken from the file: an APPn signature
    /// (`"Exif"`, `"ICC_PROFILE"`), a PNG text keyword, a GIF application
    /// identifier, a HEIF item name or content type.
    label: Option<Cow<'static, str>>,
    /// Absolute byte range in the input, headers included.
    range: Range<u64>,
    /// For a container whose children account for every byte after its own
    /// header: the sub-range those children must tile exactly.
    /// [`Inventory::validate`] enforces it. `None` for leaves, and for
    /// containers whose content is reported as a whole.
    body: Option<Range<u64>>,
    /// What the decoder does with the bytes.
    disposition: Disposition,
    /// Free-form detail: why a part is malformed, an item's MIME type, an
    /// auxiliary image URN.
    detail: Option<String>,
}

impl Part {
    /// A part with no label, body or detail.
    pub fn new(kind: PartKind, tag: PartTag, range: Range<u64>, disposition: Disposition) -> Self {
        Self {
            parent: None,
            kind,
            tag,
            label: None,
            range,
            body: None,
            disposition,
            detail: None,
        }
    }

    /// Set the identifying label.
    pub fn with_label(mut self, label: impl Into<Cow<'static, str>>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Declare the sub-range that this part's children tile exactly.
    pub fn with_body(mut self, body: Range<u64>) -> Self {
        self.body = Some(body);
        self
    }

    /// Set the free-form detail.
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Change what the decoder does with the bytes, before the part is pushed.
    /// (After `push`, use [`Inventory::set_disposition`].)
    pub fn set_disposition(&mut self, disposition: Disposition) {
        self.disposition = disposition;
    }

    /// Set the identifying label, before the part is pushed.
    pub fn set_label(&mut self, label: impl Into<Cow<'static, str>>) {
        self.label = Some(label.into());
    }

    /// Declare the sub-range this part's children tile exactly, before the part is pushed.
    pub fn set_body(&mut self, body: Range<u64>) {
        self.body = Some(body);
    }

    /// Set the free-form detail, before the part is pushed.
    /// (After `push`, use [`Inventory::set_detail`].)
    pub fn set_detail(&mut self, detail: impl Into<String>) {
        self.detail = Some(detail.into());
    }

    /// The part this one is nested in, or `None` at the top level.
    pub fn parent(&self) -> Option<PartId> {
        self.parent
    }

    /// Structural class.
    pub fn kind(&self) -> PartKind {
        self.kind
    }

    /// Identity within the format.
    pub fn tag(&self) -> &PartTag {
        &self.tag
    }

    /// The identifying label copied from the file, if any.
    pub fn label(&self) -> Option<&str> {
        self.label.as_deref()
    }

    /// Absolute byte range in the input, headers included.
    pub fn range(&self) -> Range<u64> {
        self.range.clone()
    }

    /// The sub-range this container's children tile exactly, if declared.
    pub fn body(&self) -> Option<Range<u64>> {
        self.body.clone()
    }

    /// What the decoder does with the bytes.
    pub fn disposition(&self) -> Disposition {
        self.disposition
    }

    /// The walker's free-form remark, if any.
    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }

    /// Number of bytes the part spans.
    pub fn len(&self) -> u64 {
        self.range.end.saturating_sub(self.range.start)
    }

    /// Whether the part spans no bytes.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Why an inventory is invalid, or could not grow.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum InventoryError {
    /// [`Inventory::push`] was called past the part cap.
    TooManyParts {
        /// The cap that was hit.
        max: u32,
    },
    /// A parent ID does not name an earlier part.
    BadParent {
        /// The offending part.
        part: PartId,
    },
    /// A part spans no bytes, or ends before it starts.
    EmptyPart {
        /// The offending part.
        part: PartId,
    },
    /// A part extends past the end of the input.
    OutOfBounds {
        /// The offending part.
        part: PartId,
    },
    /// A part lies (partly) outside its parent, or a body lies outside its part.
    OutsideParent {
        /// The offending part.
        part: PartId,
    },
    /// Two siblings overlap.
    Overlap {
        /// The earlier sibling.
        first: PartId,
        /// The sibling that starts inside it.
        second: PartId,
    },
    /// Bytes at the top level, or inside a declared body, that no part covers.
    Uncovered {
        /// The container whose body has the hole, or `None` for the top level.
        parent: Option<PartId>,
        /// The uncovered bytes.
        range: Range<u64>,
    },
}

impl fmt::Display for InventoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyParts { max } => write!(f, "inventory exceeds {max} parts"),
            Self::BadParent { part } => {
                write!(
                    f,
                    "part {} names a parent that is not an earlier part",
                    part.0
                )
            }
            Self::EmptyPart { part } => write!(f, "part {} spans no bytes", part.0),
            Self::OutOfBounds { part } => write!(f, "part {} extends past the input", part.0),
            Self::OutsideParent { part } => write!(f, "part {} lies outside its parent", part.0),
            Self::Overlap { first, second } => {
                write!(f, "parts {} and {} overlap", first.0, second.0)
            }
            Self::Uncovered {
                parent: None,
                range,
            } => {
                write!(
                    f,
                    "bytes {}..{} are not covered by any part",
                    range.start, range.end
                )
            }
            Self::Uncovered {
                parent: Some(p),
                range,
            } => write!(
                f,
                "bytes {}..{} inside the body of part {} are not covered",
                range.start, range.end, p.0
            ),
        }
    }
}

impl core::error::Error for InventoryError {}

/// A byte-exact map of a file's structural parts. See the
/// [module documentation](self).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inventory {
    format: ImageFormat,
    len: u64,
    parts: Vec<Part>,
    max_parts: u32,
    /// First child of each part (same indexing as `parts`), or `NONE`.
    first_child: Vec<u32>,
    /// Next sibling of each part, or `NONE`. Sibling lists run in reverse
    /// push order; readers sort them by offset.
    next_sibling: Vec<u32>,
    /// First top-level part, or `NONE`.
    first_top: u32,
}

/// End of a sibling list.
const NONE: u32 = u32::MAX;

impl Inventory {
    /// An empty inventory for an input of `len` bytes.
    pub fn new(format: ImageFormat, len: u64) -> Self {
        Self {
            format,
            len,
            parts: Vec::new(),
            max_parts: DEFAULT_MAX_PARTS,
            first_child: Vec::new(),
            next_sibling: Vec::new(),
            first_top: NONE,
        }
    }

    /// Change the part cap (default [`DEFAULT_MAX_PARTS`]).
    pub fn with_max_parts(mut self, max_parts: u32) -> Self {
        self.max_parts = max_parts;
        self
    }

    /// The format the inventory describes.
    pub fn format(&self) -> ImageFormat {
        self.format
    }

    /// Length of the input in bytes.
    pub fn input_len(&self) -> u64 {
        self.len
    }

    /// Every part, in the order it was recorded. A parent always precedes its
    /// children; siblings are not necessarily in file order (see
    /// [`children`](Self::children)).
    pub fn parts(&self) -> &[Part] {
        &self.parts
    }

    /// The part with this ID.
    pub fn get(&self, id: PartId) -> Option<&Part> {
        self.parts.get(id.index())
    }

    /// Record `part` under `parent` (`None` = top level) and return its ID.
    ///
    /// Fails when the part cap is reached, or when `parent` does not name a
    /// part recorded earlier. Coverage is checked by
    /// [`validate`](Self::validate), not here.
    pub fn push(
        &mut self,
        parent: Option<PartId>,
        mut part: Part,
    ) -> Result<PartId, InventoryError> {
        let id = PartId(self.parts.len() as u32);
        if self.parts.len() >= self.max_parts as usize {
            return Err(InventoryError::TooManyParts {
                max: self.max_parts,
            });
        }
        let next = match parent {
            Some(p) if p.index() >= self.parts.len() => {
                return Err(InventoryError::BadParent { part: id });
            }
            Some(p) => core::mem::replace(&mut self.first_child[p.index()], id.0),
            None => core::mem::replace(&mut self.first_top, id.0),
        };
        part.parent = parent;
        self.parts.push(part);
        self.first_child.push(NONE);
        self.next_sibling.push(next);
        Ok(id)
    }

    /// Change a recorded part's disposition, returning the previous one.
    ///
    /// For walkers that learn a part's fate only later: an earlier unit that a
    /// later duplicate overrides, a table that no frame ends up using. Ranges and
    /// parents can't change after `push`, so coverage stays as recorded.
    /// `None` when `id` names no part.
    pub fn set_disposition(&mut self, id: PartId, disposition: Disposition) -> Option<Disposition> {
        self.parts
            .get_mut(id.index())
            .map(|p| core::mem::replace(&mut p.disposition, disposition))
    }

    /// Replace a recorded part's detail (see [`set_disposition`](Self::set_disposition)).
    /// Returns `false` when `id` names no part.
    pub fn set_detail(&mut self, id: PartId, detail: impl Into<String>) -> bool {
        match self.parts.get_mut(id.index()) {
            Some(p) => {
                p.detail = Some(detail.into());
                true
            }
            None => false,
        }
    }

    /// Children of `parent` (`None` = top level), sorted by start offset.
    pub fn children(&self, parent: Option<PartId>) -> Vec<PartId> {
        let mut ids = Vec::new();
        self.children_into(parent, &mut ids);
        ids.into_iter().map(|i| PartId(i as u32)).collect()
    }

    /// Fill `out` with the indices of `parent`'s children, sorted by start
    /// offset (ties by recording order).
    fn children_into(&self, parent: Option<PartId>, out: &mut Vec<usize>) {
        out.clear();
        let mut next = match parent {
            None => self.first_top,
            Some(p) => match self.first_child.get(p.index()) {
                Some(&first) => first,
                None => return,
            },
        };
        while next != NONE {
            out.push(next as usize);
            next = self.next_sibling[next as usize];
        }
        out.sort_by_key(|&i| (self.parts[i].range.start, i));
    }

    /// Cover every byte of the top level (`parent` = `None`) or of `parent`'s
    /// body that no child covers, with [`Gap`](PartKind::Gap) parts of the
    /// given disposition.
    ///
    /// A walker calls this after recording the parts it understood: with
    /// [`Trailing`](Disposition::Trailing) for the top level,
    /// [`Unreferenced`](Disposition::Unreferenced) for slack inside a
    /// container. Does nothing for a part without a body. Overlapping
    /// children are left for [`validate`](Self::validate) to report.
    pub fn fill_gaps(
        &mut self,
        parent: Option<PartId>,
        disposition: Disposition,
    ) -> Result<(), InventoryError> {
        let span = match parent {
            None => 0..self.len,
            Some(p) => match self.parts.get(p.index()).and_then(|p| p.body.clone()) {
                Some(body) => body,
                None => return Ok(()),
            },
        };
        let mut holes = Vec::new();
        let mut cursor = span.start;
        for id in self.children(parent) {
            let r = &self.parts[id.index()].range;
            if r.start > cursor {
                holes.push(cursor..r.start.min(span.end));
            }
            cursor = cursor.max(r.end);
        }
        if cursor < span.end {
            holes.push(cursor..span.end);
        }
        for hole in holes {
            if hole.start < hole.end {
                self.push(
                    parent,
                    Part::new(PartKind::Gap, PartTag::None, hole, disposition),
                )?;
            }
        }
        Ok(())
    }

    /// Check the coverage guarantees:
    ///
    /// - every part spans at least one byte and lies within the input;
    /// - every parent is an earlier part, and every part lies within its parent;
    /// - siblings do not overlap;
    /// - the top-level parts tile `0..input_len()` exactly;
    /// - a part's children tile its [`body`](Part::body) exactly, when it has one.
    ///
    /// An empty input has a valid empty inventory.
    pub fn validate(&self) -> Result<(), InventoryError> {
        for (i, part) in self.parts.iter().enumerate() {
            let id = PartId(i as u32);
            if part.range.start >= part.range.end {
                return Err(InventoryError::EmptyPart { part: id });
            }
            if part.range.end > self.len {
                return Err(InventoryError::OutOfBounds { part: id });
            }
            if let Some(body) = &part.body
                && (body.start < part.range.start
                    || body.end > part.range.end
                    || body.start > body.end)
            {
                return Err(InventoryError::OutsideParent { part: id });
            }
            if let Some(p) = part.parent {
                let pi = p.index();
                if pi >= i {
                    return Err(InventoryError::BadParent { part: id });
                }
                let pr = &self.parts[pi].range;
                if part.range.start < pr.start || part.range.end > pr.end {
                    return Err(InventoryError::OutsideParent { part: id });
                }
            }
        }
        let mut level = Vec::new();
        self.children_into(None, &mut level);
        self.check_level(None, &level, Some(0..self.len))?;
        for (i, part) in self.parts.iter().enumerate() {
            if self.first_child[i] == NONE && part.body.is_none() {
                continue;
            }
            let id = PartId(i as u32);
            self.children_into(Some(id), &mut level);
            self.check_level(Some(id), &level, part.body.clone())?;
        }
        Ok(())
    }

    /// Siblings must not overlap; with a `span`, they must tile it exactly.
    fn check_level(
        &self,
        parent: Option<PartId>,
        ids: &[usize],
        span: Option<Range<u64>>,
    ) -> Result<(), InventoryError> {
        for pair in ids.windows(2) {
            if self.parts[pair[1]].range.start < self.parts[pair[0]].range.end {
                return Err(InventoryError::Overlap {
                    first: PartId(pair[0] as u32),
                    second: PartId(pair[1] as u32),
                });
            }
        }
        let Some(span) = span else {
            return Ok(());
        };
        let mut cursor = span.start;
        for &i in ids.iter() {
            let r = &self.parts[i].range;
            if r.start < span.start || r.end > span.end {
                return Err(InventoryError::OutsideParent {
                    part: PartId(i as u32),
                });
            }
            if r.start > cursor {
                return Err(InventoryError::Uncovered {
                    parent,
                    range: cursor..r.start,
                });
            }
            cursor = r.end;
        }
        if cursor < span.end {
            return Err(InventoryError::Uncovered {
                parent,
                range: cursor..span.end,
            });
        }
        Ok(())
    }

    /// Parts whose bytes the decoder does not consume (see
    /// [`Disposition::is_consumed`]), in recorded order.
    pub fn unconsumed(&self) -> impl Iterator<Item = (PartId, &Part)> {
        self.parts
            .iter()
            .enumerate()
            .filter(|(_, p)| !p.disposition.is_consumed())
            .map(|(i, p)| (PartId(i as u32), p))
    }

    /// Bytes per disposition name, counting only parts without children
    /// (leaves), so nested bytes are counted once.
    pub fn bytes_by_disposition(&self) -> Vec<(&'static str, u64)> {
        let mut totals: Vec<(&'static str, u64)> = Vec::new();
        for (i, p) in self.parts.iter().enumerate() {
            if self.first_child[i] != NONE {
                continue;
            }
            let name = p.disposition.name();
            match totals.iter_mut().find(|(n, _)| *n == name) {
                Some((_, bytes)) => *bytes += p.len(),
                None => totals.push((name, p.len())),
            }
        }
        totals
    }

    fn write_level(
        &self,
        f: &mut fmt::Formatter<'_>,
        parent: Option<PartId>,
        depth: usize,
    ) -> fmt::Result {
        for id in self.children(parent) {
            let p = &self.parts[id.index()];
            let tag = p.tag.to_string();
            let disposition = p.disposition.to_string();
            write!(
                f,
                "{:>10} {:>10}  {:width$}{:<14} {:<10} {:<20}",
                p.range.start,
                p.len(),
                "",
                p.kind.name(),
                tag,
                disposition,
                width = depth * 2
            )?;
            if let Some(label) = &p.label {
                write!(f, " {label:?}")?;
            }
            if let Some(detail) = &p.detail {
                write!(f, " ({detail})")?;
            }
            writeln!(f)?;
            self.write_level(f, Some(id), depth + 1)?;
        }
        Ok(())
    }
}

/// A table of every part in file order, one per line, indented by depth:
/// offset, length, kind, tag, disposition, label and detail.
impl fmt::Display for Inventory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "{} inventory, {} bytes, {} parts",
            self.format,
            self.len,
            self.parts.len()
        )?;
        writeln!(
            f,
            "{:>10} {:>10}  {:<14} {:<10} {:<20} label",
            "offset", "length", "kind", "tag", "disposition"
        )?;
        self.write_level(f, None, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;

    fn part(kind: PartKind, range: Range<u64>, d: Disposition) -> Part {
        Part::new(kind, PartTag::None, range, d)
    }

    #[test]
    fn empty_input_is_valid() {
        let inv = Inventory::new(ImageFormat::Png, 0);
        assert_eq!(inv.validate(), Ok(()));
        assert_eq!(inv.unconsumed().count(), 0);
    }

    #[test]
    fn top_level_must_tile_the_input() {
        let mut inv = Inventory::new(ImageFormat::Png, 20);
        inv.push(None, part(PartKind::Header, 0..8, Disposition::Structure))
            .unwrap();
        inv.push(None, part(PartKind::Chunk, 10..20, Disposition::ImageData))
            .unwrap();
        assert_eq!(
            inv.validate(),
            Err(InventoryError::Uncovered {
                parent: None,
                range: 8..10
            })
        );
        inv.fill_gaps(None, Disposition::Unreferenced).unwrap();
        assert_eq!(inv.validate(), Ok(()));
        let gap = inv.parts().last().unwrap();
        assert_eq!((gap.kind, gap.range.clone()), (PartKind::Gap, 8..10));
    }

    #[test]
    fn trailing_bytes_become_a_trailing_gap() {
        let mut inv = Inventory::new(ImageFormat::Jpeg, 100);
        inv.push(None, part(PartKind::Segment, 0..90, Disposition::Structure))
            .unwrap();
        inv.fill_gaps(None, Disposition::Trailing).unwrap();
        inv.validate().unwrap();
        let tail: Vec<_> = inv.unconsumed().collect();
        assert_eq!(tail.len(), 1);
        assert_eq!(tail[0].1.range, 90..100);
        assert_eq!(tail[0].1.disposition, Disposition::Trailing);
    }

    #[test]
    fn overlapping_siblings_are_rejected() {
        let mut inv = Inventory::new(ImageFormat::Png, 20);
        inv.push(None, part(PartKind::Chunk, 0..12, Disposition::Structure))
            .unwrap();
        inv.push(None, part(PartKind::Chunk, 10..20, Disposition::Structure))
            .unwrap();
        assert_eq!(
            inv.validate(),
            Err(InventoryError::Overlap {
                first: PartId(0),
                second: PartId(1)
            })
        );
    }

    #[test]
    fn declared_bodies_must_be_tiled_by_children() {
        let mut inv = Inventory::new(ImageFormat::Avif, 40);
        let mdat = inv
            .push(
                None,
                part(PartKind::Box, 0..40, Disposition::Structure).with_body(8..40),
            )
            .unwrap();
        inv.push(
            Some(mdat),
            part(PartKind::Extent, 8..30, Disposition::ImageData),
        )
        .unwrap();
        assert_eq!(
            inv.validate(),
            Err(InventoryError::Uncovered {
                parent: Some(mdat),
                range: 30..40
            })
        );
        inv.fill_gaps(Some(mdat), Disposition::Unreferenced)
            .unwrap();
        inv.validate().unwrap();
        // Without a body, a container may report its content as a whole.
        let mut inv = Inventory::new(ImageFormat::Avif, 40);
        let meta = inv
            .push(None, part(PartKind::Box, 0..40, Disposition::Structure))
            .unwrap();
        inv.push(
            Some(meta),
            part(PartKind::Box, 12..20, Disposition::Structure),
        )
        .unwrap();
        inv.validate().unwrap();
    }

    #[test]
    fn children_must_stay_inside_their_parent() {
        let mut inv = Inventory::new(ImageFormat::WebP, 30);
        let riff = inv
            .push(None, part(PartKind::Header, 0..20, Disposition::Structure))
            .unwrap();
        inv.push(None, part(PartKind::Trailer, 20..30, Disposition::Trailing))
            .unwrap();
        let bad = inv
            .push(
                Some(riff),
                part(PartKind::Chunk, 12..25, Disposition::Unknown),
            )
            .unwrap();
        assert_eq!(
            inv.validate(),
            Err(InventoryError::OutsideParent { part: bad })
        );
    }

    #[test]
    fn empty_and_out_of_bounds_parts_are_rejected() {
        let mut inv = Inventory::new(ImageFormat::Gif, 10);
        let p = inv
            .push(None, part(PartKind::Block, 4..4, Disposition::Structure))
            .unwrap();
        assert_eq!(inv.validate(), Err(InventoryError::EmptyPart { part: p }));
        let mut inv = Inventory::new(ImageFormat::Gif, 10);
        let p = inv
            .push(None, part(PartKind::Block, 0..11, Disposition::Structure))
            .unwrap();
        assert_eq!(inv.validate(), Err(InventoryError::OutOfBounds { part: p }));
    }

    #[test]
    fn a_parent_must_precede_its_children() {
        let mut inv = Inventory::new(ImageFormat::Png, 10);
        assert_eq!(
            inv.push(
                Some(PartId(5)),
                part(PartKind::Chunk, 0..10, Disposition::Structure),
            ),
            Err(InventoryError::BadParent { part: PartId(0) })
        );
        assert!(inv.parts().is_empty());
    }

    /// Many sibling containers each filling their own gaps stays linear in the
    /// number of parts per container, not quadratic in the whole inventory.
    #[test]
    fn gap_filling_scales_with_siblings_not_inventory_size() {
        let n: u64 = 20_000;
        let mut inv = Inventory::new(ImageFormat::Avif, n * 16);
        for i in 0..n {
            let start = i * 16;
            let b = inv
                .push(
                    None,
                    part(PartKind::Box, start..start + 16, Disposition::Structure)
                        .with_body(start + 8..start + 16),
                )
                .unwrap();
            inv.push(
                Some(b),
                part(PartKind::Box, start + 8..start + 12, Disposition::Unknown),
            )
            .unwrap();
            inv.fill_gaps(Some(b), Disposition::Unreferenced).unwrap();
        }
        inv.validate().unwrap();
        assert_eq!(inv.parts().len() as u64, n * 3);
    }

    #[test]
    fn a_later_duplicate_can_downgrade_an_earlier_part() {
        let mut inv = Inventory::new(ImageFormat::Gif, 20);
        let first = inv
            .push(
                None,
                part(
                    PartKind::Block,
                    0..10,
                    Disposition::Metadata(MetadataKind::Animation),
                ),
            )
            .unwrap();
        inv.push(
            None,
            part(
                PartKind::Block,
                10..20,
                Disposition::Metadata(MetadataKind::Animation),
            ),
        )
        .unwrap();
        assert_eq!(
            inv.set_disposition(first, Disposition::Dropped),
            Some(Disposition::Metadata(MetadataKind::Animation))
        );
        assert!(inv.set_detail(first, "overwritten by the block at 10"));
        assert_eq!(inv.unconsumed().count(), 1);
        assert_eq!(inv.set_disposition(PartId(9), Disposition::Dropped), None);
        assert!(!inv.set_detail(PartId(9), "x"));
        inv.validate().unwrap();
    }

    #[test]
    fn the_part_cap_is_enforced() {
        let mut inv = Inventory::new(ImageFormat::Png, 10).with_max_parts(2);
        inv.push(None, part(PartKind::Chunk, 0..5, Disposition::Structure))
            .unwrap();
        inv.push(None, part(PartKind::Chunk, 5..10, Disposition::Structure))
            .unwrap();
        assert_eq!(
            inv.push(None, part(PartKind::Chunk, 9..10, Disposition::Structure)),
            Err(InventoryError::TooManyParts { max: 2 })
        );
    }

    #[test]
    fn unconsumed_and_byte_totals_count_leaves() {
        let mut inv = Inventory::new(ImageFormat::Jpeg, 100);
        let app = inv
            .push(
                None,
                Part::new(
                    PartKind::Segment,
                    PartTag::Marker(0xED),
                    0..60,
                    Disposition::Skipped,
                )
                .with_label("Photoshop 3.0")
                .with_body(18..60),
            )
            .unwrap();
        inv.push(
            Some(app),
            part(PartKind::Block, 18..60, Disposition::Skipped),
        )
        .unwrap();
        inv.push(
            None,
            part(PartKind::ScanData, 60..100, Disposition::ImageData),
        )
        .unwrap();
        inv.validate().unwrap();
        assert_eq!(inv.unconsumed().count(), 2);
        let totals = inv.bytes_by_disposition();
        assert!(totals.contains(&("skipped", 42)));
        assert!(totals.contains(&("image-data", 40)));
    }

    #[test]
    fn tags_and_dispositions_print_compactly() {
        assert_eq!(PartTag::Marker(0xE1).to_string(), "FFE1");
        assert_eq!(PartTag::FourCc(*b"iCCP").to_string(), "iCCP");
        assert_eq!(PartTag::FourCc([0, 1, 2, 3]).to_string(), "0x00010203");
        assert_eq!(PartTag::Code(274).to_string(), "0x112");
        assert_eq!(
            Disposition::Metadata(MetadataKind::Exif).to_string(),
            "metadata(exif)"
        );
        assert!(!Disposition::Padding.is_consumed());
        assert!(Disposition::Metadata(MetadataKind::Icc).is_consumed());
    }

    #[test]
    fn display_lists_parts_in_file_order_with_nesting() {
        let mut inv = Inventory::new(ImageFormat::Avif, 40);
        // Recorded out of order: the table still prints by offset.
        let mdat = inv
            .push(
                None,
                Part::new(
                    PartKind::Box,
                    PartTag::FourCc(*b"mdat"),
                    20..40,
                    Disposition::Structure,
                )
                .with_body(28..40),
            )
            .unwrap();
        inv.push(
            None,
            Part::new(
                PartKind::Box,
                PartTag::FourCc(*b"ftyp"),
                0..20,
                Disposition::Structure,
            ),
        )
        .unwrap();
        inv.push(
            Some(mdat),
            Part::new(
                PartKind::Extent,
                PartTag::Code(1),
                28..40,
                Disposition::ImageData,
            ),
        )
        .unwrap();
        inv.validate().unwrap();
        let table = format!("{inv}");
        let ftyp = table.find("ftyp").unwrap();
        let mdat_pos = table.find("mdat").unwrap();
        assert!(ftyp < mdat_pos, "{table}");
        assert!(table.contains("    extent"), "{table}");
    }
}
