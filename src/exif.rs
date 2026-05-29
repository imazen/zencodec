//! Structured, borrowing EXIF/TIFF model: parse → inspect/prune → serialize.
//!
//! [`Exif::parse`] reads a TIFF/EXIF blob into a tree of IFDs whose entry
//! values *borrow* the source bytes (zero-copy — a multi-KB thumbnail is never
//! copied during parsing or pruning). [`Exif::filtered`] prunes the tree by
//! [`ExifPolicy`] category, and [`Exif::to_bytes`] re-serializes a valid TIFF,
//! recomputing all offsets. [`retain`] is the `Cow` convenience used by
//! [`Metadata::filtered`](crate::Metadata::filtered): it borrows the source
//! unchanged when nothing is dropped and allocates only on a real rewrite.
//!
//! Spec: TIFF 6.0 (Adobe, 1992) + EXIF 2.32 (CIPA DC-008). The structural
//! pointer tags — Exif IFD (0x8769), GPS IFD (0x8825), and the JPEG thumbnail
//! pointers (0x0201/0x0202) — are modeled as tree edges, not entries, and
//! re-synthesized with fresh offsets on serialize.
//!
//! Error model (no panics on untrusted input — every read is bounds-checked):
//! - **Structural failure → `None`.** A bad byte-order mark, wrong magic,
//!   IFD0 offset past EOF, or an over-cap entry count (`MAX_IFD_ENTRIES`) makes
//!   `Exif::parse` return `None`.
//! - **Graceful per-entry degradation.** A single unreadable, unknown-type, or
//!   out-of-bounds entry is *skipped*, and a truncated entry table salvages the
//!   entries read so far — one malformed (or future-typed) entry never discards
//!   the rest of the IFD's metadata. Skipped entries are dropped on a rewrite.
//! - **Fail-safe filtering.** [`retain`] drops EXIF it can't parse under a
//!   stripping policy (rather than passing it through and risking a leak); see
//!   its docs.
//!
//! Known limitation: rewriting (any partial prune) relocates the `MakerNote`
//! blob (0x927C, the `camera` category), whose maker-specific *internal*
//! offsets cannot always be fixed up. Pipelines needing byte-exact MakerNote
//! should keep all EXIF (no prune), in which case the source passes through
//! untouched. Uncompressed (StripOffsets) thumbnails are dropped-only — kept
//! correctly only in the no-prune passthrough.

use alloc::borrow::Cow;
use alloc::vec::Vec;
use zenpixels::Orientation;

const TAG_ORIENTATION: u16 = 0x0112;
const TAG_COPYRIGHT: u16 = 0x8298;
const TAG_ARTIST: u16 = 0x013B;
const TAG_EXIF_IFD: u16 = 0x8769;
const TAG_GPS_IFD: u16 = 0x8825;
const TAG_INTEROP_IFD: u16 = 0xA005;
const TAG_THUMB_OFFSET: u16 = 0x0201; // JPEGInterchangeFormat
const TAG_THUMB_LENGTH: u16 = 0x0202; // JPEGInterchangeFormatLength

const TIFF_ASCII: u16 = 2;
const TIFF_SHORT: u16 = 3;
const TIFF_LONG: u16 = 4;
/// Exif 2.32 type 129 = UTF-8 string (8-bit bytes, NUL-terminated, count
/// includes the NUL). The spec-conformant way to store Unicode in an IFD field.
const TIFF_UTF8: u16 = 129;
const TIFF_HEADER_SIZE: usize = 8;
const MAX_IFD_ENTRIES: u16 = 1000;
const EXIF_PREFIX: &[u8] = b"Exif\0\0";

/// TIFF byte order, preserved across a parse → serialize round-trip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByteOrder {
    /// Little-endian (`II`, Intel).
    Little,
    /// Big-endian (`MM`, Motorola).
    Big,
}

/// EXIF category an IFD0/Exif-IFD entry belongs to. GPS and thumbnail are
/// modeled structurally (whole sub-IFD), not per-entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Category {
    Orientation,
    Rights,
    Datetime,
    Camera,
    Other,
}

fn classify(tag: u16) -> Category {
    match tag {
        TAG_ORIENTATION => Category::Orientation,
        // Attribution / rights-holder. Copyright (the rights *notice*), Artist
        // (creator), plus the Exif-IFD creator/owner *name* tags
        // (CameraOwnerName 0xA430, Photographer 0xA437, ImageEditor 0xA438) —
        // the spec says Artist mirrors one of these, so they're the same
        // "who made / holds rights" class a copyright-preserving policy keeps.
        TAG_COPYRIGHT | TAG_ARTIST | 0xA430 | 0xA437 | 0xA438 => Category::Rights,
        // DateTime, DateTimeOriginal/Digitized, sub-sec + offset-time variants.
        0x0132 | 0x9003 | 0x9004 | 0x9010 | 0x9011 | 0x9012 | 0x9290 | 0x9291 | 0x9292 => {
            Category::Datetime
        }
        // Device / software identity: Make, Model, Software, HostComputer,
        // MakerNote, body/lens serials + lens make/model, ImageUniqueID, and the
        // firmware / developing / editing software tags.
        0x010F | 0x0110 | 0x0131 | 0x013C | 0x927C | 0xA420 | 0xA431 | 0xA432 | 0xA433 | 0xA434
        | 0xA435 | 0xA439 | 0xA43A | 0xA43B | 0xA43C => Category::Camera,
        _ => Category::Other,
    }
}

/// One IFD entry, borrowing its value bytes from the source blob.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Entry<'a> {
    tag: u16,
    kind: u16,
    count: u32,
    /// Resolved value bytes (`count × type_size`), in source byte order.
    value: &'a [u8],
}

/// A parsed EXIF/TIFF tree borrowing from the source bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exif<'a> {
    order: ByteOrder,
    had_prefix: bool,
    ifd0: Vec<Entry<'a>>,
    exif_ifd: Option<Vec<Entry<'a>>>,
    gps_ifd: Option<Vec<Entry<'a>>>,
    ifd1: Option<Vec<Entry<'a>>>,
    thumbnail: Option<&'a [u8]>,
}

/// TIFF/Exif type size in bytes, or `None` for an unknown type.
fn type_size(kind: u16) -> Option<usize> {
    Some(match kind {
        1 | 2 | 6 | 7 | 129 => 1, // BYTE, ASCII, SBYTE, UNDEFINED, UTF-8 (Exif 2.32)
        3 | 8 => 2,               // SHORT, SSHORT
        4 | 9 | 11 | 13 => 4,     // LONG, SLONG, FLOAT, IFD
        5 | 10 | 12 => 8,         // RATIONAL, SRATIONAL, DOUBLE
        _ => return None,
    })
}

fn rd16(d: &[u8], o: usize, order: ByteOrder) -> Option<u16> {
    let b = d.get(o..o + 2)?;
    Some(match order {
        ByteOrder::Big => u16::from_be_bytes([b[0], b[1]]),
        ByteOrder::Little => u16::from_le_bytes([b[0], b[1]]),
    })
}

fn rd32(d: &[u8], o: usize, order: ByteOrder) -> Option<u32> {
    let b = d.get(o..o + 4)?;
    Some(match order {
        ByteOrder::Big => u32::from_be_bytes([b[0], b[1], b[2], b[3]]),
        ByteOrder::Little => u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
    })
}

