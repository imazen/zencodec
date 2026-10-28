//! Native consumption controls for the parity fork, not a complete zentiff inventory.
use std::{
    cell::RefCell,
    io::{self, Cursor, Read, Seek, SeekFrom},
    ops::Range,
    path::PathBuf,
    rc::Rc,
    sync::OnceLock,
};
use tiff::decoder::{Decoder, DecodingSampleBuffer};
use zencodec::inventory::{Disposition, PartTag};
use zenmeta::ifd::walk::{self, Count, Entry, ExtentRule, Fate, Ifd, Kind, Rules, Walk};

const FORK: &str = "10a3b6b2fa88eb5943525066a989ea9e64e644d5";

fn fixture(name: &str) -> Vec<u8> {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    let root = ROOT.get_or_init(|| {
        codec_corpus::Corpus::new()
            .unwrap()
            .tar_url(
                "image-tiff-parity-10a3b6b2",
                "https://codeload.github.com/lilith/image-tiff/tar.gz/10a3b6b2fa88eb5943525066a989ea9e64e644d5",
            )
            .expect("download immutable TIFF parity corpus through codec-corpus")
    });
    let path = root.join("tests/libtiffpic").join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[derive(Default)]
struct Reads {
    ranges: Vec<Range<u64>>,
    seek_end: usize,
}

struct Traced<'a> {
    input: Cursor<&'a [u8]>,
    reads: Rc<RefCell<Reads>>,
}
impl Read for Traced<'_> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let start = self.input.position();
        let n = self.input.read(out)?;
        if n != 0 {
            self.reads.borrow_mut().ranges.push(start..start + n as u64);
        }
        Ok(n)
    }
}
impl Seek for Traced<'_> {
    fn seek(&mut self, at: SeekFrom) -> io::Result<u64> {
        if matches!(at, SeekFrom::End(_)) {
            self.reads.borrow_mut().seek_end += 1;
        }
        self.input.seek(at)
    }
}

fn decode(data: &[u8]) -> (Result<Vec<u8>, tiff::TiffError>, Reads) {
    let reads = Rc::new(RefCell::new(Reads::default()));
    let input = Traced {
        input: Cursor::new(data),
        reads: reads.clone(),
    };
    let result = Decoder::open(input)
        .and_then(|mut d| {
            d.next_image()?;
            d.read_image()
        })
        .map(|v| match v {
            DecodingSampleBuffer::U8(v) => v,
            other => panic!("8-bit JPEG fixture changed sample type: {other:?}"),
        });
    (result, Rc::try_unwrap(reads).ok().unwrap().into_inner())
}

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
        Fate::new(Disposition::Unknown)
    }
    fn jpeg_stream(&self, _: &Walk<'_>, _: &Ifd) -> Fate {
        Fate::new(Disposition::Unknown)
    }
}

