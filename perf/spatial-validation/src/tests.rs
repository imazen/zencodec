use crate::tiff_region::{self, Allowances, Rect, Selection};
use std::{
    cell::RefCell,
    io::{self, BufReader, Cursor, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    rc::Rc,
    sync::OnceLock,
};
use tiff::{
    decoder::{Decoder, TiffHeader},
    encoder::{DirectoryEncoder, TiffEncoder, TiffKind},
    tags::Tag,
};
use zencodec::source::ReadAt;
use zencodec_io::ReadAtReader;

#[derive(Clone)]
struct Raster {
    width: u32,
    height: u32,
    bytes: Vec<u8>,
}
fn raster(width: u32, height: u32, salt: u8) -> Raster {
    Raster {
        width,
        height,
        bytes: (0..width * height * 3)
            .map(|i| ((i * 61 + i / 19 + u32::from(salt)) % 251) as u8)
            .collect(),
    }
}
fn crop(r: &Raster, roi: Rect) -> Vec<u8> {
    (roi.y..roi.y + roi.height)
        .flat_map(|y| {
            let at = (y * r.width + roi.x) as usize * 3;
            r.bytes[at..at + roi.width as usize * 3].iter().copied()
        })
        .collect()
}

// Native directory encoder writes all grammar and byte order. This fixture
// supplies tags/pixels, not a second TIFF serializer or decoder.
fn tiled<W: Write + Seek, K: TiffKind>(
    mut dir: DirectoryEncoder<'_, W, K>,
    r: &Raster,
    planar: bool,
    reduced: bool,
    child: Option<u32>,
) -> u64 {
    let mut offsets = Vec::new();
    let mut sizes = Vec::new();
    for plane in 0..if planar { 3 } else { 1 } {
        for ty in 0..r.height.div_ceil(16) {
            for tx in 0..r.width.div_ceil(16) {
                let channels = if planar { 1 } else { 3 };
                let mut px = vec![0x7f; 16 * 16 * channels];
                for y in 0..16u32 {
                    for x in 0..16u32 {
                        if tx * 16 + x < r.width && ty * 16 + y < r.height {
                            let src = ((ty * 16 + y) * r.width + tx * 16 + x) as usize * 3;
                            let dst = (y * 16 + x) as usize * channels;
                            if planar {
                                px[dst] = r.bytes[src + plane];
                            } else {
                                px[dst..dst + 3].copy_from_slice(&r.bytes[src..src + 3]);
                            }
                        }
                    }
                }
                offsets.push(u32::try_from(dir.write_data(px.as_slice()).unwrap()).unwrap());
                sizes.push(px.len() as u32);
            }
        }
    }
    dir.write_tag(Tag::ImageWidth, r.width).unwrap();
    dir.write_tag(Tag::ImageLength, r.height).unwrap();
    dir.write_tag(Tag::BitsPerSample, &[8u16; 3][..]).unwrap();
    dir.write_tag(Tag::SamplesPerPixel, 3u16).unwrap();
    dir.write_tag(Tag::PhotometricInterpretation, 2u16).unwrap();
    dir.write_tag(Tag::Compression, 1u16).unwrap();
    dir.write_tag(Tag::PlanarConfiguration, if planar { 2u16 } else { 1 })
        .unwrap();
    dir.write_tag(Tag::TileWidth, 16u32).unwrap();
    dir.write_tag(Tag::TileLength, 16u32).unwrap();
    dir.write_tag(Tag::TileOffsets, offsets.as_slice()).unwrap();
    dir.write_tag(Tag::TileByteCounts, sizes.as_slice())
        .unwrap();
    dir.write_tag(Tag::Orientation, 1u16).unwrap();
    dir.write_tag(Tag::NewSubfileType, u32::from(reduced))
        .unwrap();
    if let Some(child) = child {
        dir.write_tag(Tag::SubIfd, &[child][..]).unwrap();
    }
    dir.finish_with_offsets().unwrap().pointer.0
}
fn fixture(bigtiff: bool, planar: bool, reduced: bool) -> (Vec<u8>, [Raster; 3]) {
    let refs = [raster(35, 29, 0), raster(19, 17, 17), raster(21, 25, 73)];
    let mut cursor = Cursor::new(Vec::new());
    fn write<K: TiffKind>(
        mut encoder: TiffEncoder<&mut Cursor<Vec<u8>>, K>,
        refs: &[Raster; 3],
        planar: bool,
        reduced: bool,
    ) {
        let child = tiled(
            encoder.extra_directory().unwrap(),
            &refs[1],
            planar,
            reduced,
            None,
        );
        tiled(
            encoder.image_directory().unwrap(),
            &refs[0],
            planar,
            false,
            Some(child as u32),
        );
        tiled(
            encoder.image_directory().unwrap(),
            &refs[2],
            planar,
            false,
            None,
        );
    }
    if bigtiff {
        write(
            TiffEncoder::new_big(&mut cursor).unwrap(),
            &refs,
            planar,
            reduced,
        );
    } else {
        write(
            TiffEncoder::new(&mut cursor).unwrap(),
            &refs,
            planar,
            reduced,
        );
    }
    (cursor.into_inner(), refs)
}

#[derive(Debug)]
enum SourceFault {
    VersionChanged,
    Injected,
    Admission,
    Eof,
}
impl std::fmt::Display for SourceFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for SourceFault {}
#[derive(Default)]
struct Ledger {
    calls: usize,
    bytes: usize,
    fail: Option<usize>,
    changed: bool,
    invalidated: bool,
}
#[derive(Clone)]
struct Source {
    bytes: Rc<Vec<u8>>,
    ledger: Rc<RefCell<Ledger>>,
    cap: usize,
}
impl Source {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes: Rc::new(bytes),
            ledger: Rc::new(RefCell::new(Ledger::default())),
            cap: 100_000,
        }
    }
    fn validate(&self) -> Result<(), SourceFault> {
        let mut l = self.ledger.borrow_mut();
        if l.changed {
            l.invalidated = true;
        }
        if l.invalidated {
            Err(SourceFault::VersionChanged)
        } else {
            Ok(())
        }
    }
}
impl ReadAt for Source {
    type Error = SourceFault;
    fn len(&mut self) -> Result<u64, Self::Error> {
        self.validate()?;
        Ok(self.bytes.len() as u64)
    }
    fn read_at(&mut self, at: u64, out: &mut [u8]) -> Result<usize, Self::Error> {
        self.validate()?;
        let mut l = self.ledger.borrow_mut();
        if l.calls == self.cap {
            return Err(SourceFault::Admission);
        }
        l.calls += 1;
        if l.fail == Some(l.calls) {
            if let Some(b) = out.first_mut() {
                *b = 0xff;
            }
            return Err(SourceFault::Injected);
        }
        let start = usize::try_from(at)
            .unwrap_or(usize::MAX)
            .min(self.bytes.len());
        let n = out.len().min(self.bytes.len() - start);
        out[..n].copy_from_slice(&self.bytes[start..start + n]);
        l.bytes += n;
        Ok(n)
    }
    fn read_exact_at(&mut self, at: u64, out: &mut [u8]) -> Result<(), Self::Error> {
        if self.read_at(at, out)? == out.len() {
            Ok(())
        } else {
            Err(SourceFault::Eof)
        }
    }
}
fn selected(levels: &[u32]) -> Selection<'_> {
    Selection {
        page: 0,
        levels,
        region: Rect {
            x: 13,
            y: 12,
            width: 6,
            height: 5,
        },
    }
}

