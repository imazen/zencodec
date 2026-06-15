//! Structured, borrowing EXIF/TIFF model: parse → inspect/prune → serialize.
//!
//! [`Exif::parse`] reads a TIFF/EXIF blob into a tree of IFDs whose entry
//! values *borrow* the source bytes (zero-copy — a multi-KB thumbnail is never
//! copied during parsing or pruning). [`Exif::filtered`] prunes the tree by
//! [`ExifPolicy`] category, and [`Exif::to_bytes`] re-serializes a valid TIFF,
//! recomputing all offsets. [`retain`](crate::exif::retain) is the `Cow` convenience used by
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
//! - **Fail-safe filtering.** [`retain`](crate::exif::retain) drops EXIF it can't parse under a
//!   stripping policy (rather than passing it through and risking a leak); see
//!   its docs.
//!
//! `MakerNote` (0x927C, the `camera` category) is opaque and may carry
//! maker-specific *internal* offsets (often TIFF-relative) that can't be fixed up
//! without parsing each vendor's format. A rewrite relocates the blob and would
//! break those, so any partial prune **drops** it rather than emit a corrupted
//! block (it also routinely embeds GPS/serials). Pipelines needing byte-exact
//! MakerNote must keep all EXIF (no prune), where the source passes through
//! untouched. Uncompressed (StripOffsets) thumbnails are likewise dropped on a
//! prune — kept only in the no-prune passthrough.

use alloc::borrow::Cow;
use alloc::vec::Vec;
use zenpixels::Orientation;

// ── TIFF/EXIF tag numbers (canonical, no bare hex in the logic below) ────────
// Names follow TIFF 6.0 / EXIF (CIPA DC-008). Tag values are stable across spec
// revisions; the comment notes the revision that introduced the less-common ones.

// IFD0 / TIFF baseline.
const TAG_MAKE: u16 = 0x010F;
const TAG_MODEL: u16 = 0x0110;
const TAG_ORIENTATION: u16 = 0x0112;
const TAG_SOFTWARE: u16 = 0x0131;
const TAG_DATETIME: u16 = 0x0132; // a.k.a. ModifyDate
const TAG_ARTIST: u16 = 0x013B;
const TAG_HOST_COMPUTER: u16 = 0x013C;
const TAG_COPYRIGHT: u16 = 0x8298;

// Structural pointers + JPEG thumbnail (modeled as tree edges, not entries).
const TAG_EXIF_IFD: u16 = 0x8769;
const TAG_GPS_IFD: u16 = 0x8825;
const TAG_INTEROP_IFD: u16 = 0xA005;
const TAG_THUMB_OFFSET: u16 = 0x0201; // JPEGInterchangeFormat
const TAG_THUMB_LENGTH: u16 = 0x0202; // JPEGInterchangeFormatLength
// SubIFDs (TIFF/DNG) — an array of offsets to nested IFDs (alt/full-res images
// with their own EXIF/GPS). NOT modeled here, so its offsets can't be fixed up
// on a rewrite; dropped during filtering rather than left dangling.
const TAG_SUBIFDS: u16 = 0x014A;
// Strip/tile image-data offset+count tags. For an uncompressed (non-JPEG)
// thumbnail in IFD1 these point at pixel data a rewrite does not carry, so they
// must be dropped on filter rather than left as dangling offsets.
const TAG_STRIP_OFFSETS: u16 = 0x0111;
const TAG_STRIP_BYTE_COUNTS: u16 = 0x0117;
const TAG_TILE_OFFSETS: u16 = 0x0144;
const TAG_TILE_BYTE_COUNTS: u16 = 0x0145;

// Exif sub-IFD: capture timestamps (the `datetimes` category).
const TAG_DATETIME_ORIGINAL: u16 = 0x9003;
const TAG_DATETIME_DIGITIZED: u16 = 0x9004;
const TAG_OFFSET_TIME: u16 = 0x9010; // Exif 2.31+
const TAG_OFFSET_TIME_ORIGINAL: u16 = 0x9011; // Exif 2.31+
const TAG_OFFSET_TIME_DIGITIZED: u16 = 0x9012; // Exif 2.31+
const TAG_SUBSEC_TIME: u16 = 0x9290;
const TAG_SUBSEC_TIME_ORIGINAL: u16 = 0x9291;
const TAG_SUBSEC_TIME_DIGITIZED: u16 = 0x9292;

// Exif sub-IFD: device / capture identity (the `camera` category).
const TAG_MAKER_NOTE: u16 = 0x927C;
const TAG_IMAGE_UNIQUE_ID: u16 = 0xA420;
const TAG_BODY_SERIAL_NUMBER: u16 = 0xA431;
const TAG_LENS_SPECIFICATION: u16 = 0xA432;
const TAG_LENS_MAKE: u16 = 0xA433;
const TAG_LENS_MODEL: u16 = 0xA434;
const TAG_LENS_SERIAL_NUMBER: u16 = 0xA435;

// Creator / rights-holder *name* tags (the `rights` category, alongside
// Copyright + Artist). CameraOwnerName is Exif 2.3+; Photographer / ImageEditor
// are Exif 3.0 (CIPA DC-008-2023).
const TAG_CAMERA_OWNER_NAME: u16 = 0xA430;
const TAG_PHOTOGRAPHER: u16 = 0xA437; // Exif 3.0
const TAG_IMAGE_EDITOR: u16 = 0xA438; // Exif 3.0

// Exif 3.0 software-identity tags (the `camera` category).
const TAG_CAMERA_FIRMWARE: u16 = 0xA439; // Exif 3.0
const TAG_RAW_DEVELOPING_SOFTWARE: u16 = 0xA43A; // Exif 3.0
const TAG_IMAGE_EDITING_SOFTWARE: u16 = 0xA43B; // Exif 3.0
const TAG_METADATA_EDITING_SOFTWARE: u16 = 0xA43C; // Exif 3.0

// ── TIFF field types (TIFF 6.0 §2; type 129 is Exif 3.0) ─────────────────────
const TIFF_BYTE: u16 = 1;
const TIFF_ASCII: u16 = 2;
const TIFF_SHORT: u16 = 3;
const TIFF_LONG: u16 = 4;
const TIFF_RATIONAL: u16 = 5;
const TIFF_SBYTE: u16 = 6;
const TIFF_UNDEFINED: u16 = 7;
const TIFF_SSHORT: u16 = 8;
const TIFF_SLONG: u16 = 9;
const TIFF_SRATIONAL: u16 = 10;
const TIFF_FLOAT: u16 = 11;
const TIFF_DOUBLE: u16 = 12;
const TIFF_IFD: u16 = 13;
/// Exif 3.0 (CIPA DC-008-2023) type 129 = UTF-8 string (8-bit bytes,
/// NUL-terminated, count includes the NUL). The spec-conformant way to store
/// Unicode in an IFD field — see [`TextEncoding`].
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

/// Which EXIF text convention a string field ([`Copyright`](Exif::set_copyright),
/// [`Artist`](Exif::set_artist)) is written with. EXIF has two ways to carry
/// text and a writer must pick one — there is no universally-read Unicode field.
///
/// Both variants write the **same UTF-8 bytes** (the `&str` you pass), NUL-
/// terminated; they differ only in the declared TIFF field type. For pure-ASCII
/// text the two outputs are identical except for that type tag.
///
/// `#[non_exhaustive]`: a future text convention can be added without a breaking
/// change. The variants are still constructible by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TextEncoding {
    /// **Exif 2.x** — TIFF `ASCII` (type 2). The spec says 7-bit ASCII, but the
    /// de-facto real-world convention (and what this writes) is UTF-8 bytes
    /// carried in the ASCII field — non-conformant for non-ASCII, yet read
    /// correctly by essentially every tool (kamadak-exif, Pillow, ExifTool, …).
    /// The most compatible choice, so the recommended default.
    Ascii,
    /// **Exif 3.0** (CIPA DC-008-2023) — TIFF `UTF-8` (type 129). The spec-
    /// conformant Unicode type, but reader support is still thin (ExifTool reads
    /// it; many libraries do not). Prefer [`Ascii`](Self::Ascii) unless the
    /// consumer is known to understand type 129.
    Utf8,
}

/// EXIF category an IFD0/Exif-IFD entry belongs to. GPS and thumbnail are
/// modeled structurally (whole sub-IFD), not per-entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Category {
    Orientation,
    Rights,
    Datetimes,
    Camera,
    Other,
}