// A narrow, executing rule for this pinned decoder: an OJPEG stream length of
// zero reads to EOF. It intentionally does not classify the stream as PII-free,
// pixel data, or necessary merely because the decoder reads it.
struct OldJpeg;
impl Rules for OldJpeg {
    fn follows(&self, w: &Walk<'_>, i: &Ifd, k: Kind, e: &Entry) -> bool {
        Structural.follows(w, i, k, e)
    }
    fn follows_next(&self, w: &Walk<'_>, i: &Ifd, k: Kind) -> bool {
        Structural.follows_next(w, i, k)
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
    fn extents(&self, w: &Walk<'_>, i: &Ifd) -> Vec<ExtentRule> {
        let mut rules = Structural.extents(w, i);
        let value = |tag| w.find(i, tag).and_then(|e| w.single_uint(e));
        if value(259) == Some(6) && value(514).unwrap_or(0) == 0 {
            let stream = rules.iter_mut().find(|r| r.offsets == 513).unwrap();
            stream.counts = Count::ToEnd;
        }
        rules
    }
}

fn covered(reads: &Reads, range: &Range<u64>) -> bool {
    range
        .clone()
        .all(|byte| reads.ranges.iter().any(|r| r.contains(&byte)))
}

fn emit(name: &str, value: &serde_json::Value) {
    let dir = PathBuf::from(
        std::env::var_os("SPATIAL_INVENTORY_ARTIFACTS")
            .expect("set explicit fresh TIFF inventory artifact directory"),
    );
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let mut file = std::fs::File::options()
        .write(true)
        .create_new(true)
        .open(path)
        .unwrap();
    serde_json::to_writer_pretty(&mut file, value).unwrap();
}

#[test]
fn tiff_inventory_native_external_tables_are_read_and_affect_decoding() {
    let data = fixture("zackthecat.tif");
    let (pixels, reads) = decode(&data);
    let pixels = pixels.expect("pinned native OJPEG external-table decode");
    let mut w = walk::walk(&data, 0, data.len() as u64, &Structural);
    let placed = walk::place(&mut w, &Structural, vec![]);
    assert!(!placed.budget_hit && !w.cancelled);
    let mut receipts = vec![];
    for tag in [519, 520, 521] {
        let tables: Vec<_> = placed
            .parts
            .iter()
            .filter(|p| {
                p.kind == zencodec::inventory::PartKind::Extent && p.tag == PartTag::Code(tag)
            })
            .collect();
        assert!(!tables.is_empty(), "missing tables for {tag}");
        for table in tables {
            assert_eq!(table.disp, Disposition::Unknown);
            assert!(
                covered(&reads, &table.range),
                "native reads do not cover table {tag}: {:?}",
                table.range
            );
            let mut changed = data.clone();
            changed[table.range.start as usize..table.range.end as usize].fill(0);
            let (mutated, _) = decode(&changed);
            let changes_output = mutated.as_ref().is_ok_and(|v| v != &pixels);
            assert!(
                mutated.is_err() || changes_output,
                "table {tag} mutation was semantically irrelevant"
            );
            receipts.push(serde_json::json!({"tag":tag,"range":[table.range.start,table.range.end],"fully_read":true,"mutation_rejected":mutated.is_err(),"mutation_changed_pixels":changes_output}));
        }
    }
    emit(
        "external-tables.json",
        &serde_json::json!({"fork":FORK,"fixture":"tests/libtiffpic/zackthecat.tif","tables":receipts,"read_calls":reads.ranges.len(),"seek_end_calls":reads.seek_end,"decoded_sample_bytes":pixels.len(),"claim":"native table consumption and mutation sensitivity; not independent pixel accuracy or privacy"}),
    );
}

#[test]
fn tiff_inventory_zero_length_shared_stream_reads_opaque_tail_without_changing_pixels() {
    let mut data = fixture("smallliz.tif");
    let original_len = data.len();
    let (expected, _) = decode(&data);
    let expected = expected.expect("pinned native whole-JPEG stream decode");
    let w = walk::walk(&data, 0, data.len() as u64, &Structural);
    let ifd = &w.ifds[0];
    assert_eq!(w.single_uint(w.find(ifd, 259).unwrap()), Some(6));
    let length = w
        .find(ifd, 514)
        .expect("fixture carries JPEGInterchangeFormatLength");
    assert_eq!(length.typ, 4);
    assert_eq!(length.count, 1);
    assert!(w.single_uint(length).unwrap() > 0);
    let field = length.field as usize;
    let stream = w.single_uint(w.find(ifd, 513).unwrap()).unwrap();
    data.extend_from_slice(b"opaque-description-that-a-PII-policy-must-not-certify");
    let tail = original_len as u64..data.len() as u64;
    let (bounded, bounded_reads) = decode(&data);
    assert_eq!(bounded.unwrap(), expected);
    assert!(
        !covered(&bounded_reads, &tail),
        "bounded JIF unexpectedly reads the appended tail"
    );
    data[field..field + 4].fill(0);
    let (unbounded, reads) = decode(&data);
    assert_eq!(unbounded.unwrap(), expected);
    assert!(covered(&reads, &tail));
    assert!(reads.seek_end > 0);
    let mut w = walk::walk(&data, 0, data.len() as u64, &OldJpeg);
    let placed = walk::place(&mut w, &OldJpeg, vec![]);
    for byte in tail.clone() {
        let p = placed
            .parts
            .iter()
            .find(|p| p.range.contains(&byte))
            .expect("private EOF rule covers opaque tail");
        assert_eq!(p.disp, Disposition::Unknown);
    }
    emit(
        "shared-stream-tail.json",
        &serde_json::json!({"fork":FORK,"fixture":"tests/libtiffpic/smallliz.tif","stream_start":stream,"original_file_bytes":original_len,"tail_range":[tail.start,tail.end],"bounded_reads_tail":false,"zero_length_reads_tail":true,"pixels_unchanged":true,"seek_end_calls":reads.seek_end,"claim":"bytes read are not necessarily needed for rendering and do not establish privacy"}),
    );
}

#[test]
fn tiff_inventory_predictor_rejection_depends_on_compression() {
    use tiff::encoder::{Compression, TiffEncoder, colortype::Gray8};
    use tiff::tags::Tag;
    let pixels = [0, 7, 255, 16, 128, 91, 42, 3];
    let mut receipts = vec![];
    for compression in [Compression::Uncompressed, Compression::Lzw] {
        let mut out = Cursor::new(Vec::new());
        {
            let mut encoder = TiffEncoder::new(&mut out)
                .unwrap()
                .with_compression(compression);
            let mut image = encoder.new_image::<Gray8>(4, 2).unwrap();
            image.encoder().write_tag(Tag::Predictor, u16::MAX).unwrap();
            image.write_data(&pixels).unwrap();
        }
        let data = out.into_inner();
        let w = walk::walk(&data, 0, data.len() as u64, &Structural);
        let ifd = &w.ifds[0];
        assert_eq!(
            w.single_uint(w.find(ifd, 317).unwrap()),
            Some(u64::from(u16::MAX))
        );
        let compression_tag = w.single_uint(w.find(ifd, 259).unwrap()).unwrap();
        let (decoded, _) = decode(&data);
        match compression_tag {
            1 => assert_eq!(decoded.unwrap(), pixels),
            5 => assert!(matches!(
                decoded,
                Err(tiff::TiffError::FormatError(
                    tiff::TiffFormatError::UnknownPredictor(u16::MAX)
                ))
            )),
            _ => panic!("fixture changed compression"),
        }
        receipts.push(serde_json::json!({"compression":compression_tag,"predictor":u16::MAX,"decode_succeeds":compression_tag == 1}));
    }
    emit(
        "predictor-context.json",
        &serde_json::json!({"fork":FORK,"cases":receipts,"claim":"inventory must mirror compression-specific rejection; unsupported tags alone do not determine decode failure"}),
    );
}