mod image_selection {
    use super::*;
    use crate::{
        image_items::{Composition, RelationKind, SelectionError},
        tiff_items::TiffItems,
    };

    fn nested_fixture(big: bool, planar: bool, cycle: bool) -> (Vec<u8>, [Raster; 3]) {
        let refs = [raster(35, 29, 13), raster(21, 25, 39), raster(19, 17, 73)];
        let mut cursor = Cursor::new(Vec::new());
        fn write<K: TiffKind>(
            mut encoder: TiffEncoder<&mut Cursor<Vec<u8>>, K>,
            refs: &[Raster; 3],
            planar: bool,
            cycle: bool,
        ) -> u64 {
            // Deliberately not ReducedImage: containment alone cannot select a level.
            let leaf = tiled(
                encoder.extra_directory().unwrap(),
                &refs[2],
                planar,
                false,
                cycle.then_some(0x0bad_cafe),
            );
            let branch = tiled(
                encoder.extra_directory().unwrap(),
                &refs[1],
                planar,
                false,
                Some(u32::try_from(leaf).unwrap()),
            );
            let root = tiled(
                encoder.image_directory().unwrap(),
                &refs[0],
                planar,
                false,
                Some(u32::try_from(branch).unwrap()),
            );
            // The second top-level image shares the leaf: this is a graph, not a tree.
            tiled(
                encoder.image_directory().unwrap(),
                &refs[1],
                planar,
                false,
                Some(u32::try_from(leaf).unwrap()),
            );
            root
        }
        let root = if big {
            write(
                TiffEncoder::new_big(&mut cursor).unwrap(),
                &refs,
                planar,
                cycle,
            )
        } else {
            write(TiffEncoder::new(&mut cursor).unwrap(), &refs, planar, cycle)
        };
        let mut bytes = cursor.into_inner();
        if cycle {
            // Corrupt the native encoder's unique sentinel link, not its grammar.
            // The encoder on this little-endian test host writes this inline LONG.
            assert_eq!(&bytes[..2], b"II");
            let marker = 0x0bad_cafe_u32.to_le_bytes();
            let hits: Vec<_> = bytes
                .windows(4)
                .enumerate()
                .filter_map(|(i, b)| (b == marker).then_some(i))
                .collect();
            assert_eq!(hits.len(), 1);
            bytes[hits[0]..hits[0] + 4]
                .copy_from_slice(&u32::try_from(root).unwrap().to_le_bytes());
        }
        (bytes, refs)
    }