/// Parse one IFD at `off`, resolving each entry's value slice. Returns the
/// entries and the next-IFD offset (0 = none). Structural pointer tags are
/// left in place for the caller to extract.
fn parse_ifd(tiff: &[u8], off: usize, order: ByteOrder) -> Option<(Vec<Entry<'_>>, u32)> {
    let count = rd16(tiff, off, order)?;
    if count > MAX_IFD_ENTRIES {
        return None; // DoS cap — reject the whole IFD
    }
    let entries_start = off.checked_add(2)?;
    let mut entries = Vec::new();
    for i in 0..count as usize {
        // Graceful degradation: a truncated entry table salvages the entries
        // read so far (stop), and an individual unreadable/unknown-type/
        // out-of-bounds entry is skipped — one malformed or future-typed entry
        // doesn't discard all of the IFD's metadata.
        let Some(e) = i.checked_mul(12).and_then(|o| entries_start.checked_add(o)) else {
            break;
        };
        if e.checked_add(12).is_none_or(|end| end > tiff.len()) {
            break; // truncated table
        }
        if let Some(entry) = resolve_entry(tiff, e, order) {
            entries.push(entry);
        }
    }
    // The next-IFD offset is structurally required, but tolerate a blob that
    // ends right after the last entry (treat as "no further IFD").
    let next = rd32(tiff, entries_start.checked_add(count as usize * 12)?, order).unwrap_or(0);
    Some((entries, next))
}

/// Read one 12-byte IFD entry at `e` (assumed within bounds) and resolve its
/// value slice. `None` for an unknown TIFF type, an overflowing
/// `count × type_size`, or an out-of-bounds out-of-line value — the caller
/// skips such an entry rather than failing the whole IFD.
fn resolve_entry(tiff: &[u8], e: usize, order: ByteOrder) -> Option<Entry<'_>> {
    let tag = rd16(tiff, e, order)?;
    let kind = rd16(tiff, e + 2, order)?;
    let cnt = rd32(tiff, e + 4, order)?;
    let tsize = type_size(kind)?;
    let byte_len = (cnt as usize).checked_mul(tsize)?;
    let value = if byte_len <= 4 {
        tiff.get(e + 8..e + 8 + byte_len)?
    } else {
        let voff = rd32(tiff, e + 8, order)? as usize;
        tiff.get(voff..voff.checked_add(byte_len)?)?
    };
    Some(Entry {
        tag,
        kind,
        count: cnt,
        value,
    })
}

/// Extract a structural pointer tag's offset, removing it from `entries`.
fn take_pointer(entries: &mut Vec<Entry<'_>>, tag: u16, order: ByteOrder) -> Option<usize> {
    let pos = entries.iter().position(|e| e.tag == tag)?;
    // Peek before removing: a malformed pointer with a < 4-byte value isn't a
    // usable offset, so leave it in place (it round-trips as a normal entry)
    // rather than silently dropping the whole sub-IFD it nominally points at.
    let b = entries[pos].value.get(0..4)?;
    let off = match order {
        ByteOrder::Big => u32::from_be_bytes([b[0], b[1], b[2], b[3]]),
        ByteOrder::Little => u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
    } as usize;
    entries.remove(pos);
    Some(off)
}

impl<'a> Exif<'a> {
    /// Parse a TIFF/EXIF blob (optionally `Exif\0\0`-prefixed). Returns `None`
    /// for malformed input. Zero-copy: entry values borrow `data`.
    pub fn parse(data: &'a [u8]) -> Option<Self> {
        let had_prefix =
            data.len() >= EXIF_PREFIX.len() && data[..EXIF_PREFIX.len()] == *EXIF_PREFIX;
        let tiff = if had_prefix {
            &data[EXIF_PREFIX.len()..]
        } else {
            data
        };
        if tiff.len() < TIFF_HEADER_SIZE {
            return None;
        }
        let order = match [tiff[0], tiff[1]] {
            [b'M', b'M'] => ByteOrder::Big,
            [b'I', b'I'] => ByteOrder::Little,
            _ => return None,
        };
        if rd16(tiff, 2, order)? != 42 {
            return None;
        }
        let ifd0_off = rd32(tiff, 4, order)? as usize;
        let (mut ifd0, next) = parse_ifd(tiff, ifd0_off, order)?;

        // Extract sub-IFD pointers as tree edges. The Interop IFD (0xA005) is
        // not modeled — strip its pointer so a rewrite can't leave a dangling
        // offset (it survives only via the no-prune passthrough).
        let exif_ifd = take_pointer(&mut ifd0, TAG_EXIF_IFD, order).and_then(|o| {
            parse_ifd(tiff, o, order).map(|(mut e, _)| {
                take_pointer(&mut e, TAG_INTEROP_IFD, order);
                e
            })
        });
        let gps_ifd = take_pointer(&mut ifd0, TAG_GPS_IFD, order)
            .and_then(|o| parse_ifd(tiff, o, order).map(|(e, _)| e));

        // IFD1 (thumbnail directory) follows IFD0's next pointer. The JPEG
        // thumbnail offset (0x0201) and length (0x0202) are peeked first and
        // only removed once the thumbnail is actually captured — so a thumbnail
        // whose length is encoded as SHORT (spec-permitted, common in real
        // cameras) is preserved, not silently dropped, and a malformed pair
        // round-trips as ordinary entries instead of vanishing.
        let (mut ifd1, mut thumbnail) = (None, None);
        if next != 0
            && let Some((mut entries, _)) = parse_ifd(tiff, next as usize, order)
        {
            let toff = entries
                .iter()
                .find(|e| e.tag == TAG_THUMB_OFFSET)
                .and_then(|e| read_uint(e, order));
            let tlen = entries
                .iter()
                .find(|e| e.tag == TAG_THUMB_LENGTH)
                .and_then(|e| read_uint(e, order));
            if let (Some(o), Some(l)) = (toff, tlen)
                && let Some(t) = tiff.get(o as usize..(o as usize).checked_add(l as usize)?)
            {
                thumbnail = Some(t);
                entries.retain(|e| e.tag != TAG_THUMB_OFFSET && e.tag != TAG_THUMB_LENGTH);
            }
            ifd1 = Some(entries);
        }

        Some(Exif {
            order,
            had_prefix,
            ifd0,
            exif_ifd,
            gps_ifd,
            ifd1,
            thumbnail,
        })
    }

    /// The EXIF Orientation tag (0x0112), if present and valid.
    pub fn orientation(&self) -> Option<Orientation> {
        let e = self.ifd0.iter().find(|e| e.tag == TAG_ORIENTATION)?;
        let raw = read_uint(e, self.order)?;
        Orientation::from_exif(u8::try_from(raw).ok()?)
    }

