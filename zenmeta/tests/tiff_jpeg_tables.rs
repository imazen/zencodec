#![cfg(feature = "inventory")]

use core::cell::Cell;
use zencodec::ImageFormat;
use zencodec::inventory::{Disposition, Inventory, MetadataKind, PartKind, PartTag};
use zenmeta::ifd::walk::{self, Cand, Entry, Fate, Ifd, Kind, Rules, Walk};

struct Structural;

impl Rules for Structural {
    fn follows(&self, _: &Walk<'_>, _: &Ifd, _: Kind, _: &Entry) -> bool {
        true
    }
    fn follows_next(&self, _: &Walk<'_>, _: &Ifd, _: Kind) -> bool {
        true
    }
    fn ifd(&self, _: &Walk<'_>, _: &Ifd) -> Fate {
        Fate::new(Disposition::Structure)
    }
    fn entry(&self, _: &Walk<'_>, _: &Ifd, _: usize) -> Fate {
        Fate::new(Disposition::Structure)
    }
    fn image_data(&self, _: &Walk<'_>, _: &Ifd) -> Fate {
        Fate::new(Disposition::ImageData)
    }
    fn jpeg_stream(&self, _: &Walk<'_>, _: &Ifd) -> Fate {
        Fate::new(Disposition::Skipped)
    }
}

struct DecoderRules {
    polls: Cell<usize>,
    cancel_after: Cell<usize>,
}

impl Default for DecoderRules {
    fn default() -> Self {
        Self {
            polls: Cell::new(0),
            cancel_after: Cell::new(usize::MAX),
        }
    }
}

impl Rules for DecoderRules {
    fn follows(&self, _: &Walk<'_>, _: &Ifd, _: Kind, _: &Entry) -> bool {
        true
    }
    fn follows_next(&self, _: &Walk<'_>, _: &Ifd, _: Kind) -> bool {
        true
    }
    fn ifd(&self, w: &Walk<'_>, i: &Ifd) -> Fate {
        Structural.ifd(w, i)
    }
    fn entry(&self, w: &Walk<'_>, i: &Ifd, e: usize) -> Fate {
        Structural.entry(w, i, e)
    }
    fn image_data(&self, w: &Walk<'_>, i: &Ifd) -> Fate {
        Structural.image_data(w, i)
    }
    fn jpeg_stream(&self, w: &Walk<'_>, i: &Ifd) -> Fate {
        Structural.jpeg_stream(w, i)
    }
    fn jpeg_table(&self, _: &Walk<'_>, _: &Ifd, tag: u16) -> Fate {
        Fate::new(match tag {
            519 => Disposition::ImageData,
            520 => Disposition::Skipped,
            521 => Disposition::Unknown,
            _ => panic!("not a JPEG table tag"),
        })
    }
    fn cancelled(&self) -> bool {
        let n = self.polls.get();
        self.polls.set(n + 1);
        n >= self.cancel_after.get()
    }
}

struct Fixture {
    bytes: Vec<u8>,
    base: usize,
    big: bool,
    le: bool,
}

impl Fixture {
    fn new(big: bool, le: bool, base: usize) -> Self {
        let header = if big { 16 } else { 8 };
        let count = if big { 8 } else { 2 };
        let entry = if big { 20 } else { 12 };
        let offset = if big { 8 } else { 4 };
        let mut f = Self {
            bytes: vec![0; base + header + count + 3 * entry + offset],
            base,
            big,
            le,
        };
        f.bytes[base..base + 2].copy_from_slice(if le { b"II" } else { b"MM" });
        f.integer(base + 2, if big { 43 } else { 42 }, 2);
        if big {
            f.integer(base + 4, 8, 2);
            f.integer(base + 8, header as u64, 8);
        } else {
            f.integer(base + 4, header as u64, 4);
        }
        f.integer(base + header, 3, count);
        f
    }

    fn integer(&mut self, at: usize, value: u64, n: usize) {
        let bytes = if self.le {
            value.to_le_bytes()
        } else {
            value.to_be_bytes()
        };
        let source = if self.le {
            &bytes[..n]
        } else {
            &bytes[8 - n..]
        };
        self.bytes[at..at + n].copy_from_slice(source);
    }