    #[test]
    fn native_catalog_preserves_nested_shared_subifd_identity_and_pixels() {
        for big in [false, true] {
            for planar in [false, true] {
                let (bytes, refs) = nested_fixture(big, planar, false);
                let mut source =
                    TiffItems::open(Cursor::new(bytes), Allowances::default()).unwrap();
                let mut pages = source.images();
                let first = source.next(&mut pages).unwrap().unwrap();
                let second = source.next(&mut pages).unwrap().unwrap();
                assert!(source.next(&mut pages).unwrap().is_none());
                assert_eq!(first.kind, RelationKind::TopLevel);
                assert!(first.parent.is_none());
                let mut children = source.children(&first.image).unwrap();
                let branch = source.next(&mut children).unwrap().unwrap();
                assert_eq!(branch.kind, RelationKind::SubImage);
                assert_eq!(branch.parent.as_ref(), Some(&first.image));
                assert!(source.next(&mut children).unwrap().is_none());
                let mut children = source.children(&branch.image).unwrap();
                let leaf = source.next(&mut children).unwrap().unwrap();
                let mut shared = source.children(&second.image).unwrap();
                assert_eq!(source.next(&mut shared).unwrap().unwrap().image, leaf.image);
                for (image, raster) in [
                    (&first.image, &refs[0]),
                    (&branch.image, &refs[1]),
                    (&leaf.image, &refs[2]),
                ] {
                    let roi = selected(&[]).region;
                    let output = source
                        .decode(image, roi, Composition::Composited, || Ok(()))
                        .unwrap();
                    assert_eq!(output.bytes, crop(raster, roi));
                    assert_eq!(output.orientation, 1);
                }
            }
        }
    }