    /// The Copyright tag (0x8298) as text — a **lossy view** of
    /// [`copyright_bytes`](Self::copyright_bytes). See the [encoding
    /// note](#encoding).
    ///
    /// This is the copyright *notice* (rights statement). The rights-holder /
    /// creator *name* is a separate concept — the [`artist`](Self::artist) tag
    /// and the Exif-IFD CameraOwnerName / Photographer / ImageEditor tags (all
    /// in the [`rights`](ExifPolicy::rights) category). The Copyright field has
    /// historically held two NUL-separated segments (photographer copyright,
    /// then editor copyright); this returns the first segment. A second segment,
    /// if present, is preserved byte-for-byte on a rewrite but not surfaced
    /// separately.
    pub fn copyright(&self) -> Option<Cow<'a, str>> {
        ascii_value(&self.ifd0, TAG_COPYRIGHT)
    }

    /// The Artist tag (0x013B) as text — a **lossy view** of
    /// [`artist_bytes`](Self::artist_bytes). See the [encoding note](#encoding).
    pub fn artist(&self) -> Option<Cow<'a, str>> {
        ascii_value(&self.ifd0, TAG_ARTIST)
    }

    /// The raw Copyright (0x8298) value bytes, NUL-terminator stripped — the
    /// field exactly as stored, with no decoding.
    ///
    /// # Encoding
    ///
    /// Per Exif 2.32 / CIPA DC-008 (Table 6), Copyright and Artist may be stored
    /// as **ASCII (type 2, NUL-terminated 7-bit)** *or* **UTF-8 (type 129)** —
    /// UTF-8 is the spec-conformant way to carry Unicode in these fields. A
    /// type-2 field that nonetheless contains non-ASCII bytes (UTF-8 / Latin-1
    /// stuffed into an ASCII field — common in the wild) is the non-conformant
    /// case. zencodec reads both string types and
    /// [`copyright`](Self::copyright) / [`artist`](Self::artist) decode them as
    /// UTF-8 lossily (invalid sequences → U+FFFD) for a display string, while
    /// these `*_bytes` accessors return the exact bytes. zencodec never
    /// *generates* or transcodes these fields: a pruning rewrite preserves the
    /// value bytes **and TIFF type** verbatim, so a field is neither corrupted
    /// nor "corrected".
    ///
    /// Non-ASCII bytes in a type-2 field are **not** stripped: before type 129
    /// existed (Exif 2.32), the de-facto way to carry non-ASCII here was
    /// undeclared UTF-8, so decoding as UTF-8 recovers the common case —
    /// stripping the high bytes would corrupt it. A field that actually used a
    /// legacy code page (Latin-1, Shift-JIS) decodes lossily (→ U+FFFD); read
    /// `*_bytes` and apply your own decoder for those.
    pub fn copyright_bytes(&self) -> Option<&'a [u8]> {
        ascii_bytes(&self.ifd0, TAG_COPYRIGHT)
    }

    /// The raw Artist (0x013B) value bytes, NUL-terminator stripped. See
    /// [`copyright_bytes`](Self::copyright_bytes) for the encoding note.
    pub fn artist_bytes(&self) -> Option<&'a [u8]> {
        ascii_bytes(&self.ifd0, TAG_ARTIST)
    }

    /// Whether an embedded thumbnail is present.
    pub fn has_thumbnail(&self) -> bool {
        self.thumbnail.is_some()
    }

    /// Whether a GPS sub-IFD is present.
    pub fn has_gps(&self) -> bool {
        self.gps_ifd.is_some()
    }

    /// Prune the tree by `policy`, returning a new borrowing view. Surviving
    /// entries still borrow the original source (no payload copy).
    pub fn filtered(&self, policy: &ExifPolicy) -> Exif<'a> {
        let keep = |e: &&Entry<'a>| policy.keeps(classify(e.tag));
        let ifd0 = self.ifd0.iter().filter(keep).copied().collect();
        let exif_ifd = self
            .exif_ifd
            .as_ref()
            .map(|d| d.iter().filter(keep).copied().collect::<Vec<_>>())
            .filter(|d: &Vec<_>| !d.is_empty());
        let gps_ifd = match policy.gps {
            Retention::Keep => self.gps_ifd.clone(),
            Retention::Discard => None,
        };
        let (ifd1, thumbnail) = match policy.thumbnail {
            Retention::Keep => (self.ifd1.clone(), self.thumbnail),
            Retention::Discard => (None, None),
        };
        Exif {
            order: self.order,
            had_prefix: self.had_prefix,
            ifd0,
            exif_ifd,
            gps_ifd,
            ifd1,
            thumbnail,
        }
    }

    /// Serialize to a valid TIFF, recomputing every offset. Preserves the
    /// source byte order and `Exif\0\0` framing.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        if self.had_prefix {
            out.extend_from_slice(EXIF_PREFIX);
        }
        // All stored offsets are TIFF-relative (origin = the byte after any
        // `Exif\0\0` prefix), so they're computed from `TIFF_HEADER_SIZE`, not
        // from the prefixed buffer position.

        // Synthesized structural pointers per IFD (tag → kind always LONG).
        let ifd0_ptrs = {
            let mut v = Vec::new();
            if self.exif_ifd.is_some() {
                v.push(TAG_EXIF_IFD);
            }
            if self.gps_ifd.is_some() {
                v.push(TAG_GPS_IFD);
            }
            v
        };
        let ifd1_ptrs: &[u16] = if self.thumbnail.is_some() {
            &[TAG_THUMB_OFFSET, TAG_THUMB_LENGTH]
        } else {
            &[]
        };

        // Block sizes (entry table + ext data), pointers counted in the table.
        // `ext_size` is the *deduplicated* out-of-line size, matching what
        // `write_ifd` emits.
        let sz = |entries: &[Entry<'a>], nptr: usize| -> (usize, usize) {
            let table = 2 + 12 * (entries.len() + nptr) + 4;
            (table, ext_size(entries))
        };

        let (t0, x0) = sz(&self.ifd0, ifd0_ptrs.len());
        let mut cursor = TIFF_HEADER_SIZE + t0 + x0;
        let exif_off = self.exif_ifd.as_ref().map(|d| {
            let o = cursor;
            let (t, x) = sz(d, 0);
            cursor += t + x;
            o
        });
        let gps_off = self.gps_ifd.as_ref().map(|d| {
            let o = cursor;
            let (t, x) = sz(d, 0);
            cursor += t + x;
            o
        });
        let ifd1_off = self.ifd1.as_ref().map(|d| {
            let o = cursor;
            let (t, x) = sz(d, ifd1_ptrs.len());
            cursor += t + x;
            o
        });
        let thumb_off = self.thumbnail.map(|t| {
            let o = cursor;
            cursor += t.len();
            o
        });

        // Header.
        match self.order {
            ByteOrder::Little => out.extend_from_slice(b"II"),
            ByteOrder::Big => out.extend_from_slice(b"MM"),
        }
        self.put16(&mut out, 42);
        self.put32(&mut out, (TIFF_HEADER_SIZE) as u32);

        // IFD0 (with Exif/GPS pointers; next → IFD1).
        let mut ifd0_ptr_vals = Vec::new();
        if let Some(o) = exif_off {
            ifd0_ptr_vals.push((TAG_EXIF_IFD, o as u32));
        }
        if let Some(o) = gps_off {
            ifd0_ptr_vals.push((TAG_GPS_IFD, o as u32));
        }
        self.write_ifd(
            &mut out,
            &self.ifd0,
            &ifd0_ptr_vals,
            TIFF_HEADER_SIZE + t0,
            ifd1_off.unwrap_or(0) as u32,
        );

        if let Some(d) = &self.exif_ifd {
            let eb = exif_off.unwrap() + 2 + 12 * d.len() + 4;
            self.write_ifd(&mut out, d, &[], eb, 0);
        }
        if let Some(d) = &self.gps_ifd {
            let eb = gps_off.unwrap() + 2 + 12 * d.len() + 4;
            self.write_ifd(&mut out, d, &[], eb, 0);
        }
        if let Some(d) = &self.ifd1 {
            let mut pv = Vec::new();
            if let (Some(to), Some(t)) = (thumb_off, self.thumbnail) {
                pv.push((TAG_THUMB_OFFSET, to as u32));
                pv.push((TAG_THUMB_LENGTH, t.len() as u32));
            }
            let eb = ifd1_off.unwrap() + 2 + 12 * (d.len() + ifd1_ptrs.len()) + 4;
            self.write_ifd(&mut out, d, &pv, eb, 0);
        }
        if let Some(t) = self.thumbnail {
            out.extend_from_slice(t);
        }
        out
    }

    fn put16(&self, out: &mut Vec<u8>, v: u16) {
        match self.order {
            ByteOrder::Big => out.extend_from_slice(&v.to_be_bytes()),
            ByteOrder::Little => out.extend_from_slice(&v.to_le_bytes()),
        }
    }

    fn put32(&self, out: &mut Vec<u8>, v: u32) {
        match self.order {
            ByteOrder::Big => out.extend_from_slice(&v.to_be_bytes()),
            ByteOrder::Little => out.extend_from_slice(&v.to_le_bytes()),
        }
    }

    /// Write one IFD: `ptr_vals` are synthesized LONG entries (tag → value);
    /// `ext_base` is the TIFF-relative offset where this IFD's out-of-line
    /// values begin; `next` is the next-IFD offset.
    ///
    /// Entries are written tag-sorted, and out-of-line values are laid out in
    /// that **same** order (and deduplicated by alias). Matching the layout
    /// order to the write order makes `to_bytes` *canonical* — re-serializing
    /// its own output is a byte-exact fixpoint, so filtering is idempotent.
    fn write_ifd(
        &self,
        out: &mut Vec<u8>,
        entries: &[Entry<'a>],
        ptr_vals: &[(u16, u32)],
        ext_base: usize,
        next: u32,
    ) {
        // Merge real entries and synthesized pointers, sorted by tag.
        enum Item<'b, 'a> {
            Real(&'b Entry<'a>),
            Ptr(u16, u32),
        }
        let mut items: Vec<Item> = entries
            .iter()
            .map(Item::Real)
            .chain(ptr_vals.iter().map(|&(t, v)| Item::Ptr(t, v)))
            .collect();
        items.sort_by_key(|it| match it {
            Item::Real(e) => e.tag,
            Item::Ptr(t, _) => *t,
        });

        self.put16(out, items.len() as u16);
        // Lay out ext data in this (tag-sorted) write order, deduping aliases.
        let mut ext = Vec::new();
        let mut placed: Vec<(usize, usize, usize)> = Vec::new(); // (ptr, len, ext_off)
        for it in &items {
            match it {
                Item::Ptr(tag, val) => {
                    self.put16(out, *tag);
                    self.put16(out, TIFF_LONG);
                    self.put32(out, 1);
                    self.put32(out, *val);
                }
                Item::Real(e) => {
                    self.put16(out, e.tag);
                    self.put16(out, e.kind);
                    self.put32(out, e.count);
                    if e.value.len() <= 4 {
                        let mut v = [0u8; 4];
                        v[..e.value.len()].copy_from_slice(e.value);
                        out.extend_from_slice(&v);
                    } else {
                        let (ptr, len) = (e.value.as_ptr() as usize, e.value.len());
                        let off = if let Some(&(.., o)) =
                            placed.iter().find(|&&(p, l, _)| p == ptr && l == len)
                        {
                            o
                        } else {
                            let o = ext.len();
                            ext.extend_from_slice(e.value);
                            if ext.len() % 2 == 1 {
                                ext.push(0);
                            }
                            placed.push((ptr, len, o));
                            o
                        };
                        self.put32(out, (ext_base + off) as u32);
                    }
                }
            }
        }
        self.put32(out, next);
        out.extend_from_slice(&ext);
    }
}