fn classify(tag: u16) -> Category {
    match tag {
        TAG_ORIENTATION => Category::Orientation,
        // Attribution / rights-holder. Copyright (the rights *notice*), Artist
        // (creator), plus the Exif-IFD creator/owner *name* tags
        // (CameraOwnerName, Photographer, ImageEditor) — the spec says Artist
        // mirrors one of these, so they're the same "who made / holds rights"
        // class a copyright-preserving policy keeps.
        TAG_COPYRIGHT
        | TAG_ARTIST
        | TAG_CAMERA_OWNER_NAME
        | TAG_PHOTOGRAPHER
        | TAG_IMAGE_EDITOR => Category::Rights,
        // DateTime, DateTimeOriginal/Digitized, sub-sec + offset-time variants.
        TAG_DATETIME
        | TAG_DATETIME_ORIGINAL
        | TAG_DATETIME_DIGITIZED
        | TAG_OFFSET_TIME
        | TAG_OFFSET_TIME_ORIGINAL
        | TAG_OFFSET_TIME_DIGITIZED
        | TAG_SUBSEC_TIME
        | TAG_SUBSEC_TIME_ORIGINAL
        | TAG_SUBSEC_TIME_DIGITIZED => Category::Datetimes,
        // Device / software identity: Make, Model, Software, HostComputer,
        // MakerNote, body/lens serials + lens make/model, ImageUniqueID, and the
        // firmware / developing / editing software tags.
        TAG_MAKE
        | TAG_MODEL
        | TAG_SOFTWARE
        | TAG_HOST_COMPUTER
        | TAG_MAKER_NOTE
        | TAG_IMAGE_UNIQUE_ID
        | TAG_BODY_SERIAL_NUMBER
        | TAG_LENS_SPECIFICATION
        | TAG_LENS_MAKE
        | TAG_LENS_MODEL
        | TAG_LENS_SERIAL_NUMBER
        | TAG_CAMERA_FIRMWARE
        | TAG_RAW_DEVELOPING_SOFTWARE
        | TAG_IMAGE_EDITING_SOFTWARE
        | TAG_METADATA_EDITING_SOFTWARE => Category::Camera,
        _ => Category::Other,
    }
}

/// One IFD entry. Value bytes are [`Cow`]: **borrowed** from the source blob on
/// [`parse`](Exif::parse) (zero-copy — a multi-KB thumbnail is never copied) and
/// **owned** for an entry injected by an edit ([`set_copyright`](Exif::set_copyright)).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry<'a> {
    tag: u16,
    kind: u16,
    count: u32,
    /// Resolved value bytes (`count × type_size`), in source byte order.
    value: Cow<'a, [u8]>,
    /// Byte offset of `value` within the TIFF (post-prefix): `e + 8` for an
    /// inline value, or the out-of-line pointer. Lets an in-place tag rewrite
    /// (e.g. [`set_orientation`]) reuse this parse instead of re-walking the IFD.
    /// Meaningful only for a parsed (borrowed) entry; `0` for an injected one
    /// (which is always re-serialized by [`to_bytes`](Exif::to_bytes), never
    /// rewritten in place).
    value_offset: usize,
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
    /// Field type used when *writing* a string tag ([`set_copyright`](Self::set_copyright)
    /// / [`set_artist`](Self::set_artist)) — the Exif 2.x-vs-3.0 compatibility
    /// choice. Set by [`new`](Self::new); parsing defaults it to
    /// [`TextEncoding::Ascii`] (it is not stored in the TIFF, so it does not
    /// survive a parse round-trip).
    text_encoding: TextEncoding,
}