    fn put(&mut self, bytes: &[u8]) -> u64 {
        let offset = (self.bytes.len() - self.base) as u64;
        self.bytes.extend_from_slice(bytes);
        offset
    }

    fn entry(&mut self, index: usize, tag: u16, values: &[u64]) {
        let (header, count, entry, offset) = if self.big {
            (16, 8, 20, 8)
        } else {
            (8, 2, 12, 4)
        };
        let at = self.base + header + count + index * entry;
        self.integer(at, u64::from(tag), 2);
        self.integer(at + 2, if self.big { 16 } else { 4 }, 2);
        self.integer(at + 4, values.len() as u64, offset);
        let field = at + 4 + offset;
        let data = if values.len() > 1 {
            let relative = self.put(&vec![0; values.len() * offset]);
            self.integer(field, relative, offset);
            self.base + relative as usize
        } else {
            field
        };
        for (i, &value) in values.iter().enumerate() {
            self.integer(data + i * offset, value, offset);
        }
    }

    fn inventory(&self) -> Inventory {
        let mut w = walk::walk(
            &self.bytes,
            self.base as u64,
            self.bytes.len() as u64,
            &Structural,
        );
        assert!(w.header_ok);
        let placement = walk::place(&mut w, &Structural, vec![]);
        assert!(placement.ifd0_ok);
        assert!(!placement.budget_hit);
        emit(&self.bytes, placement, w.cancelled)
    }
}

fn emit(data: &[u8], placement: walk::Placement, cancelled: bool) -> Inventory {
    let gap = if placement.budget_hit || cancelled {
        Disposition::Unknown
    } else {
        Disposition::Unreferenced
    };
    let mut inv = Inventory::new(ImageFormat::Tiff, data.len() as u64);
    walk::emit_top(
        &mut inv,
        data,
        placement.parts,
        data.len() as u64,
        (gap, None),
    )
    .unwrap();
    inv.fill_gaps(None, gap).unwrap();
    inv.validate().unwrap();
    inv
}

#[test]
fn external_jpeg_tables_are_referenced_without_claiming_decoder_use_or_privacy() {
    for big in [false, true] {
        for le in [false, true] {
            for base in [0, 13] {
                let mut f = Fixture::new(big, le, base);
                let q = f.put(&[17; 64]);
                let mut huffman = vec![2; 16];
                huffman.extend(0..32);
                let dc = f.put(&huffman);
                let ac = f.put(&huffman);
                let tail = f.put(b"opaque-private-tail");
                f.entry(0, 519, &[q, 0, q]);
                f.entry(1, 520, &[dc]);
                f.entry(2, 521, &[ac]);
                let inv = f.inventory();
                for (offset, len) in [(q, 64), (dc, 48), (ac, 48)] {
                    for byte in base as u64 + offset..base as u64 + offset + len {
                        let p = inv
                            .parts()
                            .iter()
                            .find(|p| p.range().contains(&byte))
                            .unwrap();
                        assert_eq!(
                            p.disposition(),
                            Disposition::Unknown,
                            "big={big} le={le} base={base} byte={byte}: {inv}"
                        );
                    }
                }
                let byte = base as u64 + tail + 2;
                let p = inv
                    .parts()
                    .iter()
                    .find(|p| p.range().contains(&byte))
                    .unwrap();
                assert_eq!(p.disposition(), Disposition::Unreferenced, "{inv}");
            }
        }
    }
}