/// Total out-of-line (>4-byte) byte size for one IFD, **deduplicating values
/// that alias the same source bytes** (so the count matches what `write_ifd`
/// emits). Order-independent — used by the layout pre-pass to size IFD blocks.
///
/// Dedup defends against a serializer memory-amplification DoS: a malformed
/// IFD can point hundreds of entries at one out-of-line blob (parse is
/// zero-copy, so they alias). Without dedup, `to_bytes` would copy that blob
/// once per entry — up to ~1000× blowup. With dedup the rewritten output is
/// bounded by the source size. Entries that merely have *equal content* at
/// *different* source locations are not merged (only true aliases are).
fn ext_size(entries: &[Entry<'_>]) -> usize {
    let mut total = 0usize;
    let mut placed: Vec<(usize, usize)> = Vec::new(); // (ptr, len)
    for e in entries {
        if e.value.len() <= 4 {
            continue;
        }
        let key = (e.value.as_ptr() as usize, e.value.len());
        if !placed.contains(&key) {
            total += align2(e.value.len());
            placed.push(key);
        }
    }
    total
}

fn align2(n: usize) -> usize {
    n + (n & 1)
}

/// Read a SHORT or LONG value as `u32` (for thumbnail length / offset).
fn read_uint(e: &Entry<'_>, order: ByteOrder) -> Option<u32> {
    match e.kind {
        TIFF_SHORT => rd16(e.value, 0, order).map(u32::from),
        TIFF_LONG => rd32(e.value, 0, order),
        _ => None,
    }
}

/// The raw bytes of a string-typed entry — ASCII (type 2) or UTF-8 (type 129,
/// Exif 2.32) — up to (not incl.) the first NUL. `None` if the tag is absent,
/// not a string type (a wrong-type field is ignored, not reinterpreted), or
/// empty.
fn ascii_bytes<'a>(entries: &[Entry<'a>], tag: u16) -> Option<&'a [u8]> {
    let e = entries.iter().find(|e| e.tag == tag)?;
    if e.kind != TIFF_ASCII && e.kind != TIFF_UTF8 {
        return None;
    }
    let end = e
        .value
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(e.value.len());
    let bytes = &e.value[..end];
    if bytes.is_empty() { None } else { Some(bytes) }
}

/// Lossy-UTF-8 *view* of [`ascii_bytes`]. EXIF type-2 is spec'd 7-bit ASCII;
/// real files embed UTF-8/Latin-1, so valid UTF-8 (incl. ASCII) is borrowed
/// and invalid sequences (e.g. raw Latin-1 `0xA9`) become U+FFFD (owned). The
/// result is always a valid `str`; it is a read-only view and is never written
/// back (the filter preserves the original bytes verbatim).
fn ascii_value<'a>(entries: &[Entry<'a>], tag: u16) -> Option<Cow<'a, str>> {
    let bytes = ascii_bytes(entries, tag)?;
    Some(match core::str::from_utf8(bytes) {
        Ok(s) => Cow::Borrowed(s),
        Err(_) => Cow::Owned(alloc::string::String::from_utf8_lossy(bytes).into_owned()),
    })
}

/// Keep-or-discard for a single metadata field. Explicit (no `bool`-direction
/// ambiguity).
///
/// `#[non_exhaustive]`: a future disposition (e.g. anonymize-in-place) can be
/// added without a breaking change. Query via [`keeps`](Self::keeps) /
/// [`discards`](Self::discards) rather than matching the variant, so callers
/// stay correct as variants are added.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Retention {
    /// Keep the field.
    Keep,
    /// Discard the field.
    Discard,
}

impl Retention {
    /// `true` if the field is kept.
    #[inline]
    #[must_use]
    pub const fn keeps(self) -> bool {
        matches!(self, Retention::Keep)
    }

    /// `true` if the field is dropped.
    #[inline]
    #[must_use]
    pub const fn discards(self) -> bool {
        matches!(self, Retention::Discard)
    }
}

/// Per-category EXIF retention. Categories not matched by a specific field
/// fall under [`other`](Self::other).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ExifPolicy {
    /// Orientation tag (0x0112).
    pub orientation: Retention,
    /// Rights: Copyright (0x8298) + Artist (0x013B).
    pub rights: Retention,
    /// Embedded thumbnail (IFD1 + its image data).
    pub thumbnail: Retention,
    /// GPS sub-IFD (location).
    pub gps: Retention,
    /// Capture timestamps (DateTime / Original / Digitized + sub-sec/offset).
    pub datetime: Retention,
    /// Camera/device identity (Make, Model, Software, lens, serial, MakerNote).
    pub camera: Retention,
    /// Everything else (dimensions, exposure settings, …).
    pub other: Retention,
}

impl ExifPolicy {
    /// Keep every category.
    pub const KEEP_ALL: Self = Self {
        orientation: Retention::Keep,
        rights: Retention::Keep,
        thumbnail: Retention::Keep,
        gps: Retention::Keep,
        datetime: Retention::Keep,
        camera: Retention::Keep,
        other: Retention::Keep,
    };
    /// Discard every category (drops EXIF entirely).
    pub const DISCARD_ALL: Self = Self {
        orientation: Retention::Discard,
        rights: Retention::Discard,
        thumbnail: Retention::Discard,
        gps: Retention::Discard,
        datetime: Retention::Discard,
        camera: Retention::Discard,
        other: Retention::Discard,
    };
    /// Keep only orientation + rights (the web default).
    pub const ATTRIBUTED_ORIENTATION: Self = Self {
        orientation: Retention::Keep,
        rights: Retention::Keep,
        ..Self::DISCARD_ALL
    };
    /// Keep only orientation.
    pub const ORIENTATION_ONLY: Self = Self {
        orientation: Retention::Keep,
        ..Self::DISCARD_ALL
    };