    #[test]
    fn foreign_selection_cursor_and_layer_requirement_refuse_before_io() {
        let (bytes, _) = nested_fixture(false, false, false);
        let observed = Source::new(bytes.clone());
        let mut first =
            TiffItems::open(ReadAtReader::new(observed.clone()), Allowances::default()).unwrap();
        let mut second = TiffItems::open(Cursor::new(bytes), Allowances::default()).unwrap();
        let mut pages = first.images();
        let image = first.next(&mut pages).unwrap().unwrap().image;
        let other = second.images();
        let mut foreign = other;
        let before = observed.ledger.borrow().calls;
        let error = first.next(&mut foreign).unwrap_err();
        assert_eq!(
            zencodec::find_cause::<SelectionError>(&*error),
            Some(&SelectionError::ForeignCursor)
        );
        let error = first
            .decode(
                &image,
                selected(&[]).region,
                Composition::Uncomposited,
                || Ok(()),
            )
            .unwrap_err();
        assert_eq!(
            zencodec::find_cause::<SelectionError>(&*error),
            Some(&SelectionError::UnsupportedComposition)
        );
        assert_eq!(observed.ledger.borrow().calls, before);
        let error = second
            .decode(
                &image,
                selected(&[]).region,
                Composition::Composited,
                || Ok(()),
            )
            .unwrap_err();
        assert_eq!(
            zencodec::find_cause::<SelectionError>(&*error),
            Some(&SelectionError::ForeignImage)
        );
    }

    #[test]
    fn native_subifd_cycle_and_discovery_limit_are_explicit_failures() {
        for big in [false, true] {
            let (bytes, _) = nested_fixture(big, false, true);
            let mut source = TiffItems::open(Cursor::new(bytes), Allowances::default()).unwrap();
            let mut cursor = source.images();
            let mut item = source.next(&mut cursor).unwrap().unwrap().image;
            for _ in 0..2 {
                cursor = source.children(&item).unwrap();
                item = source.next(&mut cursor).unwrap().unwrap().image;
            }
            cursor = source.children(&item).unwrap();
            let error = source.next(&mut cursor).unwrap_err();
            assert_eq!(
                zencodec::find_cause::<SelectionError>(&*error),
                Some(&SelectionError::Cycle)
            );
        }
        let (bytes, _) = nested_fixture(false, false, false);
        let mut source = TiffItems::open(
            Cursor::new(bytes),
            Allowances {
                discoveries: 1,
                ..Default::default()
            },
        )
        .unwrap();
        let mut cursor = source.images();
        source.next(&mut cursor).unwrap().unwrap();
        let error = source.next(&mut cursor).unwrap_err();
        assert_eq!(
            zencodec::find_cause::<SelectionError>(&*error),
            Some(&SelectionError::DiscoveryLimit)
        );
    }
}

#[test]
fn native_pages_levels_and_planar_units_match_original_samples() {
    for big in [false, true] {
        for planar in [false, true] {
            let (bytes, refs) = fixture(big, planar, true);
            for buffered in [false, true] {
                for (page, levels, index) in [(0, &[][..], 0), (0, &[0][..], 1), (1, &[][..], 2)] {
                    let source = Source::new(bytes.clone());
                    let request = Selection {
                        page,
                        levels,
                        ..selected(levels)
                    };
                    let expected = crop(&refs[index], request.region);
                    let bridge = ReadAtReader::new(source.clone());
                    let output = if buffered {
                        tiff_region::decode(
                            BufReader::with_capacity(4096, bridge),
                            request,
                            Allowances::default(),
                            || Ok(()),
                        )
                    } else {
                        tiff_region::decode(bridge, request, Allowances::default(), || Ok(()))
                    }
                    .unwrap();
                    assert_eq!(output.bytes, expected);
                    assert_eq!(output.stride, 18);
                    assert_eq!(output.color, tiff::ColorType::RGB(8));
                    assert_eq!(output.sample, tiff::tags::SampleFormat::Uint);
                    assert_eq!(output.orientation, 1);
                    let l = source.ledger.borrow();
                    println!(
                        "tiff_reads\t{big}\t{planar}\t{buffered}\t{page}\t{index}\t{}\t{}",
                        l.calls, l.bytes
                    );
                }
            }
        }
    }
}