#[test]
fn table_truncation_nulls_overflow_and_maximum_declared_symbol_count_are_bounded() {
    for big in [false, true] {
        for le in [false, true] {
            for tag in [519, 520, 521] {
                let size = if tag == 519 { 64 } else { 32 };
                for available in 1..=size {
                    let mut f = Fixture::new(big, le, 7);
                    let table = f.put(&vec![1; available]);
                    for (index, entry_tag) in [519, 520, 521].into_iter().enumerate() {
                        f.entry(
                            index,
                            entry_tag,
                            &[if entry_tag == tag { table } else { 0 }],
                        );
                    }
                    let inv = f.inventory();
                    let byte = f.base as u64 + table;
                    let p = inv
                        .parts()
                        .iter()
                        .find(|p| p.range().contains(&byte))
                        .unwrap();
                    assert_eq!(
                        p.disposition(),
                        if available < size {
                            Disposition::Malformed
                        } else {
                            Disposition::Unknown
                        },
                        "tag={tag} available={available}: {inv}"
                    );
                    assert_eq!(p.range(), byte..byte + available as u64);
                }
            }

            let mut f = Fixture::new(big, le, 13);
            let mut huffman = vec![255; 16];
            huffman.resize(16 + 16 * 255, 0);
            let dc = f.put(&huffman);
            f.entry(0, 519, &[0]);
            f.entry(1, 520, &[dc]);
            f.entry(2, 521, &[if big { u64::MAX } else { u64::from(u32::MAX) }]);
            let inv = f.inventory();
            let p = inv
                .parts()
                .iter()
                .find(|p| p.range().start == f.base as u64 + dc)
                .unwrap();
            assert_eq!(p.len(), huffman.len() as u64);
            assert_eq!(p.disposition(), Disposition::Unknown);
            assert!(
                inv.parts()
                    .iter()
                    .any(|p| p.detail().is_some_and(|s| s.contains(if big {
                        "overflows"
                    } else {
                        "past the end"
                    })))
            );
            assert!(
                !inv.parts()
                    .iter()
                    .any(|p| p.kind() == PartKind::Extent && p.range().contains(&(f.base as u64)))
            );
        }
    }
}

#[test]
fn decoder_table_rules_are_separate_and_metadata_overlap_keeps_the_stronger_fate() {
    let mut f = Fixture::new(true, true, 0);
    let q = f.put(&[1; 64]);
    let dc = f.put(&[0; 16]);
    let ac = f.put(&[0; 16]);
    f.entry(0, 519, &[q]);
    f.entry(1, 520, &[dc]);
    f.entry(2, 521, &[ac]);
    let rules = DecoderRules::default();
    let mut w = walk::walk(&f.bytes, 0, f.bytes.len() as u64, &rules);
    let metadata = Cand::new(
        q + 5..q + 19,
        PartKind::Field,
        PartTag::Code(700),
        Disposition::Metadata(MetadataKind::Xmp),
        "explicit overlapping metadata evidence".into(),
    );
    let placed = walk::place(&mut w, &rules, vec![metadata]);
    let inv = emit(&f.bytes, placed, false);
    for (byte, expected) in [
        (q, Disposition::ImageData),
        (q + 7, Disposition::Metadata(MetadataKind::Xmp)),
        (dc, Disposition::Skipped),
        (ac, Disposition::Unknown),
    ] {
        let p = inv
            .parts()
            .iter()
            .find(|p| p.range().contains(&byte))
            .unwrap();
        assert_eq!(p.disposition(), expected, "{inv}");
    }
}

#[test]
fn repeated_tables_use_scan_budget_and_cancel_with_unexamined_bytes_unknown() {
    let mut f = Fixture::new(false, false, 0);
    let dc = f.put(&[0; 16]);
    f.entry(0, 519, &[0]);
    f.entry(1, 520, &[dc; 128]);
    f.entry(2, 521, &[0]);
    let rules = DecoderRules::default();
    let mut w = walk::walk(&f.bytes, 0, f.bytes.len() as u64, &rules);
    while w.take_scan(16) {}
    let placed = walk::place(&mut w, &rules, vec![]);
    assert!(placed.budget_hit);
    let inv = emit(&f.bytes, placed, w.cancelled);
    let p = inv
        .parts()
        .iter()
        .find(|p| p.range().contains(&dc))
        .unwrap();
    assert_eq!(p.disposition(), Disposition::Unknown, "{inv}");

    let mut w = walk::walk(&f.bytes, 0, f.bytes.len() as u64, &rules);
    rules.polls.set(0);
    rules.cancel_after.set(3);
    let placed = walk::place(&mut w, &rules, vec![]);
    assert!(w.cancelled);
    assert!(
        rules.polls.get() < 16,
        "cancellation was not bounded by the current table"
    );
    let inv = emit(&f.bytes, placed, w.cancelled);
    inv.validate().unwrap();
}