    /// Set the orientation category. (Builder — this type is `#[non_exhaustive]`,
    /// so downstream crates tweak it from a const via `with_*` rather than
    /// struct-update syntax.)
    #[must_use]
    pub const fn with_orientation(mut self, r: Retention) -> Self {
        self.orientation = r;
        self
    }
    /// Set the rights (copyright/artist) category.
    #[must_use]
    pub const fn with_rights(mut self, r: Retention) -> Self {
        self.rights = r;
        self
    }
    /// Set the thumbnail category.
    #[must_use]
    pub const fn with_thumbnail(mut self, r: Retention) -> Self {
        self.thumbnail = r;
        self
    }
    /// Set the GPS category.
    #[must_use]
    pub const fn with_gps(mut self, r: Retention) -> Self {
        self.gps = r;
        self
    }
    /// Set the timestamps category.
    #[must_use]
    pub const fn with_datetime(mut self, r: Retention) -> Self {
        self.datetime = r;
        self
    }
    /// Set the camera/device-identity category.
    #[must_use]
    pub const fn with_camera(mut self, r: Retention) -> Self {
        self.camera = r;
        self
    }
    /// Set the "everything else" category.
    #[must_use]
    pub const fn with_other(mut self, r: Retention) -> Self {
        self.other = r;
        self
    }

    fn keeps(&self, c: Category) -> bool {
        match c {
            Category::Orientation => self.orientation.keeps(),
            Category::Rights => self.rights.keeps(),
            Category::Datetime => self.datetime.keeps(),
            Category::Camera => self.camera.keeps(),
            Category::Other => self.other.keeps(),
        }
    }

    /// Whether every category is kept (→ source passes through unchanged).
    pub fn keeps_everything(&self) -> bool {
        self.orientation.keeps()
            && self.rights.keeps()
            && self.thumbnail.keeps()
            && self.gps.keeps()
            && self.datetime.keeps()
            && self.camera.keeps()
            && self.other.keeps()
    }

    /// Whether every category is discarded (→ EXIF dropped entirely).
    pub fn discards_everything(&self) -> bool {
        *self == Self::DISCARD_ALL
    }
}