#[test]
fn selection_refuses_unsupported_level_and_admission_before_publication() {
    let (bytes, _) = fixture(false, false, false);
    assert!(
        tiff_region::decode(
            Cursor::new(&bytes),
            selected(&[0]),
            Allowances::default(),
            || Ok(())
        )
        .err()
        .unwrap()
        .to_string()
        .contains("not a reduced image")
    );
    for limits in [
        Allowances {
            pages: 0,
            ..Default::default()
        },
        Allowances {
            output: 10,
            ..Default::default()
        },
        Allowances {
            units: 0,
            ..Default::default()
        },
        Allowances {
            unit: 10,
            ..Default::default()
        },
        Allowances {
            tag: 1,
            ..Default::default()
        },
    ] {
        assert!(
            tiff_region::decode(Cursor::new(&bytes), selected(&[]), limits, || Ok(())).is_err()
        );
    }
}

#[test]
fn partial_errors_and_version_replacement_retain_the_typed_cause() {
    let (bytes, _) = fixture(false, false, true);
    let good = Source::new(bytes.clone());
    let earlier = tiff_region::decode(
        ReadAtReader::new(good.clone()),
        selected(&[]),
        Allowances::default(),
        || Ok(()),
    )
    .unwrap();
    let calls = good.ledger.borrow().calls;
    for at in [1, calls / 2, calls] {
        let source = Source::new(bytes.clone());
        source.ledger.borrow_mut().fail = Some(at);
        let error = tiff_region::decode(
            ReadAtReader::new(source),
            selected(&[]),
            Allowances::default(),
            || Ok(()),
        )
        .err()
        .unwrap();
        assert!(
            matches!(
                zencodec::find_cause::<SourceFault>(&*error),
                Some(SourceFault::Injected)
            ),
            "{error:?}"
        );
    }
    let source = Source::new(bytes);
    let mut checks = 0;
    let error = tiff_region::decode(
        ReadAtReader::new(source.clone()),
        selected(&[]),
        Allowances::default(),
        || {
            checks += 1;
            if checks == 2 {
                source.ledger.borrow_mut().changed = true;
            }
            Ok(())
        },
    )
    .err()
    .unwrap();
    assert!(
        matches!(
            zencodec::find_cause::<SourceFault>(&*error),
            Some(SourceFault::VersionChanged)
        ),
        "{error:?}"
    );
    source.ledger.borrow_mut().changed = false;
    assert!(source.clone().len().is_err());
    assert!(!earlier.bytes.is_empty());
}

#[test]
fn cancellation_before_publication_does_not_return_partially_decoded_pixels() {
    let (bytes, _) = fixture(false, true, true);
    let mut checks = 0;
    tiff_region::decode(
        Cursor::new(&bytes),
        selected(&[]),
        Allowances::default(),
        || {
            checks += 1;
            Ok(())
        },
    )
    .unwrap();
    let last = checks;
    checks = 0;
    let err = tiff_region::decode(
        Cursor::new(&bytes),
        selected(&[]),
        Allowances::default(),
        || {
            checks += 1;
            if checks == last {
                Err(Box::new(io::Error::from(io::ErrorKind::Interrupted)))
            } else {
                Ok(())
            }
        },
    )
    .err()
    .unwrap();
    assert_eq!(
        err.downcast_ref::<io::Error>().unwrap().kind(),
        io::ErrorKind::Interrupted
    );
}

#[test]
fn seek_bridge_keeps_its_cursor_and_rejects_bad_seeks_without_reading() {
    let source = Source::new(vec![1, 2, 3, 4]);
    let mut r = ReadAtReader::new(source.clone());
    assert_eq!(r.seek(SeekFrom::End(-2)).unwrap(), 2);
    let mut out = [0; 8];
    assert_eq!(r.read(&mut out).unwrap(), 2);
    assert_eq!(&out[..2], &[3, 4]);
    assert_eq!(r.stream_position().unwrap(), 4);
    assert!(r.seek(SeekFrom::Current(i64::MIN)).is_err());
    assert_eq!(r.stream_position().unwrap(), 4);
    assert_eq!(r.seek(SeekFrom::Start(u64::MAX)).unwrap(), u64::MAX);
    assert!(r.seek(SeekFrom::Current(1)).is_err());
    assert_eq!(r.read(&mut []).unwrap(), 0);
    assert_eq!(source.ledger.borrow().calls, 1);
    r.rewind().unwrap();
    assert_eq!(r.read(&mut out[..1]).unwrap(), 1);
    let _source = r.into_inner();
}