/// TIFF/Exif type size in bytes, or `None` for an unknown type.
fn type_size(kind: u16) -> Option<usize> {
    Some(match kind {
        TIFF_BYTE | TIFF_ASCII | TIFF_SBYTE | TIFF_UNDEFINED | TIFF_UTF8 => 1,
        TIFF_SHORT | TIFF_SSHORT => 2,
        TIFF_LONG | TIFF_SLONG | TIFF_FLOAT | TIFF_IFD => 4,
        TIFF_RATIONAL | TIFF_SRATIONAL | TIFF_DOUBLE => 8,
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
    let (value, value_offset) = if byte_len <= 4 {
        (tiff.get(e + 8..e + 8 + byte_len)?, e + 8)
    } else {
        let voff = rd32(tiff, e + 8, order)? as usize;
        (tiff.get(voff..voff.checked_add(byte_len)?)?, voff)
    };
    Some(Entry {
        tag,
        kind,
        count: cnt,
        value: Cow::Borrowed(value),
        value_offset,
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

impl<'a> Default for Exif<'a> {
    /// An empty EXIF tree with the compatible [`TextEncoding::Ascii`] default —
    /// see [`Exif::new`].
    fn default() -> Self {
        Self::new(TextEncoding::Ascii)
    }
}

impl<'a> Exif<'a> {
    /// Start an empty EXIF tree to build from scratch — e.g. to stamp a
    /// Copyright on an image that carried no EXIF. Little-endian, no `Exif\0\0`
    /// prefix.
    ///
    /// `text_encoding` is the **required** Exif 2.x-vs-3.0 compatibility choice
    /// for any string field this blob writes, because it can't be defaulted
    /// safely: [`TextEncoding::Utf8`] (type 129) is unreadable by most tools, so
    /// pick [`TextEncoding::Ascii`] (UTF-8 bytes in a type-2 field — the
    /// compatible de-facto form) unless every consumer is known to handle
    /// type 129. ([`Exif::default()`](Default) uses `Ascii`.) Set fields with
    /// [`set_copyright`](Self::set_copyright) / [`set_artist`](Self::set_artist),
    /// then [`to_bytes`](Self::to_bytes) (a raw TIFF — the JPEG/codec layer adds
    /// the APP1 `Exif\0\0` framing).
    ///
    /// ```
    /// use zencodec::exif::{Exif, TextEncoding};
    /// let mut exif = Exif::new(TextEncoding::Ascii); // compatible default
    /// exif.set_copyright("© 2026 Lilith");
    /// let blob = exif.to_bytes();
    /// assert_eq!(Exif::parse(&blob).unwrap().copyright().unwrap(), "© 2026 Lilith");
    /// ```
    pub fn new(text_encoding: TextEncoding) -> Self {
        Exif {
            order: ByteOrder::Little,
            had_prefix: false,
            ifd0: Vec::new(),
            exif_ifd: None,
            gps_ifd: None,
            ifd1: None,
            thumbnail: None,
            text_encoding,
        }
    }

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
                && let Some(t) = (o as usize)
                    .checked_add(l as usize)
                    .and_then(|end| tiff.get(o as usize..end))
            {
                thumbnail = Some(t);
                entries.retain(|e| e.tag != TAG_THUMB_OFFSET && e.tag != TAG_THUMB_LENGTH);
            }
            ifd1 = Some(entries);
        }

        // `to_bytes` SYNTHESIZES the IFD0 sub-IFD pointers (Exif/GPS) fresh from
        // the tree shape, so once a sub-IFD has been extracted, any *remaining*
        // entry with that pointer tag in IFD0 is a parse artifact — a source
        // duplicate (the original repro had two 0x8825 entries). It must be
        // dropped, NOT round-tripped as data: re-parse matches these tags by
        // number, so the stale entry shadows the synthesized pointer and drops
        // the real sub-IFD (gps presence drift + broken fixpoint, fuzz
        // zencodec#30/#96). Strip ONLY when the pointer is being synthesized —
        // a structural tag whose value was too short to be a usable offset is
        // legitimately preserved as data by `take_pointer` (sub-IFD stays
        // `None`), and must keep round-tripping (`short_subifd_pointer_is_preserved`).
        if exif_ifd.is_some() {
            ifd0.retain(|e| e.tag != TAG_EXIF_IFD);
        }
        if gps_ifd.is_some() {
            ifd0.retain(|e| e.tag != TAG_GPS_IFD);
        }

        Some(Exif {
            order,
            had_prefix,
            ifd0,
            exif_ifd,
            gps_ifd,
            ifd1,
            thumbnail,
            // Not stored in the TIFF; edits to a parsed blob default to the
            // compatible ASCII (type-2) form unless rebuilt via `Exif::new`.
            text_encoding: TextEncoding::Ascii,
        })
    }

    /// The byte order (endianness) of the parsed TIFF/EXIF stream. Preserved
    /// across [`to_bytes`](Self::to_bytes).
    pub fn byte_order(&self) -> ByteOrder {
        self.order
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
    pub fn copyright(&self) -> Option<Cow<'_, str>> {
        ascii_value(&self.ifd0, TAG_COPYRIGHT)
    }

    /// The Artist tag (0x013B) as text — a **lossy view** of
    /// [`artist_bytes`](Self::artist_bytes). See the [encoding note](#encoding).
    pub fn artist(&self) -> Option<Cow<'_, str>> {
        ascii_value(&self.ifd0, TAG_ARTIST)
    }

    /// The raw Copyright (0x8298) value bytes, NUL-terminator stripped — the
    /// field exactly as stored, with no decoding.
    ///
    /// # Encoding
    ///
    /// Per Exif / CIPA DC-008 (Table 6), Copyright and Artist are stored as
    /// **ASCII (type 2, NUL-terminated 7-bit)**; Exif 3.0 (CIPA DC-008-2023)
    /// added **UTF-8 (type 129)** as the spec-conformant way to carry Unicode in
    /// these fields. A type-2 field that nonetheless contains non-ASCII bytes
    /// (UTF-8 / Latin-1 stuffed into an ASCII field — common in the wild) is the
    /// non-conformant case. zencodec reads both string types and
    /// [`copyright`](Self::copyright) / [`artist`](Self::artist) decode them as
    /// UTF-8 lossily (invalid sequences → U+FFFD) for a display string, while
    /// these `*_bytes` accessors return the exact bytes. A pruning rewrite
    /// **never transcodes** — it preserves the value bytes **and TIFF type**
    /// verbatim, so a field is neither corrupted nor "corrected". Writing a field
    /// is explicit and the only path that mints new bytes:
    /// [`set_copyright`](Self::set_copyright) / [`set_artist`](Self::set_artist)
    /// take a [`TextEncoding`] choosing the TIFF type (Exif 2.x ASCII vs Exif 3.0
    /// UTF-8).
    ///
    /// Non-ASCII bytes in a type-2 field are **not** stripped: before type 129
    /// existed (Exif 2.32), the de-facto way to carry non-ASCII here was
    /// undeclared UTF-8, so decoding as UTF-8 recovers the common case —
    /// stripping the high bytes would corrupt it. A field that actually used a
    /// legacy code page (Latin-1, Shift-JIS) decodes lossily (→ U+FFFD); read
    /// `*_bytes` and apply your own decoder for those.
    pub fn copyright_bytes(&self) -> Option<&[u8]> {
        ascii_bytes(&self.ifd0, TAG_COPYRIGHT)
    }

    /// The raw Artist (0x013B) value bytes, NUL-terminator stripped. See
    /// [`copyright_bytes`](Self::copyright_bytes) for the encoding note.
    pub fn artist_bytes(&self) -> Option<&[u8]> {
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

    /// Whether any device/capture-identity tag (the [`camera`](ExifPolicy::camera)
    /// category — Make/Model/Software/MakerNote/serials/lens/ImageUniqueID/firmware…)
    /// is present, in IFD0 or the Exif sub-IFD. Lets a privacy check assert that a
    /// stripping policy actually removed camera identity (not just GPS/thumbnail).
    pub fn has_camera(&self) -> bool {
        self.has_category(Category::Camera)
    }

    /// Whether any capture-timestamp tag (the [`datetimes`](ExifPolicy::datetimes)
    /// category — DateTime / DateTimeOriginal / Digitized / SubSecTime\* /
    /// OffsetTime\*) is present, in IFD0 or the Exif sub-IFD.
    pub fn has_datetimes(&self) -> bool {
        self.has_category(Category::Datetimes)
    }

    /// Whether any IFD0/Exif-IFD entry falls in `cat` (the per-entry categories;
    /// GPS/thumbnail are structural — use [`has_gps`](Self::has_gps) /
    /// [`has_thumbnail`](Self::has_thumbnail)).
    fn has_category(&self, cat: Category) -> bool {
        let any = |ifd: &[Entry<'_>]| ifd.iter().any(|e| classify(e.tag) == cat);
        any(&self.ifd0) || self.exif_ifd.as_deref().is_some_and(any)
    }

    /// Set (insert or replace) the IFD0 Copyright tag (0x8298) to `text`.
    ///
    /// The TIFF field type is this blob's [`text_encoding`](Self::new) (Exif 2.x
    /// ASCII type 2, or Exif 3.0 UTF-8 type 129) — chosen once at [`new`](Self::new),
    /// or [`TextEncoding::Ascii`] for a parsed blob. The value is written
    /// NUL-terminated (count includes the NUL); an existing Copyright entry is
    /// replaced in place (keeping IFD order), otherwise a new one is appended.
    /// Materialized on the next [`to_bytes`](Self::to_bytes); the injected value
    /// is owned, so the output is independent of any source.
    ///
    /// To *remove* the field instead, [`filtered`](Self::filtered) with a policy
    /// that discards [`rights`](ExifPolicy::rights). `text` is written as-is (its
    /// UTF-8 bytes); an embedded NUL truncates the field when later read.
    pub fn set_copyright(&mut self, text: &str) {
        set_ifd0_string(&mut self.ifd0, TAG_COPYRIGHT, text, self.text_encoding);
    }

    /// Set (insert or replace) the IFD0 Artist tag (0x013B) to `text`. See
    /// [`set_copyright`](Self::set_copyright) for encoding and replace semantics.
    pub fn set_artist(&mut self, text: &str) {
        set_ifd0_string(&mut self.ifd0, TAG_ARTIST, text, self.text_encoding);
    }

    /// Set (insert or replace) the IFD0 Orientation tag (0x0112) to `o`.
    ///
    /// An existing SHORT/LONG entry keeps its TIFF type (value updated, count
    /// normalized to 1); a malformed non-integer carrier is replaced by the
    /// canonical 1-count SHORT form; a tag-less blob gains a SHORT entry (the
    /// serializer writes IFDs tag-sorted, so insertion position is immaterial).
    /// Materialized on the next [`to_bytes`](Self::to_bytes); the injected
    /// value is owned, so the output is independent of any source.
    ///
    /// This *authors* the blob — e.g. stamping Orientation on a from-scratch
    /// tree ([`new`](Self::new)) for an encoder that takes raw EXIF bytes. In
    /// the framework flow, [`Metadata::orientation`](crate::Metadata) remains
    /// the authoritative field; `Metadata::filtered` reconciles an embedded tag
    /// against it (via the byte-level
    /// [`helpers::set_exif_orientation`](crate::helpers::set_exif_orientation))
    /// and deliberately never *adds* one.
    ///
    /// ```
    /// use zencodec::Orientation;
    /// use zencodec::exif::{Exif, TextEncoding};
    /// let mut exif = Exif::new(TextEncoding::Ascii);
    /// exif.set_orientation(Orientation::Rotate90);
    /// let blob = exif.to_bytes();
    /// assert_eq!(Exif::parse(&blob).unwrap().orientation(), Some(Orientation::Rotate90));
    /// ```
    pub fn set_orientation(&mut self, o: Orientation) {
        let order = self.order;
        let v = u32::from(o.to_exif());
        match self.ifd0.iter_mut().find(|e| e.tag == TAG_ORIENTATION) {
            Some(entry) => match int_bytes(entry.kind, v, order) {
                Some(value) => {
                    entry.count = 1;
                    entry.value = value;
                }
                // Non-integer carrier: an explicit set replaces it with the
                // canonical form — contrast `set_orientation_tag`, which must
                // stay conservative because reconciliation has no license to
                // repair a field it wasn't asked to author.
                None => *entry = orientation_entry(o, order),
            },
            None => self.ifd0.push(orientation_entry(o, order)),
        }
    }

    /// Prune the tree by `policy`, returning a new borrowing view. Surviving
    /// entries still borrow the original source (no payload copy).
    pub fn filtered(&self, policy: &ExifPolicy) -> Exif<'a> {
        let keep = |e: &&Entry<'a>| match e.tag {
            // Offset-bearing structural tags whose value is a file offset, dropped
            // here so a rewrite never emits a dangling offset:
            //  - The modeled sub-IFD/thumbnail pointers (Exif/GPS/Interop, the
            //    JPEGInterchangeFormat offset+length) are re-synthesized by
            //    `to_bytes` from the extracted sub-IFDs / thumbnail. A copy left in
            //    an IFD would either duplicate the synthesized one or, if it was
            //    never extractable (malformed type, so it stayed here), dangle.
            //  - The unrelocatable ones (SubIFDs; strip/tile tables for an
            //    uncompressed thumbnail) point at bytes a rewrite doesn't carry.
            TAG_SUBIFDS
            | TAG_EXIF_IFD
            | TAG_GPS_IFD
            | TAG_INTEROP_IFD
            | TAG_THUMB_OFFSET
            | TAG_THUMB_LENGTH
            | TAG_STRIP_OFFSETS
            | TAG_STRIP_BYTE_COUNTS
            | TAG_TILE_OFFSETS
            | TAG_TILE_BYTE_COUNTS => false,
            // MakerNote is opaque and may carry maker-internal (TIFF-relative)
            // offsets we can't fix up; a rewrite relocates it and would corrupt
            // those. It also routinely embeds GPS/serials. Drop it on any prune —
            // byte-exact preservation is the keep-everything (no-rewrite) path.
            TAG_MAKER_NOTE => false,
            tag => policy.keeps(classify(tag)),
        };
        let ifd0 = self.ifd0.iter().filter(keep).cloned().collect();
        let exif_ifd = self
            .exif_ifd
            .as_ref()
            .map(|d| d.iter().filter(keep).cloned().collect::<Vec<_>>())
            .filter(|d: &Vec<_>| !d.is_empty());
        let gps_ifd = match policy.gps {
            Retention::Keep => self.gps_ifd.clone(),
            Retention::Discard => None,
        };
        // IFD1 (thumbnail directory) carries its own Make/Model/DateTime/etc.;
        // run it through the same per-category filter as IFD0 so "keep thumbnail,
        // drop camera/datetimes" doesn't leak those via the thumbnail dir. The
        // IFD1 wrapper is kept (possibly empty) to hold the thumbnail pointers
        // that `to_bytes` synthesizes.
        let (ifd1, thumbnail) = match policy.thumbnail {
            Retention::Keep => (
                self.ifd1
                    .as_ref()
                    .map(|d| d.iter().filter(keep).cloned().collect::<Vec<_>>()),
                self.thumbnail,
            ),
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
            text_encoding: self.text_encoding,
        }
    }

    /// Set the IFD0 orientation tag value to `o` *if such an entry exists* (does
    /// not add one — a tag-less blob is left tag-less, matching `set_orientation`).
    /// Edits the parsed entry so a subsequent [`to_bytes`](Self::to_bytes) emits
    /// the reconciled value in a single serialize, no re-parse. Preserves the
    /// entry's TIFF type (SHORT/LONG); a non-integer carrier is left untouched.
    fn set_orientation_tag(&mut self, o: Orientation) {
        let order = self.order;
        let Some(entry) = self.ifd0.iter_mut().find(|e| e.tag == TAG_ORIENTATION) else {
            return;
        };
        // Non-integer orientation carriers are left untouched (reconciliation
        // rewrites the value, it doesn't repair a field it wasn't asked to
        // author). But when we DO rewrite the value, we author a single
        // element, so `count` must become 1 to match: leaving a malformed
        // source count (e.g. 20) next to a 1-element value yields an entry whose
        // declared `count × type_size` exceeds its data, which serializes as a
        // dangling out-of-line offset and is silently dropped on re-parse —
        // making `filtered` non-idempotent (`Some` → `None`) and losing the
        // orientation (fuzz zencodec#97).
        if let Some(value) = int_bytes(entry.kind, u32::from(o.to_exif()), order) {
            entry.value = value;
            entry.count = 1;
        }
    }

    /// Projected serialized length — exactly `self.to_bytes().len()`, computed
    /// without allocating the output.
    ///
    /// Lets a rewrite be size-checked *before* it allocates, so a malformed blob
    /// whose out-of-line values alias **overlapping** source windows (which the
    /// per-IFD exact-alias dedup in [`write_ifd`](Self::write_ifd) does not merge)
    /// can be rejected instead of amplifying the output ~1000× / overflowing the
    /// `u32` offsets. Must stay in lockstep with [`to_bytes`](Self::to_bytes)
    /// (guarded by `serialized_len_equals_to_bytes_len`).
    pub(crate) fn serialized_len(&self) -> usize {
        let ifd0_nptr = self.exif_ifd.is_some() as usize + self.gps_ifd.is_some() as usize;
        let ifd1_nptr = if self.thumbnail.is_some() { 2 } else { 0 };
        let block = |entries: &[Entry<'a>], nptr: usize| -> usize {
            2 + 12 * (entries.len() + nptr) + 4 + ext_size(entries)
        };
        let mut total = if self.had_prefix {
            EXIF_PREFIX.len()
        } else {
            0
        };
        total += TIFF_HEADER_SIZE + block(&self.ifd0, ifd0_nptr);
        if let Some(d) = &self.exif_ifd {
            total += block(d, 0);
        }
        if let Some(d) = &self.gps_ifd {
            total += block(d, 0);
        }
        if let Some(d) = &self.ifd1 {
            total += block(d, ifd1_nptr);
        }
        if let Some(t) = self.thumbnail {
            total += t.len();
        }
        total
    }

    /// Serialize to a valid TIFF, recomputing every offset. Preserves the
    /// source byte order and `Exif\0\0` framing.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.serialized_len());
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
                        v[..e.value.len()].copy_from_slice(&e.value);
                        out.extend_from_slice(&v);
                    } else {
                        let (ptr, len) = (e.value.as_ptr() as usize, e.value.len());
                        let off = if let Some(&(.., o)) =
                            placed.iter().find(|&&(p, l, _)| p == ptr && l == len)
                        {
                            o
                        } else {
                            let o = ext.len();
                            ext.extend_from_slice(&e.value);
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
        TIFF_SHORT => rd16(&e.value, 0, order).map(u32::from),
        TIFF_LONG => rd32(&e.value, 0, order),
        _ => None,
    }
}

/// Rewrite the IFD0 Orientation tag (0x0112) to `value` in place, returning a
/// new blob — or `None` if the blob is malformed or carries no SHORT/LONG
/// Orientation tag.
///
/// Reuses [`Exif::parse`] to locate the tag's inline value (via the entry's
/// recorded [`value_offset`](Entry::value_offset)) rather than re-walking the
/// IFD, then overwrites those bytes — offset-preserving, so the rest of the blob
/// (offsets, thumbnail, all other tags) is byte-identical. Orientation is always
/// inline (a 1-element SHORT/LONG fits the 4-byte value field).
pub(crate) fn set_orientation(data: &[u8], value: Orientation) -> Option<Vec<u8>> {
    let exif = Exif::parse(data)?;
    set_orientation_with(data, &exif, value)
}

/// In-place orientation rewrite using an already-parsed [`Exif`] (avoids a second
/// parse when the caller already has one). Offset-preserving: the rest of `data`
/// is byte-identical.
fn set_orientation_with(data: &[u8], exif: &Exif<'_>, value: Orientation) -> Option<Vec<u8>> {
    let entry = exif.ifd0.iter().find(|e| e.tag == TAG_ORIENTATION)?;
    let size = match entry.kind {
        TIFF_SHORT => 2usize,
        TIFF_LONG => 4,
        _ => return None, // non-integer orientation carrier — leave untouched
    };
    // `value_offset` is relative to the TIFF; account for the optional prefix.
    let base = if exif.had_prefix {
        EXIF_PREFIX.len()
    } else {
        0
    };
    let off = base.checked_add(entry.value_offset)?;
    let v = u32::from(value as u8);
    let mut out = data.to_vec();
    let dst = out.get_mut(off..off.checked_add(size)?)?;
    match exif.order {
        ByteOrder::Big => dst.copy_from_slice(&v.to_be_bytes()[4 - size..]),
        ByteOrder::Little => dst.copy_from_slice(&v.to_le_bytes()[..size]),
    }
    Some(out)
}

/// The raw bytes of a string-typed entry — ASCII (type 2) or UTF-8 (type 129,
/// Exif 2.32) — up to (not incl.) the first NUL. `None` if the tag is absent,
/// not a string type (a wrong-type field is ignored, not reinterpreted), or
/// empty.
fn ascii_bytes<'e>(entries: &'e [Entry<'_>], tag: u16) -> Option<&'e [u8]> {
    let e = entries.iter().find(|e| e.tag == tag)?;
    if e.kind != TIFF_ASCII && e.kind != TIFF_UTF8 {
        return None;
    }
    let value: &[u8] = &e.value;
    let end = value.iter().position(|&b| b == 0).unwrap_or(value.len());
    let bytes = &value[..end];
    if bytes.is_empty() { None } else { Some(bytes) }
}

/// Lossy-UTF-8 *view* of [`ascii_bytes`]. EXIF type-2 is spec'd 7-bit ASCII;
/// real files embed UTF-8/Latin-1, so valid UTF-8 (incl. ASCII) is borrowed
/// and invalid sequences (e.g. raw Latin-1 `0xA9`) become U+FFFD (owned). The
/// result is always a valid `str`; it is a read-only view and is never written
/// back (the filter preserves the original bytes verbatim).
fn ascii_value<'e>(entries: &'e [Entry<'_>], tag: u16) -> Option<Cow<'e, str>> {
    let bytes = ascii_bytes(entries, tag)?;
    Some(match core::str::from_utf8(bytes) {
        Ok(s) => Cow::Borrowed(s),
        Err(_) => Cow::Owned(alloc::string::String::from_utf8_lossy(bytes).into_owned()),
    })
}

/// Insert-or-replace a NUL-terminated string entry (Copyright, Artist, …) in an
/// IFD. The value is `text`'s UTF-8 bytes plus a trailing NUL (count includes
/// the NUL); the TIFF type is ASCII (2) or UTF-8 (129) per [`TextEncoding`]. The
/// owned `Cow` makes the entry independent of any source blob. An existing entry
/// with the same tag is overwritten in place (preserving IFD order); otherwise
/// the new entry is appended.
fn set_ifd0_string<'a>(entries: &mut Vec<Entry<'a>>, tag: u16, text: &str, encoding: TextEncoding) {
    let kind = match encoding {
        TextEncoding::Ascii => TIFF_ASCII,
        TextEncoding::Utf8 => TIFF_UTF8,
    };
    let mut bytes = text.as_bytes().to_vec();
    bytes.push(0); // NUL terminator; TIFF string count includes it.
    let entry = Entry {
        tag,
        kind,
        count: bytes.len() as u32,
        value: Cow::Owned(bytes),
        value_offset: 0, // injected entry: re-serialized by to_bytes, never rewritten in place
    };
    match entries.iter_mut().find(|e| e.tag == tag) {
        Some(slot) => *slot = entry,
        None => entries.push(entry),
    }
}

/// `v` encoded as an owned SHORT/LONG value in `order`; `None` for any other
/// TIFF type. Shared by the authoring setter ([`Exif::set_orientation`]) and
/// the reconciliation rewrite ([`Exif::set_orientation_tag`]).
fn int_bytes(kind: u16, v: u32, order: ByteOrder) -> Option<Cow<'static, [u8]>> {
    Some(Cow::Owned(match (kind, order) {
        (TIFF_SHORT, ByteOrder::Little) => (v as u16).to_le_bytes().to_vec(),
        (TIFF_SHORT, ByteOrder::Big) => (v as u16).to_be_bytes().to_vec(),
        (TIFF_LONG, ByteOrder::Little) => v.to_le_bytes().to_vec(),
        (TIFF_LONG, ByteOrder::Big) => v.to_be_bytes().to_vec(),
        _ => return None,
    }))
}

/// The canonical injected Orientation entry — 1-count SHORT, owned value.
fn orientation_entry<'a>(o: Orientation, order: ByteOrder) -> Entry<'a> {
    let v = u16::from(o.to_exif());
    Entry {
        tag: TAG_ORIENTATION,
        kind: TIFF_SHORT,
        count: 1,
        value: Cow::Owned(match order {
            ByteOrder::Little => v.to_le_bytes().to_vec(),
            ByteOrder::Big => v.to_be_bytes().to_vec(),
        }),
        value_offset: 0, // injected: re-serialized by to_bytes, never rewritten in place
    }
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
    pub datetimes: Retention,
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
        datetimes: Retention::Keep,
        camera: Retention::Keep,
        other: Retention::Keep,
    };
    /// Discard every category (drops EXIF entirely).
    pub const DISCARD_ALL: Self = Self {
        orientation: Retention::Discard,
        rights: Retention::Discard,
        thumbnail: Retention::Discard,
        gps: Retention::Discard,
        datetimes: Retention::Discard,
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
    pub const fn with_datetimes(mut self, r: Retention) -> Self {
        self.datetimes = r;
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
            Category::Datetimes => self.datetimes.keeps(),
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
            && self.datetimes.keeps()
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
/// - **Oversize (> 4 GiB) under a stripping policy → `None` (fail-safe).** Such a
///   blob can't be safely rewritten (offsets are `u32`), so it is dropped rather
///   than passed through unfiltered — same reasoning as the unparseable case.
pub fn retain<'a>(src: &'a [u8], policy: &ExifPolicy) -> Option<Cow<'a, [u8]>> {
    retain_reconciled(src, policy, None)
}

/// [`retain`], but in the **same parse** also reconciles the embedded IFD0
/// orientation tag to `want` (when `Some`).
///
/// This is what [`Metadata::filtered`](crate::Metadata::filtered) uses. Folding
/// the orientation reconcile in here collapses what was up to three
/// `Exif::parse` calls + two full-blob copies (filter, then a separate
/// parse-to-check + parse-and-rewrite) into **one parse and at most one
/// serialize**, with the same output:
///
/// - keep-everything + no orientation change → [`Cow::Borrowed`] (zero-copy);
/// - keep-everything + orientation change → byte-exact in-place tag rewrite
///   (or `Cow::Borrowed` if the tag already matches / is absent);
/// - partial policy → prune, set the tag on the parsed tree, serialize once.
///
/// `want = None` reproduces [`retain`] exactly. Fail-safe and oversize behavior
/// match [`retain`].
pub(crate) fn retain_reconciled<'a>(
    src: &'a [u8],
    policy: &ExifPolicy,
    want: Option<Orientation>,
) -> Option<Cow<'a, [u8]>> {
    if policy.discards_everything() {
        return None;
    }
    let keeps_all = policy.keeps_everything();
    // Keep-everything with no orientation change: zero-copy passthrough, no parse.
    if keeps_all && want.is_none() {
        return Some(Cow::Borrowed(src));
    }
    // A valid TIFF is ≤ 4 GiB (offsets are `u32`); a larger blob can't be safely
    // rewritten. Keep-everything passes through unchanged (skipping the orientation
    // fix rather than doing a multi-GiB copy); a stripping policy fails safe.
    if src.len() > u32::MAX as usize {
        return if keeps_all {
            Some(Cow::Borrowed(src))
        } else {
            None
        };
    }
    let Some(exif) = Exif::parse(src) else {
        // Unparseable: keep-everything passes through (can't verify, none asked);
        // a stripping policy fails safe (drop) — orientation lives on `Metadata`.
        return if keeps_all {
            Some(Cow::Borrowed(src))
        } else {
            None
        };
    };

    if keeps_all {
        // Nothing pruned — only maybe rewrite the orientation tag in place
        // (byte-exact except the value), reusing the parse we already have.
        match want {
            Some(o) if exif.orientation() != Some(o) => match set_orientation_with(src, &exif, o) {
                Some(v) => Some(Cow::Owned(v)),
                None => Some(Cow::Borrowed(src)), // tag-less / non-integer → unchanged
            },
            _ => Some(Cow::Borrowed(src)), // already matches, or absent → unchanged
        }
    } else {
        // Pruning rewrite: filter, set the tag on the tree, serialize once.
        let mut pruned = exif.filtered(policy);
        if let Some(o) = want {
            pruned.set_orientation_tag(o);
        }
        if pruned.ifd0.is_empty()
            && pruned.exif_ifd.is_none()
            && pruned.gps_ifd.is_none()
            && pruned.ifd1.is_none()
        {
            None
        } else {
            // Amplification / overflow guard. A faithful prune only *removes*
            // data, so its canonical re-serialization never exceeds the source
            // (a little slack covers per-value alignment padding). A malformed
            // blob that points many entries at overlapping source windows — which
            // the per-IFD exact-alias dedup does not coalesce — would instead blow
            // the output up ~1000× and push offsets past `u32`. Reject it (fail
            // safe: drop the EXIF) rather than OOM-aborting on an infallible `Vec`.
            let cap = src
                .len()
                .saturating_mul(2)
                .saturating_add(1024)
                .min(u32::MAX as usize);
            if pruned.serialized_len() > cap {
                return None;
            }
            Some(Cow::Owned(pruned.to_bytes()))
        }
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
            value: Cow::Borrowed(value),
            value_offset: 0, // synthetic entries; only the rewrite path needs a real offset
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
                e(TAG_MAKE, TIFF_ASCII, 4, b"Cam\0"),         // Make (camera)
                e(TAG_ORIENTATION, TIFF_SHORT, 1, &ori),      // Orientation=Rotate90
                e(TAG_COPYRIGHT, TIFF_ASCII, 7, b"(c) Me\0"), // Copyright (out-of-line)
            ],
            exif_ifd: Some(vec![e(TAG_DATETIME_ORIGINAL, TIFF_ASCII, 5, b"2020\0")]),
            gps_ifd: Some(vec![e(0x0001, TIFF_ASCII, 2, b"N\0")]), // GPSLatitudeRef
            ifd1: Some(vec![]),
            thumbnail: Some(&[0xFF, 0xD8, 0xFF, 0xD9]),
            text_encoding: TextEncoding::Ascii,
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
        // Camera (Make) and DateTimeOriginal gone.
        assert!(!out.windows(2).any(|w| w == TAG_MAKE.to_le_bytes()));
        assert!(
            !out.windows(2)
                .any(|w| w == TAG_DATETIME_ORIGINAL.to_le_bytes())
        );
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
            text_encoding: TextEncoding::Ascii,
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
            text_encoding: TextEncoding::Ascii,
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

    /// Sibling to the exact-alias test, for the case the per-IFD `(ptr,len)` dedup
    /// **cannot** catch: many entries whose out-of-line values are *overlapping,
    /// byte-shifted* windows of one region (distinct pointers). Re-serializing them
    /// faithfully would balloon the output ~13× and could push offsets past `u32`,
    /// so the rewrite path must reject the blob (fail-safe → `None`) rather than
    /// amplify on an infallible `Vec`.
    #[test]
    fn overlapping_window_values_rejected_not_amplified() {
        let k: u32 = 16;
        let win: u32 = 1024; // BYTE count → 1024-byte out-of-line value
        let vstart: u32 = 8 + 2 + 12 * k + 4; // where `le_ifd0` places the tail
        let entries: Vec<[u8; 12]> = (0..k)
            .map(|i| entry_offset(0xC000 + i as u16, TIFF_BYTE, win, vstart + i))
            .collect();
        let tail = vec![0xABu8; (k - 1 + win + 16) as usize];
        let src = le_ifd0(&entries, 0, &tail);

        let x = Exif::parse(&src).expect("malicious-but-parseable blob");
        assert_eq!(x.ifd0.len(), k as usize, "all overlapping entries parsed");
        // Distinct windows can't be deduped — `serialized_len` sees the blow-up
        // without allocating the (multi-MB in the wild) output.
        assert!(
            x.serialized_len() > src.len() * 3,
            "expected amplification: serialized_len {} vs src {}",
            x.serialized_len(),
            src.len()
        );
        // A stripping policy forces the rewrite; the guard must fail safe.
        let policy = ExifPolicy::KEEP_ALL.with_gps(Retention::Discard);
        assert_eq!(
            retain(&src, &policy),
            None,
            "amplifying blob must fail safe"
        );
    }

    /// The amplification guard relies on `serialized_len` being an exact preview of
    /// `to_bytes().len()` — verify they stay in lockstep across byte orders, the
    /// `Exif\0\0` prefix, and a pruned subset.
    #[test]
    fn serialized_len_equals_to_bytes_len() {
        for prefix in [false, true] {
            let bytes = sample(ByteOrder::Little, prefix);
            let x = Exif::parse(&bytes).expect("parses");
            assert_eq!(
                x.serialized_len(),
                x.to_bytes().len(),
                "full tree (prefix={prefix})"
            );
            let pruned = x.filtered(&ExifPolicy::KEEP_ALL.with_gps(Retention::Discard));
            assert_eq!(
                pruned.serialized_len(),
                pruned.to_bytes().len(),
                "pruned (prefix={prefix})"
            );
        }
        let be_bytes = sample(ByteOrder::Big, false);
        let be = Exif::parse(&be_bytes).expect("parses");
        assert_eq!(
            be.serialized_len(),
            be.to_bytes().len(),
            "big-endian full tree"
        );
    }

    #[test]
    fn has_camera_and_datetimes_classify_and_filter() {
        // `sample()` carries Make (camera, IFD0) + DateTimeOriginal (datetimes,
        // Exif-IFD) — the categories the testkit privacy check now asserts removed.
        let bytes = sample(ByteOrder::Little, false);
        let x = Exif::parse(&bytes).expect("parses");
        assert!(x.has_camera(), "Make is the camera category");
        assert!(
            x.has_datetimes(),
            "DateTimeOriginal is the datetimes category"
        );
        let stripped = x.filtered(
            &ExifPolicy::KEEP_ALL
                .with_camera(Retention::Discard)
                .with_datetimes(Retention::Discard),
        );
        assert!(!stripped.has_camera(), "camera category dropped");
        assert!(!stripped.has_datetimes(), "datetimes category dropped");
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

    /// A structural sub-IFD pointer left un-extracted (malformed type) must NOT
    /// be re-emitted on a *filtering* rewrite: keeping it would write a dangling
    /// offset, and for a GPS/Exif pointer it leaves a stray pointer the policy
    /// meant to strip. (The pure-serialize `to_bytes` path preserves it, per
    /// `short_subifd_pointer_is_preserved`; the filter path cleans it up.)
    #[test]
    fn stray_structural_pointer_dropped_on_filtering_rewrite() {
        let ori = entry_inline(TAG_ORIENTATION, TIFF_SHORT, 1, [6, 0, 0, 0]);
        // 0x8825 (GPS) as SHORT/count-1: not an extractable 4-byte pointer, so it
        // survives parse sitting in IFD0 (classified Other).
        let bad_gps = entry_inline(TAG_GPS_IFD, TIFF_SHORT, 1, [1, 0, 0, 0]);
        let src = le_ifd0(&[ori, bad_gps], 0, &[]);
        let x = Exif::parse(&src).expect("parses");
        // A pruning policy (drops gps) forces a rewrite; the stray GPS pointer
        // must not be re-emitted as a dangling offset.
        let policy = ExifPolicy::KEEP_ALL.with_gps(Retention::Discard);
        let out = x.filtered(&policy).to_bytes();
        assert!(
            !out.windows(2).any(|w| w == TAG_GPS_IFD.to_le_bytes()),
            "stray GPS pointer must be dropped on a filtering rewrite, not left dangling"
        );
        assert_eq!(
            Exif::parse(&out).unwrap().orientation(),
            Some(Orientation::Rotate90),
            "real metadata still round-trips"
        );
    }

    /// `retain_reconciled` prunes AND reconciles orientation in one pass, with the
    /// same output the old retain-then-reparse-twice path produced.
    #[test]
    fn retain_reconciled_reconciles_in_one_pass() {
        let ori = entry_inline(TAG_ORIENTATION, TIFF_SHORT, 1, [6, 0, 0, 0]); // Rotate90
        let cr = entry_inline(TAG_COPYRIGHT, TIFF_ASCII, 3, [b'M', b'e', 0, 0]);
        let src = le_ifd0(&[ori, cr], 0, &[]);

        // Prune (drop gps → rewrite) + reconcile the tag to the field (baked upright).
        let policy = ExifPolicy::KEEP_ALL.with_gps(Retention::Discard);
        let out = retain_reconciled(&src, &policy, Some(Orientation::Identity)).expect("some");
        let x = Exif::parse(&out).unwrap();
        assert_eq!(
            x.orientation(),
            Some(Orientation::Identity),
            "tag reconciled"
        );
        assert_eq!(x.copyright().unwrap(), "Me", "rights preserved");

        // Keep-everything + tag already matches → zero-copy borrow (no rewrite).
        assert!(matches!(
            retain_reconciled(&src, &ExifPolicy::KEEP_ALL, Some(Orientation::Rotate90)),
            Some(Cow::Borrowed(_))
        ));
        // Keep-everything + mismatch → in-place owned rewrite.
        let fixed =
            retain_reconciled(&src, &ExifPolicy::KEEP_ALL, Some(Orientation::Rotate180)).unwrap();
        assert_eq!(
            Exif::parse(&fixed).unwrap().orientation(),
            Some(Orientation::Rotate180)
        );
        // want = None reproduces `retain` (keep-everything → borrow).
        assert!(matches!(
            retain_reconciled(&src, &ExifPolicy::KEEP_ALL, None),
            Some(Cow::Borrowed(_))
        ));
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
            text_encoding: TextEncoding::Ascii,
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
                e(TAG_BODY_SERIAL_NUMBER, TIFF_ASCII, 4, b"SN1\0"), // → Camera
                e(TAG_PHOTOGRAPHER, TIFF_ASCII, 4, b"Me\0\0"),      // → Rights
            ]),
            gps_ifd: None,
            ifd1: None,
            thumbnail: None,
            text_encoding: TextEncoding::Ascii,
        };
        // Web keeps orientation + rights, drops camera/device.
        let out = exif
            .filtered(&ExifPolicy::ATTRIBUTED_ORIENTATION)
            .to_bytes();
        assert!(
            out.windows(2).any(|w| w == TAG_PHOTOGRAPHER.to_le_bytes()),
            "Photographer (attribution) must survive a rights policy"
        );
        assert!(
            !out.windows(2)
                .any(|w| w == TAG_BODY_SERIAL_NUMBER.to_le_bytes()),
            "BodySerialNumber (device identity) must be stripped"
        );
    }

    // ── Editing: set_copyright / set_artist (Exif 2.x ASCII vs Exif 3.0 UTF-8) ─

    /// One orientation-only IFD0 blob, for insert tests.
    fn orientation_only() -> alloc::vec::Vec<u8> {
        le_ifd0(
            &[entry_inline(TAG_ORIENTATION, TIFF_SHORT, 1, [6, 0, 0, 0])],
            0,
            &[],
        )
    }

    /// Insert a Copyright into a blob that had none, as Exif 2.x ASCII (type 2).
    /// Round-trips through serialize → parse; the other tags are untouched.
    #[test]
    fn set_copyright_inserts_ascii_type2() {
        let blob = orientation_only();
        let mut x = Exif::parse(&blob).unwrap();
        assert!(x.copyright().is_none());
        x.set_copyright("(c) 2026 Lilith");
        let out = x.to_bytes();
        let y = Exif::parse(&out).unwrap();
        assert_eq!(y.copyright().unwrap(), "(c) 2026 Lilith");
        assert_eq!(y.copyright_bytes(), Some(&b"(c) 2026 Lilith"[..]));
        assert_eq!(y.orientation(), Some(Orientation::Rotate90)); // unchanged
        // Stored as ASCII (type 2 → LE 0x02,0x00).
        assert!(out.windows(2).any(|w| w == TIFF_ASCII.to_le_bytes()));
    }

    /// Set a Copyright as Exif 3.0 UTF-8 (type 129); the declared type survives.
    #[test]
    fn set_copyright_utf8_writes_type129() {
        // Exif 3.0 / type-129 blob (the explicit opt-in).
        let mut x = Exif::new(TextEncoding::Utf8);
        x.set_copyright("© 2026 Lilith");
        let out = x.to_bytes();
        let y = Exif::parse(&out).unwrap();
        assert_eq!(y.copyright().unwrap(), "© 2026 Lilith");
        // UTF-8 type (129 → LE 0x81,0x00) is what was written.
        assert!(out.windows(2).any(|w| w == TIFF_UTF8.to_le_bytes()));
    }

    /// Setting Copyright replaces an existing entry in place (no duplicate).
    #[test]
    fn set_copyright_replaces_existing() {
        let src = sample(ByteOrder::Little, false); // has "(c) Me"
        let mut x = Exif::parse(&src).unwrap();
        assert_eq!(x.copyright().unwrap(), "(c) Me");
        x.set_copyright("(c) New Owner");
        let out = x.to_bytes();
        let y = Exif::parse(&out).unwrap();
        assert_eq!(y.copyright().unwrap(), "(c) New Owner");
        let n = y.ifd0.iter().filter(|e| e.tag == TAG_COPYRIGHT).count();
        assert_eq!(n, 1, "must replace, not duplicate");
    }

    /// Exif 2.x ASCII shoehorns UTF-8 bytes into the type-2 field (the de-facto
    /// convention): the bytes are the string's UTF-8, the type stays 2, and the
    /// value reads back as the same Unicode.
    #[test]
    fn set_copyright_ascii_carries_utf8_bytes_defacto() {
        let blob = orientation_only();
        let mut x = Exif::parse(&blob).unwrap();
        x.set_copyright("© Лилит"); // non-ASCII into type 2
        let out = x.to_bytes();
        let y = Exif::parse(&out).unwrap();
        assert_eq!(y.copyright_bytes(), Some("© Лилит".as_bytes()));
        assert_eq!(y.copyright().unwrap(), "© Лилит"); // valid UTF-8 → reads back
        assert!(out.windows(2).any(|w| w == TIFF_ASCII.to_le_bytes()));
    }

    /// `set_artist` mirrors `set_copyright` and lands in the `rights` category
    /// (kept by a rights-keeping policy, dropped when rights are discarded).
    #[test]
    fn set_artist_round_trips_and_is_rights() {
        let blob = orientation_only();
        let mut x = Exif::parse(&blob).unwrap();
        x.set_artist("Lilith");
        let out = x.to_bytes();
        let y = Exif::parse(&out).unwrap();
        assert_eq!(y.artist().unwrap(), "Lilith");
        let kept = y.filtered(&ExifPolicy::ATTRIBUTED_ORIENTATION).to_bytes();
        assert!(Exif::parse(&kept).unwrap().artist().is_some());
        let dropped = y
            .filtered(&ExifPolicy::KEEP_ALL.with_rights(Retention::Discard))
            .to_bytes();
        assert!(Exif::parse(&dropped).unwrap().artist().is_none());
    }

    /// An edited (owned) entry survives a layout-shifting rewrite: the value is
    /// owned, not aliased to a source offset, so the serializer relocates it like
    /// any other out-of-line value.
    #[test]
    fn edited_copyright_survives_filter_rewrite() {
        let src = sample(ByteOrder::Little, false);
        let mut x = Exif::parse(&src).unwrap();
        let long = "Copyright 2026 Lilith River — all rights reserved worldwide.";
        x.set_copyright(long); // long → out-of-line
        let out = x
            .filtered(&ExifPolicy::KEEP_ALL.with_gps(Retention::Discard))
            .to_bytes();
        let y = Exif::parse(&out).unwrap();
        assert_eq!(y.copyright().unwrap(), long);
        assert!(!y.has_gps());
        assert_eq!(y.orientation(), Some(Orientation::Rotate90));
    }

    /// Editing then serializing stays canonical (a byte-exact fixpoint), so an
    /// edited blob filters idempotently like a parsed one.
    #[test]
    fn edited_to_bytes_is_canonical_fixpoint() {
        let blob = orientation_only();
        let mut x = Exif::parse(&blob).unwrap();
        x.set_copyright("(c) Me");
        let b1 = x.to_bytes();
        let b2 = Exif::parse(&b1).unwrap().to_bytes();
        assert_eq!(b1, b2, "edited output must be a canonical fixpoint");
    }

    // ── Privacy hardening (MakerNote / SubIFDs / IFD1) ───────────────────────

    /// MakerNote (0x927C) is opaque and can embed GPS/serials; it must drop when
    /// GPS is stripped even if the `camera` category is kept (it can't be
    /// selectively scrubbed), and stay when both camera and gps are kept.
    #[test]
    fn makernote_dropped_when_gps_stripped_even_if_camera_kept() {
        let exif = Exif {
            order: ByteOrder::Little,
            had_prefix: false,
            ifd0: vec![
                e(TAG_ORIENTATION, TIFF_SHORT, 1, &[6, 0]),
                e(TAG_MAKER_NOTE, TIFF_UNDEFINED, 8, b"MAKER\0\0\0"),
            ],
            exif_ifd: None,
            gps_ifd: None,
            ifd1: None,
            thumbnail: None,
            text_encoding: TextEncoding::Ascii,
        };
        // Keep camera, drop GPS → MakerNote must be gone (could carry location).
        let stripped = exif
            .filtered(&ExifPolicy::KEEP_ALL.with_gps(Retention::Discard))
            .to_bytes();
        assert!(
            !stripped
                .windows(2)
                .any(|w| w == TAG_MAKER_NOTE.to_le_bytes()),
            "MakerNote must be stripped when GPS is dropped"
        );
        // Even with camera+gps kept, pruning `other` forces a rewrite that would
        // relocate the opaque MakerNote and break its maker-internal offsets — so
        // a pruning rewrite drops it rather than emit a corrupted block.
        let pruned = exif
            .filtered(&ExifPolicy::KEEP_ALL.with_other(Retention::Discard))
            .to_bytes();
        assert!(
            !pruned.windows(2).any(|w| w == TAG_MAKER_NOTE.to_le_bytes()),
            "MakerNote dropped on a pruning rewrite (can't safely relocate)"
        );
        // Byte-exact preservation is the no-prune path: retain(KEEP_ALL) returns
        // the blob untouched, MakerNote intact.
        let blob = exif.to_bytes();
        let kept = retain(&blob, &ExifPolicy::KEEP_ALL).expect("keep-all");
        assert!(
            matches!(kept, Cow::Borrowed(_))
                && kept.windows(2).any(|w| w == TAG_MAKER_NOTE.to_le_bytes()),
            "MakerNote preserved byte-exact under keep-everything (no rewrite)"
        );
    }

    /// An unmodeled SubIFDs pointer (0x014A) is dropped on a rewrite rather than
    /// left as a dangling offset; the rest of IFD0 survives.
    #[test]
    fn subifds_pointer_dropped_on_rewrite() {
        let exif = Exif {
            order: ByteOrder::Little,
            had_prefix: false,
            ifd0: vec![
                e(TAG_ORIENTATION, TIFF_SHORT, 1, &[6, 0]),
                e(TAG_SUBIFDS, TIFF_LONG, 1, &[0x40, 0, 0, 0]),
            ],
            exif_ifd: None,
            gps_ifd: None,
            ifd1: None,
            thumbnail: None,
            text_encoding: TextEncoding::Ascii,
        };
        let out = exif
            .filtered(&ExifPolicy::KEEP_ALL.with_gps(Retention::Discard))
            .to_bytes();
        assert!(
            !out.windows(2).any(|w| w == TAG_SUBIFDS.to_le_bytes()),
            "SubIFDs pointer must be dropped on rewrite (would dangle)"
        );
        assert_eq!(
            Exif::parse(&out).unwrap().orientation(),
            Some(Orientation::Rotate90)
        );
    }

    /// IFD1 (thumbnail dir) entries obey categories: keep the thumbnail image but
    /// drop camera → the thumbnail survives while IFD1's Make is stripped.
    #[test]
    fn ifd1_entries_filtered_by_category() {
        let exif = Exif {
            order: ByteOrder::Little,
            had_prefix: false,
            ifd0: vec![e(TAG_ORIENTATION, TIFF_SHORT, 1, &[6, 0])],
            exif_ifd: None,
            gps_ifd: None,
            ifd1: Some(vec![e(TAG_MAKE, TIFF_ASCII, 4, b"Cam\0")]), // camera tag in IFD1
            thumbnail: Some(&[0xFF, 0xD8, 0xFF, 0xD9]),
            text_encoding: TextEncoding::Ascii,
        };
        let out = exif
            .filtered(&ExifPolicy::KEEP_ALL.with_camera(Retention::Discard))
            .to_bytes();
        let y = Exif::parse(&out).unwrap();
        assert!(y.has_thumbnail(), "thumbnail image must be kept");
        assert!(
            !out.windows(2).any(|w| w == TAG_MAKE.to_le_bytes()),
            "IFD1 camera tag (Make) must be stripped"
        );
    }

    // ── From-scratch construction (Exif::new) ────────────────────────────────

    /// Build a fresh EXIF from nothing: new → set_copyright → to_bytes → parse.
    #[test]
    fn new_from_scratch_copyright_round_trips() {
        let mut exif = Exif::new(TextEncoding::Ascii);
        assert!(exif.copyright().is_none());
        exif.set_copyright("(c) 2026 Lilith");
        let blob = exif.to_bytes();
        let y = Exif::parse(&blob).expect("fresh blob parses");
        assert_eq!(y.copyright().unwrap(), "(c) 2026 Lilith");
        assert!(!y.has_gps() && !y.has_thumbnail());
        // Copyright is `rights`, so it survives even the web preset.
        let kept = y.filtered(&ExifPolicy::ATTRIBUTED_ORIENTATION).to_bytes();
        assert_eq!(
            Exif::parse(&kept).unwrap().copyright().unwrap(),
            "(c) 2026 Lilith"
        );
    }

    /// `Exif::default()` == empty `new()`, and an empty blob round-trips.
    #[test]
    fn new_default_empty_round_trips() {
        let blob = Exif::default().to_bytes();
        let y = Exif::parse(&blob).expect("empty blob parses");
        assert!(y.copyright().is_none() && !y.has_gps() && !y.has_thumbnail());
    }

    // ── Orientation injection (set_orientation) ──────────────────────────────

    /// From-scratch authoring: new → set_copyright + set_orientation →
    /// to_bytes — the "stamp Orientation + Copyright on an image that carried
    /// no EXIF" blob — parses back with both fields readable.
    #[test]
    fn set_orientation_adds_entry_from_scratch() {
        let mut exif = Exif::new(TextEncoding::Ascii);
        assert!(exif.orientation().is_none());
        exif.set_copyright("(c) Me");
        exif.set_orientation(Orientation::Rotate90);
        let blob = exif.to_bytes();
        let y = Exif::parse(&blob).expect("fresh blob parses");
        assert_eq!(y.orientation(), Some(Orientation::Rotate90));
        assert_eq!(y.copyright().unwrap(), "(c) Me");
        // The injected entry is the canonical 1-count SHORT, and authored
        // output is already a serializer fixpoint.
        let en = y.ifd0.iter().find(|e| e.tag == TAG_ORIENTATION).unwrap();
        assert_eq!((en.kind, en.count), (TIFF_SHORT, 1));
        assert_eq!(blob, y.to_bytes());
    }

    /// Replacing an existing tag preserves its TIFF type — SHORT stays SHORT,
    /// LONG stays LONG — and re-injecting after a policy dropped the tag
    /// exercises the add path on a big-endian tree.
    #[test]
    fn set_orientation_replaces_existing_preserving_kind() {
        let bytes = sample(ByteOrder::Big, false); // SHORT Orientation=Rotate90
        let mut x = Exif::parse(&bytes).unwrap();
        x.set_orientation(Orientation::Identity);
        let out = x.to_bytes();
        let y = Exif::parse(&out).unwrap();
        assert_eq!(y.orientation(), Some(Orientation::Identity));
        let en = y.ifd0.iter().find(|e| e.tag == TAG_ORIENTATION).unwrap();
        assert_eq!((en.kind, en.count), (TIFF_SHORT, 1));

        // Drop the tag, then inject into the (big-endian) tag-less tree.
        let mut nb = y.filtered(&ExifPolicy::KEEP_ALL.with_orientation(Retention::Discard));
        assert!(nb.orientation().is_none());
        nb.set_orientation(Orientation::Rotate180);
        let re = nb.to_bytes();
        assert_eq!(
            Exif::parse(&re).unwrap().orientation(),
            Some(Orientation::Rotate180)
        );

        // A LONG carrier (spec-tolerated) keeps its type on an explicit set.
        let mut long = Exif {
            order: ByteOrder::Little,
            had_prefix: false,
            ifd0: vec![e(TAG_ORIENTATION, TIFF_LONG, 1, &[3, 0, 0, 0])],
            exif_ifd: None,
            gps_ifd: None,
            ifd1: None,
            thumbnail: None,
            text_encoding: TextEncoding::Ascii,
        };
        long.set_orientation(Orientation::Rotate180);
        let lb = long.to_bytes();
        let z = Exif::parse(&lb).unwrap();
        assert_eq!(z.orientation(), Some(Orientation::Rotate180));
        let en = z.ifd0.iter().find(|e| e.tag == TAG_ORIENTATION).unwrap();
        assert_eq!((en.kind, en.count), (TIFF_LONG, 1));
    }

    /// A malformed non-integer Orientation carrier is replaced by the canonical
    /// SHORT entry on an explicit set — an authoring API must make the value
    /// readable (contrast `set_orientation_tag`, which leaves such carriers
    /// alone during reconciliation).
    #[test]
    fn set_orientation_replaces_non_integer_carrier() {
        let mut exif = Exif {
            order: ByteOrder::Little,
            had_prefix: false,
            ifd0: vec![e(TAG_ORIENTATION, TIFF_ASCII, 2, b"6\0")],
            exif_ifd: None,
            gps_ifd: None,
            ifd1: None,
            thumbnail: None,
            text_encoding: TextEncoding::Ascii,
        };
        assert!(exif.orientation().is_none(), "ASCII carrier is unreadable");
        exif.set_orientation(Orientation::Rotate270);
        let blob = exif.to_bytes();
        let y = Exif::parse(&blob).unwrap();
        assert_eq!(y.orientation(), Some(Orientation::Rotate270));
        let en = y.ifd0.iter().find(|e| e.tag == TAG_ORIENTATION).unwrap();
        assert_eq!((en.kind, en.count), (TIFF_SHORT, 1));
    }

    /// Regression for fuzz zencodec#97 (`Metadata::filtered` non-idempotent):
    /// the orientation-reconcile path (`set_orientation_tag`) must canonicalize
    /// `count` to 1 when it rewrites the value. A malformed source entry with
    /// `count > 1` left next to a 1-element value serializes as a dangling
    /// out-of-line offset that re-parse silently drops — so `filtered` went
    /// `Some` → `None` and lost the orientation on the second pass.
    #[test]
    fn reconcile_orientation_canonicalizes_count_97() {
        let mut x = Exif::new(TextEncoding::Ascii);
        // count=20 SHORT (40 declared bytes) but only a 2-byte value — exactly
        // the malformed shape the fuzzer produced.
        x.ifd0.push(e(TAG_ORIENTATION, TIFF_SHORT, 20, &[0, 1]));
        x.set_orientation_tag(Orientation::Rotate90);
        let en = x.ifd0.iter().find(|e| e.tag == TAG_ORIENTATION).unwrap();
        assert_eq!(en.count, 1, "reconcile must canonicalize orientation count");
        // Round-trips and is a serializer fixpoint (the property that broke).
        let b1 = x.to_bytes();
        let y = Exif::parse(&b1).expect("must re-parse");
        assert_eq!(y.orientation(), Some(Orientation::Rotate90));
        assert_eq!(b1, y.to_bytes(), "must be a serializer fixpoint");
    }

    /// Regression for fuzz zencodec#30/#96: when a GPS sub-IFD pointer is
    /// successfully extracted, a *duplicate* GPS pointer tag must not survive in
    /// IFD0 — else `to_bytes` re-emits it and on re-parse it shadows the
    /// synthesized pointer, dropping the real sub-IFD (gps presence drift) and
    /// breaking the serializer fixpoint. (A short/unusable pointer that was NOT
    /// extracted stays as data — see `short_subifd_pointer_is_preserved`.)
    #[test]
    fn duplicate_gps_pointer_stripped_on_parse_30() {
        // MM TIFF, IFD0 @ 8 with two LONG GPS pointers (both usable), each → an
        // empty GPS IFD. take_pointer extracts the first (gps_ifd = Some), so the
        // second is a duplicate that must be stripped. Offsets: IFD0 spans
        // 0x08..0x26 (count + 2×12 + next), empty GPS IFDs at 0x26 and 0x2c.
        let mut t = vec![b'M', b'M', 0, 0x2a, 0, 0, 0, 8];
        t.extend_from_slice(&[0, 2]); // 2 entries
        t.extend_from_slice(&[0x88, 0x25, 0, 4, 0, 0, 0, 1, 0, 0, 0, 0x26]); // GPS LONG -> 0x26
        t.extend_from_slice(&[0x88, 0x25, 0, 4, 0, 0, 0, 1, 0, 0, 0, 0x2c]); // GPS LONG -> 0x2c (dup)
        t.extend_from_slice(&[0, 0, 0, 0]); // IFD0 next = 0
        t.extend_from_slice(&[0, 0, 0, 0, 0, 0]); // empty GPS IFD @ 0x26 (count0, next0)
        t.extend_from_slice(&[0, 0, 0, 0, 0, 0]); // empty GPS IFD @ 0x2c
        let x = Exif::parse(&t).expect("fixture must parse");
        assert!(x.has_gps(), "first GPS pointer should be extracted");
        assert!(
            !x.ifd0.iter().any(|e| e.tag == TAG_GPS_IFD),
            "duplicate GPS structural tag leaked into ifd0"
        );
        let b1 = x.to_bytes();
        let y = Exif::parse(&b1).expect("must re-parse");
        assert_eq!(x.has_gps(), y.has_gps(), "gps presence must round-trip");
        assert_eq!(b1, y.to_bytes(), "must be a serializer fixpoint");
    }
}