/// Apply an [`ExifPolicy`] to a TIFF/EXIF blob, returning the retained bytes.
///
/// - Keep-everything policy → [`Cow::Borrowed`] (source unchanged, zero-copy).
/// - Partial policy on parseable EXIF → [`Cow::Owned`] rewrite (or `None` if
///   nothing survives).
/// - Discard-everything policy → `None`.
/// - **Unparseable EXIF under a stripping policy → `None` (fail-safe).** If the
///   blob can't be parsed, the requested strip can't be verified, so the EXIF
///   is dropped rather than passed through — a passthrough could leak GPS /
///   camera data a lenient viewer might still read. (Orientation is unaffected:
///   it's carried separately on [`Metadata`](crate::Metadata).) A
///   keep-everything policy never reaches this path.
pub fn retain<'a>(src: &'a [u8], policy: &ExifPolicy) -> Option<Cow<'a, [u8]>> {
    if policy.discards_everything() {
        return None;
    }
    if policy.keeps_everything() {
        return Some(Cow::Borrowed(src));
    }
    // A valid TIFF is ≤ 4 GiB (offsets are `u32`); a larger blob can't be
    // rewritten without risking offset truncation, and rewriting one is
    // pathological anyway. Pass it through untouched rather than risk a corrupt
    // output. (Rewrites are bounded by the source size — `ext_size` dedups
    // aliased values — so a parseable in-range blob is always safe.)
    if src.len() > u32::MAX as usize {
        return Some(Cow::Borrowed(src));
    }
    match Exif::parse(src) {
        Some(exif) => {
            let pruned = exif.filtered(policy);
            if pruned.ifd0.is_empty()
                && pruned.exif_ifd.is_none()
                && pruned.gps_ifd.is_none()
                && pruned.ifd1.is_none()
            {
                None
            } else {
                Some(Cow::Owned(pruned.to_bytes()))
            }
        }
        // Unparseable EXIF under a stripping policy: we can't verify the strip,
        // so fail safe and drop it (orientation survives on `Metadata`).
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn e(tag: u16, kind: u16, count: u32, value: &[u8]) -> Entry<'_> {
        Entry {
            tag,
            kind,
            count,
            value,
        }
    }

    /// A full tree: IFD0 (Make + Orientation + Copyright) + Exif-IFD
    /// (DateTimeOriginal) + GPS-IFD + IFD1 thumbnail. Built directly (the test
    /// module sees private fields) and serialized by `to_bytes`.
    fn sample(order: ByteOrder, had_prefix: bool) -> alloc::vec::Vec<u8> {
        let ori: alloc::vec::Vec<u8> = match order {
            ByteOrder::Little => 6u16.to_le_bytes().to_vec(),
            ByteOrder::Big => 6u16.to_be_bytes().to_vec(),
        };
        // Leak-free: build owned then serialize; borrows live within this fn.
        let exif = Exif {
            order,
            had_prefix,
            ifd0: vec![
                e(0x010F, 2, 4, b"Cam\0"),               // Make (camera)
                e(TAG_ORIENTATION, TIFF_SHORT, 1, &ori), // Orientation=Rotate90
                e(TAG_COPYRIGHT, 2, 7, b"(c) Me\0"),     // Copyright (out-of-line)
            ],
            exif_ifd: Some(vec![e(0x9003, 2, 5, b"2020\0")]), // DateTimeOriginal
            gps_ifd: Some(vec![e(0x0001, 2, 2, b"N\0")]),     // GPSLatitudeRef
            ifd1: Some(vec![]),
            thumbnail: Some(&[0xFF, 0xD8, 0xFF, 0xD9]),
        };
        exif.to_bytes()
    }

    #[test]
    fn round_trip_full_tree_little_endian() {
        let bytes = sample(ByteOrder::Little, false);
        let x = Exif::parse(&bytes).expect("parses");
        assert_eq!(x.order, ByteOrder::Little);
        assert_eq!(x.orientation(), Some(Orientation::Rotate90));
        assert_eq!(x.copyright().unwrap(), "(c) Me");
        assert!(x.has_gps());
        assert!(x.has_thumbnail());
        // Idempotent: re-serializing and re-parsing is stable.
        let bytes2 = x.to_bytes();
        let x2 = Exif::parse(&bytes2).expect("re-parses");
        assert_eq!(x2.orientation(), Some(Orientation::Rotate90));
        assert_eq!(x2.copyright().unwrap(), "(c) Me");
        assert!(x2.has_gps() && x2.has_thumbnail());
    }

    #[test]
    fn round_trip_big_endian() {
        let bytes = sample(ByteOrder::Big, false);
        let x = Exif::parse(&bytes).expect("parses");
        assert_eq!(x.order, ByteOrder::Big);
        assert_eq!(x.orientation(), Some(Orientation::Rotate90));
        assert_eq!(x.copyright().unwrap(), "(c) Me");
        assert!(x.has_gps() && x.has_thumbnail());
    }

    #[test]
    fn round_trip_with_exif_prefix_and_subifds() {
        // Exercises the offset fix: sub-IFD pointers must be TIFF-relative even
        // with an `Exif\0\0` prefix present.
        let bytes = sample(ByteOrder::Little, true);
        assert_eq!(&bytes[..6], b"Exif\0\0");
        let x = Exif::parse(&bytes).expect("parses");
        assert_eq!(x.orientation(), Some(Orientation::Rotate90));
        assert_eq!(x.copyright().unwrap(), "(c) Me");
        assert!(x.has_gps());
        assert!(x.has_thumbnail());
    }

    #[test]
    fn drop_gps_keeps_everything_else() {
        let bytes = sample(ByteOrder::Little, false);
        let x = Exif::parse(&bytes).unwrap();
        let p = ExifPolicy {
            gps: Retention::Discard,
            ..ExifPolicy::KEEP_ALL
        };
        let out = x.filtered(&p).to_bytes();
        let y = Exif::parse(&out).unwrap();
        assert!(!y.has_gps());
        assert!(y.has_thumbnail());
        assert_eq!(y.orientation(), Some(Orientation::Rotate90));
        assert_eq!(y.copyright().unwrap(), "(c) Me");
    }

    #[test]
    fn drop_thumbnail_keeps_everything_else() {
        let bytes = sample(ByteOrder::Little, false);
        let x = Exif::parse(&bytes).unwrap();
        let p = ExifPolicy {
            thumbnail: Retention::Discard,
            ..ExifPolicy::KEEP_ALL
        };
        let out = x.filtered(&p).to_bytes();
        let y = Exif::parse(&out).unwrap();
        assert!(!y.has_thumbnail());
        assert!(y.has_gps());
        assert_eq!(y.orientation(), Some(Orientation::Rotate90));
        assert_eq!(y.copyright().unwrap(), "(c) Me");
    }

    #[test]
    fn attributed_orientation_drops_camera_datetime_gps_thumbnail() {
        let bytes = sample(ByteOrder::Little, false);
        let x = Exif::parse(&bytes).unwrap();
        let out = x.filtered(&ExifPolicy::ATTRIBUTED_ORIENTATION).to_bytes();
        let y = Exif::parse(&out).unwrap();
        assert_eq!(y.orientation(), Some(Orientation::Rotate90));
        assert_eq!(y.copyright().unwrap(), "(c) Me");
        assert!(!y.has_gps());
        assert!(!y.has_thumbnail());
        // Camera (Make 0x010F) and DateTime (0x9003) gone.
        assert!(!out.windows(2).any(|w| w == 0x010Fu16.to_le_bytes()));
        assert!(!out.windows(2).any(|w| w == 0x9003u16.to_le_bytes()));
    }

    #[test]
    fn retain_cow_behaviour() {
        let bytes = sample(ByteOrder::Little, false);
        // Keep everything → borrows source (zero-copy).
        assert!(matches!(
            retain(&bytes, &ExifPolicy::KEEP_ALL),
            Some(Cow::Borrowed(_))
        ));
        // Prune → owns a rewritten buffer.
        let p = ExifPolicy {
            gps: Retention::Discard,
            ..ExifPolicy::KEEP_ALL
        };
        assert!(matches!(retain(&bytes, &p), Some(Cow::Owned(_))));
        // Discard everything → None.
        assert!(retain(&bytes, &ExifPolicy::DISCARD_ALL).is_none());
    }

    #[test]
    fn out_of_line_copyright_relocates_correctly() {
        // A long copyright forces out-of-line storage; after a prune that
        // shifts layout, it must still resolve.
        let long = b"Copyright 2026 Lilith, all rights reserved worldwide.\0";
        let exif = Exif {
            order: ByteOrder::Little,
            had_prefix: false,
            ifd0: vec![
                e(0x010F, 2, 4, b"Cam\0"),
                e(TAG_COPYRIGHT, 2, long.len() as u32, long),
            ],
            exif_ifd: None,
            gps_ifd: None,
            ifd1: None,
            thumbnail: None,
        };
        let pruned = exif.filtered(&ExifPolicy {
            camera: Retention::Discard,
            ..ExifPolicy::KEEP_ALL
        });
        let out = pruned.to_bytes();
        let y = Exif::parse(&out).unwrap();
        assert_eq!(
            y.copyright().unwrap(),
            "Copyright 2026 Lilith, all rights reserved worldwide."
        );
    }

    #[test]
    fn malformed_inputs_return_none() {
        assert!(Exif::parse(b"").is_none());
        assert!(Exif::parse(b"garbage").is_none());
        assert!(Exif::parse(&[0u8; 7]).is_none());
        // Good header, IFD0 offset past EOF.
        let mut bad = b"II".to_vec();
        bad.extend_from_slice(&42u16.to_le_bytes());
        bad.extend_from_slice(&9999u32.to_le_bytes());
        assert!(Exif::parse(&bad).is_none());
    }

    #[test]
    fn excessive_entry_count_rejected() {
        let mut bad = b"II".to_vec();
        bad.extend_from_slice(&42u16.to_le_bytes());
        bad.extend_from_slice(&8u32.to_le_bytes());
        bad.extend_from_slice(&60000u16.to_le_bytes()); // > MAX_IFD_ENTRIES
        assert!(Exif::parse(&bad).is_none());
    }

    // ── Edge cases (mined from reference EXIF parsers) ──────────────────────

    /// Little-endian TIFF, IFD0 at offset 8, given 12-byte entries + tail.
    fn le_ifd0(entries: &[[u8; 12]], next: u32, tail: &[u8]) -> alloc::vec::Vec<u8> {
        let mut v = vec![b'I', b'I', 0x2A, 0x00];
        v.extend_from_slice(&8u32.to_le_bytes());
        v.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        for e in entries {
            v.extend_from_slice(e);
        }
        v.extend_from_slice(&next.to_le_bytes());
        v.extend_from_slice(tail);
        v
    }

    /// One little-endian IFD entry with an inline (≤4-byte) value.
    fn entry_inline(tag: u16, kind: u16, count: u32, val: [u8; 4]) -> [u8; 12] {
        let mut e = [0u8; 12];
        e[0..2].copy_from_slice(&tag.to_le_bytes());
        e[2..4].copy_from_slice(&kind.to_le_bytes());
        e[4..8].copy_from_slice(&count.to_le_bytes());
        e[8..12].copy_from_slice(&val);
        e
    }

    /// One little-endian IFD entry whose value lives out-of-line at `offset`.
    fn entry_offset(tag: u16, kind: u16, count: u32, offset: u32) -> [u8; 12] {
        entry_inline(tag, kind, count, offset.to_le_bytes())
    }

    #[test]
    fn orientation_as_rational_is_rejected() {
        // Orientation declared RATIONAL (type 5) → orientation() returns None
        // (only SHORT/LONG accepted), but the blob still parses.
        let e = entry_offset(TAG_ORIENTATION, 5, 1, 26); // 8-byte value in tail
        let blob = le_ifd0(&[e], 0, &[1, 0, 0, 0, 1, 0, 0, 0]);
        let x = Exif::parse(&blob).expect("parses");
        assert_eq!(x.orientation(), None);
    }

    #[test]
    fn ascii_no_nul_terminator_uses_whole_value() {
        let e = entry_inline(TAG_COPYRIGHT, TIFF_ASCII, 4, *b"abcd"); // no NUL
        let blob = le_ifd0(&[e], 0, &[]);
        assert_eq!(Exif::parse(&blob).unwrap().copyright().unwrap(), "abcd");
    }

    #[test]
    fn ascii_embedded_nul_truncates() {
        let e = entry_offset(TAG_COPYRIGHT, TIFF_ASCII, 6, 26);
        let blob = le_ifd0(&[e], 0, b"ab\0cd\0");
        assert_eq!(Exif::parse(&blob).unwrap().copyright().unwrap(), "ab");
    }

    #[test]
    fn ascii_leading_nul_is_none() {
        let e = entry_inline(TAG_COPYRIGHT, TIFF_ASCII, 1, [0, 0, 0, 0]);
        let blob = le_ifd0(&[e], 0, &[]);
        assert!(Exif::parse(&blob).unwrap().copyright().is_none());
    }

    #[test]
    fn latin1_copyright_decodes_lossy() {
        // 0xA9 is Latin-1 ©, invalid UTF-8 → U+FFFD via lossy decode.
        let e = entry_offset(TAG_COPYRIGHT, TIFF_ASCII, 5, 26);
        let blob = le_ifd0(&[e], 0, b"\xA9 Me\0");
        assert_eq!(
            Exif::parse(&blob).unwrap().copyright().unwrap(),
            "\u{FFFD} Me"
        );
    }

    #[test]
    fn utf8_copyright_decodes_borrowed() {
        // 0xC2 0xA9 is UTF-8 © → valid, returned borrowed.
        let e = entry_offset(TAG_COPYRIGHT, TIFF_ASCII, 6, 26);
        let blob = le_ifd0(&[e], 0, b"\xC2\xA9 Me\0");
        let x = Exif::parse(&blob).unwrap();
        assert_eq!(x.copyright().unwrap(), "© Me");
        assert!(matches!(x.copyright().unwrap(), Cow::Borrowed(_)));
    }

    #[test]
    fn copyright_wrong_type_is_ignored() {
        // Copyright tag declared SHORT (not ASCII) → copyright() returns None
        // rather than reinterpreting the bytes as a string.
        let e = entry_inline(TAG_COPYRIGHT, TIFF_SHORT, 1, [6, 0, 0, 0]);
        let blob = le_ifd0(&[e], 0, &[]);
        assert!(Exif::parse(&blob).unwrap().copyright().is_none());
    }

    #[test]
    fn count_type_size_overflow_entry_skipped_others_survive() {
        // An entry whose count × type_size overflows (or is absurdly large) is
        // skipped, not fatal — sibling entries still parse (graceful
        // degradation). Orientation here survives the bad RATIONAL entry.
        let ori = entry_inline(TAG_ORIENTATION, TIFF_SHORT, 1, [6, 0, 0, 0]);
        let bad = entry_offset(0x0111, 5, 0x8000_0000, 100); // huge RATIONAL
        let blob = le_ifd0(&[ori, bad], 0, &[]);
        let x = Exif::parse(&blob).expect("salvages the good entry");
        assert_eq!(x.orientation(), Some(Orientation::Rotate90));
    }

    #[test]
    fn unknown_tiff_type_entry_skipped() {
        // A future/unknown TIFF type (99) is skipped, not fatal.
        let ori = entry_inline(TAG_ORIENTATION, TIFF_SHORT, 1, [6, 0, 0, 0]);
        let weird = entry_inline(0x9999, 99, 1, [1, 2, 3, 4]);
        let blob = le_ifd0(&[ori, weird], 0, &[]);
        let x = Exif::parse(&blob).expect("salvages the good entry");
        assert_eq!(x.orientation(), Some(Orientation::Rotate90));
    }

    #[test]
    fn truncated_entry_table_salvages_prior_entries() {
        // IFD claims 2 entries but only 1 (+ a stub) is present: the readable
        // entry survives, the truncated one is dropped, no panic.
        let mut v = vec![b'I', b'I', 0x2A, 0x00];
        v.extend_from_slice(&8u32.to_le_bytes());
        v.extend_from_slice(&2u16.to_le_bytes()); // claims 2 entries
        v.extend_from_slice(&entry_inline(TAG_ORIENTATION, TIFF_SHORT, 1, [6, 0, 0, 0]));
        v.extend_from_slice(&[0xAA, 0xBB]); // truncated 2nd entry (only 2 of 12 bytes)
        let x = Exif::parse(&v).expect("salvages the readable entry");
        assert_eq!(x.orientation(), Some(Orientation::Rotate90));
    }

    #[test]
    fn unparseable_exif_under_stripping_policy_drops_fail_safe() {
        // Can't parse → can't verify the strip → drop (fail-safe), not pass
        // through. (A keep-everything policy never reaches this path.)
        assert!(
            retain(
                b"not a valid tiff blob",
                &ExifPolicy::ATTRIBUTED_ORIENTATION
            )
            .is_none()
        );
        // Keep-everything still passes through unchanged.
        let garbage = b"not a valid tiff blob";
        assert!(matches!(
            retain(garbage, &ExifPolicy::KEEP_ALL),
            Some(Cow::Borrowed(_))
        ));
    }

    #[test]
    fn exif_pointer_to_invalid_offset_is_swallowed() {
        // Exif-IFD pointer past EOF → exif_ifd None, IFD0 still readable.
        let ori = entry_inline(TAG_ORIENTATION, TIFF_SHORT, 1, [6, 0, 0, 0]);
        let ptr = entry_inline(TAG_EXIF_IFD, TIFF_LONG, 1, 0xFFFFu32.to_le_bytes());
        let blob = le_ifd0(&[ori, ptr], 0, &[]);
        let x = Exif::parse(&blob).expect("parses");
        assert_eq!(x.orientation(), Some(Orientation::Rotate90));
        assert!(!x.has_gps());
    }

    #[test]
    fn exif_pointer_cycle_to_ifd0_terminates() {
        // Exif-IFD pointer points back at IFD0 (offset 8). zencodec re-parses
        // IFD0 once as the child (no recursion into child pointers) → must
        // terminate, no hang/stack-overflow.
        let ori = entry_inline(TAG_ORIENTATION, TIFF_SHORT, 1, [6, 0, 0, 0]);
        let ptr = entry_inline(TAG_EXIF_IFD, TIFF_LONG, 1, 8u32.to_le_bytes());
        let blob = le_ifd0(&[ori, ptr], 0, &[]);
        let x = Exif::parse(&blob).expect("parses + terminates");
        assert_eq!(x.orientation(), Some(Orientation::Rotate90));
    }

    #[test]
    fn huge_thumbnail_1mb_round_trips_and_borrows() {
        let big = vec![0xABu8; 1 << 20]; // 1 MiB thumbnail
        let exif = Exif {
            order: ByteOrder::Little,
            had_prefix: false,
            ifd0: vec![e(TAG_ORIENTATION, TIFF_SHORT, 1, &[6, 0])],
            exif_ifd: None,
            gps_ifd: None,
            ifd1: Some(vec![]),
            thumbnail: Some(&big),
        };
        let bytes = exif.to_bytes();
        let y = Exif::parse(&bytes).expect("parses");
        assert!(y.has_thumbnail());
        assert_eq!(y.thumbnail.unwrap().len(), 1 << 20);
        assert_eq!(y.thumbnail.unwrap(), &big[..]);
        // Keeping everything borrows the source — no copy of the 1 MiB payload.
        assert!(matches!(
            retain(&bytes, &ExifPolicy::KEEP_ALL),
            Some(Cow::Borrowed(_))
        ));
    }

    // ── Regressions for adversarial-review findings ─────────────────────────

    /// #4: a thumbnail whose length tag (0x0202) is encoded as SHORT (common in
    /// real cameras) must be recognized, not silently dropped.
    #[test]
    fn thumbnail_length_as_short_is_recognized() {
        // IFD0 @8 (orientation, next→IFD1@26); IFD1 @26 (0x0201 LONG offset=56,
        // 0x0202 SHORT length=4); thumbnail bytes @56.
        let mut v = vec![b'I', b'I', 0x2A, 0x00];
        v.extend_from_slice(&8u32.to_le_bytes());
        // IFD0: 1 entry, next = 26.
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&entry_inline(TAG_ORIENTATION, TIFF_SHORT, 1, [6, 0, 0, 0]));
        v.extend_from_slice(&26u32.to_le_bytes());
        // IFD1 @26: 2 entries, next = 0.
        v.extend_from_slice(&2u16.to_le_bytes());
        v.extend_from_slice(&entry_inline(
            TAG_THUMB_OFFSET,
            TIFF_LONG,
            1,
            56u32.to_le_bytes(),
        ));
        v.extend_from_slice(&entry_inline(TAG_THUMB_LENGTH, TIFF_SHORT, 1, [4, 0, 0, 0])); // SHORT!
        v.extend_from_slice(&0u32.to_le_bytes());
        // Thumbnail @56.
        v.extend_from_slice(&[0xFF, 0xD8, 0xFF, 0xD9]);

        let x = Exif::parse(&v).expect("parses");
        assert!(
            x.has_thumbnail(),
            "SHORT-length thumbnail must be recognized"
        );
        assert_eq!(x.thumbnail.unwrap(), &[0xFF, 0xD8, 0xFF, 0xD9]);
        assert_eq!(x.orientation(), Some(Orientation::Rotate90));
    }

    /// #1: many entries aliasing one out-of-line blob must not amplify the
    /// rewritten output (`ext_size` dedups true aliases).
    #[test]
    fn aliased_out_of_line_values_do_not_amplify() {
        // 40 "other"-category entries, ASCII count=100, all pointing at one
        // 100-byte blob in the tail (aliased after zero-copy parse).
        let n = 40u32;
        let blob_len = 100u32;
        let tail_off = 8 + 2 + 12 * n + 4;
        let entries: Vec<[u8; 12]> = (0..n)
            .map(|i| entry_offset(0x1000 + i as u16, TIFF_ASCII, blob_len, tail_off))
            .collect();
        let src = le_ifd0(&entries, 0, &vec![0x41u8; blob_len as usize]);

        let x = Exif::parse(&src).expect("parses");
        // Force a rewrite while keeping the aliased "other" entries.
        let policy = ExifPolicy::KEEP_ALL.with_gps(Retention::Discard);
        let out = x.filtered(&policy).to_bytes();
        // Deduped: one shared blob, not 40 copies. Without dedup this would be
        // ~40 × 100 = 4000+ bytes of ext; with it, ~100.
        assert!(
            out.len() < src.len() + 64,
            "amplification: {} vs src {}",
            out.len(),
            src.len()
        );
        assert!(Exif::parse(&out).is_some(), "rewritten output re-parses");
    }

    /// #3: a structural sub-IFD pointer too short to hold a 4-byte offset is
    /// preserved as an ordinary entry, not silently dropped.
    #[test]
    fn short_subifd_pointer_is_preserved() {
        let ori = entry_inline(TAG_ORIENTATION, TIFF_SHORT, 1, [6, 0, 0, 0]);
        // 0x8769 as BYTE/count-1 → 1-byte value, not a usable pointer.
        let bad_ptr = entry_inline(TAG_EXIF_IFD, 1, 1, [1, 0, 0, 0]);
        let src = le_ifd0(&[ori, bad_ptr], 0, &[]);
        let x = Exif::parse(&src).expect("parses");
        assert_eq!(x.orientation(), Some(Orientation::Rotate90));
        let out = x.to_bytes();
        // The 0x8769 entry survived the round-trip (LE tag bytes 0x69, 0x87).
        assert!(out.windows(2).any(|w| w == TAG_EXIF_IFD.to_le_bytes()));
    }

    /// Encoding: a non-ASCII (Latin-1) copyright is exposed raw via
    /// `copyright_bytes`, viewed lossily via `copyright`, and — critically —
    /// survives a pruning rewrite **byte-exact** (no transcode, no corruption,
    /// no "fixing" to ASCII).
    #[test]
    fn non_ascii_copyright_preserved_byte_exact() {
        // 0xA9 = Latin-1 ©, invalid UTF-8.
        let e = entry_offset(TAG_COPYRIGHT, TIFF_ASCII, 5, 26);
        let src = le_ifd0(&[e], 0, b"\xA9 Me\0");
        let x = Exif::parse(&src).expect("parses");
        assert_eq!(x.copyright_bytes(), Some(&b"\xA9 Me"[..])); // exact bytes
        assert_eq!(x.copyright().unwrap(), "\u{FFFD} Me"); // lossy view

        // Force a rewrite that keeps rights (copyright). Bytes must be preserved
        // verbatim — NOT transcoded to the UTF-8 of U+FFFD.
        let policy = ExifPolicy::KEEP_ALL.with_gps(Retention::Discard);
        let out = x.filtered(&policy).to_bytes();
        let y = Exif::parse(&out).expect("re-parses");
        assert_eq!(
            y.copyright_bytes(),
            Some(&b"\xA9 Me"[..]),
            "Latin-1 copyright must round-trip byte-exact, not transcode"
        );
    }

    /// Canonicalization (regression for a fuzz-found non-idempotence):
    /// `to_bytes` is a byte-exact fixpoint even when input entries are not
    /// tag-sorted and carry out-of-line values — the ext layout follows the
    /// tag-sorted write order, so re-serializing the output reproduces it and
    /// filtering is idempotent.
    #[test]
    fn to_bytes_is_canonical_fixpoint() {
        let va = [0xAAu8; 10];
        let vb = [0xBBu8; 10];
        // Descending tag order (unsorted), both out-of-line (>4 bytes).
        let x = Exif {
            order: ByteOrder::Little,
            had_prefix: false,
            ifd0: vec![
                e(0x0200, TIFF_ASCII, 10, &va),
                e(0x0100, TIFF_ASCII, 10, &vb),
            ],
            exif_ifd: None,
            gps_ifd: None,
            ifd1: None,
            thumbnail: None,
        };
        let b1 = x.to_bytes();
        let b2 = Exif::parse(&b1).expect("re-parses").to_bytes();
        assert_eq!(b1, b2, "to_bytes must be a byte-exact fixpoint (canonical)");
    }

    /// Exif 2.32 / CIPA DC-008 Table 6: Copyright/Artist may be **UTF-8**
    /// (type 129), not just ASCII (type 2). A UTF-8-typed Copyright must parse,
    /// read as Unicode, and round-trip (type + bytes preserved) — not get
    /// dropped as an unknown type.
    #[test]
    fn utf8_typed_copyright_parses_and_round_trips() {
        // "© Me\0" = C2 A9 20 4D 65 00 (6 bytes incl. NUL), stored out-of-line.
        let e = entry_offset(TAG_COPYRIGHT, TIFF_UTF8, 6, 26);
        let blob = le_ifd0(&[e], 0, b"\xC2\xA9 Me\0");
        let x = Exif::parse(&blob).expect("parses UTF-8-typed copyright");
        assert_eq!(x.copyright().unwrap(), "© Me");
        assert!(matches!(x.copyright().unwrap(), Cow::Borrowed(_))); // valid UTF-8
        let out = x.to_bytes();
        let y = Exif::parse(&out).expect("re-parses");
        assert_eq!(y.copyright().unwrap(), "© Me");
        assert_eq!(y.copyright_bytes(), Some(&b"\xC2\xA9 Me"[..]));
        // The UTF-8 type (129 → LE 0x81,0x00) survived the round-trip.
        assert!(out.windows(2).any(|w| w == TIFF_UTF8.to_le_bytes()));
    }

    /// Filtering is idempotent: re-filtering the result with the same policy
    /// yields byte-identical EXIF (relies on the canonical `to_bytes`).
    #[test]
    fn filtering_is_idempotent() {
        let src = sample(ByteOrder::Little, false);
        let policy = ExifPolicy::KEEP_ALL.with_gps(Retention::Discard);
        let once = match retain(&src, &policy) {
            Some(c) => c.into_owned(),
            None => return,
        };
        let twice = retain(&once, &policy).map(|c| c.into_owned());
        assert_eq!(Some(once), twice, "filtering must be idempotent");
    }

    /// Attribution vs device identity: the Exif-IFD creator/owner *name* tags
    /// (Photographer 0xA437, etc.) are `Rights` — a rights-keeping policy keeps
    /// them — while device tags (BodySerialNumber 0xA431) are `Camera` and get
    /// stripped. (Regression for the copyright-owner-vs-string classification.)
    #[test]
    fn attribution_tags_kept_device_tags_dropped() {
        let exif = Exif {
            order: ByteOrder::Little,
            had_prefix: false,
            ifd0: vec![e(TAG_ORIENTATION, TIFF_SHORT, 1, &[6, 0])],
            exif_ifd: Some(vec![
                e(0xA431, TIFF_ASCII, 4, b"SN1\0"),  // BodySerialNumber → Camera
                e(0xA437, TIFF_ASCII, 4, b"Me\0\0"), // Photographer → Rights
            ]),
            gps_ifd: None,
            ifd1: None,
            thumbnail: None,
        };
        // Web keeps orientation + rights, drops camera/device.
        let out = exif
            .filtered(&ExifPolicy::ATTRIBUTED_ORIENTATION)
            .to_bytes();
        assert!(
            out.windows(2).any(|w| w == 0xA437u16.to_le_bytes()),
            "Photographer (attribution) must survive a rights policy"
        );
        assert!(
            !out.windows(2).any(|w| w == 0xA431u16.to_le_bytes()),
            "BodySerialNumber (device identity) must be stripped"
        );
    }
}