fn artifacts() -> &'static Path {
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    PATH.get_or_init(|| {
        let p = PathBuf::from(
            std::env::var_os("SPATIAL_ARTIFACTS")
                .expect("SPATIAL_ARTIFACTS must name a fresh persistent directory"),
        );
        std::fs::create_dir(&p).unwrap();
        p
    })
}
#[test]
fn positional_disk_and_independent_libtiff_decode_match_native_selection() {
    for planar in [false, true] {
        let (bytes, refs) = fixture(false, planar, true);
        let path = artifacts().join(format!("planar-{planar}.tiff"));
        std::fs::File::create_new(&path)
            .unwrap()
            .write_all(&bytes)
            .unwrap();
        let mut limits = tiff::decoder::Limits::default();
        limits.ifd_value_size = 64 * 1024;
        limits.decoding_buffer_size = 16 * 1024 * 1024;
        assert_eq!(
            crate::checked_source::stored_dimensions(&path, limits).unwrap(),
            (refs[0].width, refs[0].height)
        );
        let disk = zencodec_io::FileReadAt::open(&path).unwrap();
        let decoded = tiff_region::decode(
            ReadAtReader::new(disk),
            selected(&[]),
            Allowances::default(),
            || Ok(()),
        )
        .unwrap();
        assert_eq!(decoded.bytes, crop(&refs[0], selected(&[]).region));
        let raw = path.with_extension("rgb");
        let result = std::process::Command::new("timeout")
            .args(["--kill-after=5s", "30s"])
            .arg(
                std::env::var_os("SPATIAL_ORACLE_PYTHON")
                    .expect("SPATIAL_ORACLE_PYTHON must have the pinned Pillow oracle"),
            )
            .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/oracle.py"))
            .arg(&path)
            .arg(&raw)
            .output()
            .expect("pinned Pillow/libtiff and timeout are required");
        let mut log = std::fs::File::create_new(path.with_extension("log")).unwrap();
        log.write_all(&result.stdout).unwrap();
        log.write_all(&result.stderr).unwrap();
        assert!(result.status.success(), "{result:?}");
        assert_eq!(std::fs::read(raw).unwrap(), refs[0].bytes);
        // Full selected SubIFD decode independently checks the region's geometry
        // against the native full-image route as well as the original samples.
        let mut full = Decoder::open(Cursor::new(&bytes)).unwrap();
        full.next_image().unwrap();
        let child = full
            .current_ifd()
            .get_tag(Tag::SubIfd)
            .unwrap()
            .into_ifd_vec()
            .unwrap()[0];
        let mut cursor = Cursor::new(&bytes);
        let mut header = TiffHeader::parse(&mut cursor).unwrap();
        header.first_ifd = child;
        let mut full = header.open(cursor);
        full.next_image().unwrap();
        let mut buffer = tiff::decoder::DecodingSampleBuffer::U8(Vec::new());
        let layout = full.read_image_to_buffer(&mut buffer).unwrap();
        assert_eq!(layout.planes, if planar { 3 } else { 1 });
        let native = buffer.as_buffer(0);
        let samples = native.as_bytes();
        assert_eq!(samples.len(), layout.complete_len);
        let expected = &refs[1];
        if planar {
            for c in 0..3 {
                for y in 0..expected.height as usize {
                    for x in 0..expected.width as usize {
                        assert_eq!(
                            samples[c * layout.plane_stride.unwrap().get()
                                + y * layout.row_stride.unwrap().get()
                                + x],
                            expected.bytes[(y * expected.width as usize + x) * 3 + c]
                        );
                    }
                }
            }
        } else {
            assert_eq!(samples, expected.bytes);
        }
    }
}
