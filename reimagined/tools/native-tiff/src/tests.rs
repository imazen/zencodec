use super::native::TiffCodec;
use std::{
    io,
    io::Write,
    num::NonZeroU16,
    process::{Command, Stdio},
};
use zencodec::{decode::DecodeSpec, document::*};
use zencodec_io::{Collect, DriveOptions, ReadAt, document::DocumentReader};

struct Source {
    bytes: Vec<u8>,
    reads: Vec<(u64, usize)>,
    fail: bool,
}
impl ReadAt for Source {
    fn read_at(&mut self, offset: u64, out: &mut [u8]) -> io::Result<usize> {
        if self.fail {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "native-tiff-source-cause",
            ));
        }
        let at = (offset as usize).min(self.bytes.len());
        let n = out.len().min(self.bytes.len() - at);
        self.reads.push((offset, n));
        out[..n].copy_from_slice(&self.bytes[at..at + n]);
        Ok(n)
    }
    fn total_len(&mut self) -> io::Result<Option<u64>> {
        Ok(Some(self.bytes.len() as u64))
    }
}
fn put16(bytes: &mut [u8], at: usize, n: u16) {
    bytes[at..at + 2].copy_from_slice(&n.to_le_bytes());
}
fn put32(bytes: &mut [u8], at: usize, n: u32) {
    bytes[at..at + 4].copy_from_slice(&n.to_le_bytes());
}
fn directory(
    bytes: &mut [u8],
    at: usize,
    size: u32,
    data: u32,
    next: u32,
    child: Option<u32>,
    reduced: bool,
) {
    let count = if child.is_some() { 13 } else { 12 };
    put16(bytes, at, count);
    let tiles = if size == 32 { 4 } else { 1 };
    let tags = [
        (254, 4, 1, u32::from(reduced)),
        (256, 4, 1, size),
        (257, 4, 1, size),
        (258, 3, 1, 8),
        (259, 3, 1, 1),
        (262, 3, 1, 1),
        (277, 3, 1, 1),
        (284, 3, 1, 1),
        (322, 4, 1, 16),
        (323, 4, 1, 16),
        (324, 4, tiles, data),
        (325, 4, tiles, if tiles == 4 { 496 } else { 256 }),
        (330, 4, 1, child.unwrap_or(0)),
    ];
    for (i, (tag, kind, count, value)) in tags[..usize::from(count)].iter().copied().enumerate() {
        let p = at + 2 + i * 12;
        put16(bytes, p, tag);
        put16(bytes, p + 2, kind);
        put32(bytes, p + 4, count);
        put32(bytes, p + 8, value);
    }
    put32(bytes, at + 2 + usize::from(count) * 12, next);
}
fn pixel(image: u8, x: u32, y: u32) -> u8 {
    image.wrapping_mul(71).wrapping_add((x * 5 + y * 11) as u8)
}
fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 2048];
    bytes[..4].copy_from_slice(b"II*\0");
    put32(&mut bytes, 4, 8);
    directory(&mut bytes, 8, 32, 480, 320, Some(170), false);
    directory(&mut bytes, 170, 16, 1536, 0, None, true);
    directory(&mut bytes, 320, 16, 1792, 0, None, false);
    for tile in 0..4usize {
        put32(&mut bytes, 480 + tile * 4, (512 + tile * 256) as u32);
        put32(&mut bytes, 496 + tile * 4, 256);
        for y in 0..16usize {
            for x in 0..16usize {
                bytes[512 + tile * 256 + y * 16 + x] = pixel(
                    0,
                    ((tile % 2) * 16 + x) as u32,
                    ((tile / 2) * 16 + y) as u32,
                );
            }
        }
    }
    for image in 1..=2usize {
        for y in 0..16usize {
            for x in 0..16usize {
                bytes[1536 + (image - 1) * 256 + y * 16 + x] =
                    pixel(image as u8, x as u32, y as u32);
            }
        }
    }
    bytes
}
fn query(listing: Listing) -> Query {
    Query::list(listing, NonZeroU16::new(16).unwrap())
}
fn entries(answer: Answer) -> Vec<Entry> {
    match answer {
        Answer::Page { entries, next } => {
            assert!(next.is_none());
            entries
        }
        _ => panic!("not a listing"),
    }
}
fn output(collect: Collect) -> Vec<u8> {
    let frames = collect.into_frames();
    assert_eq!(frames.len(), 1);
    let p = frames[0].pixels.packed().unwrap();
    (0..p.height())
        .flat_map(|y| p.as_slice().row(y)[..p.width() as usize].to_vec())
        .collect()
}

#[test]
fn native_directory_browsing_and_repeated_tile_roi_use_the_same_range_dialogue() {
    let source = Source {
        bytes: fixture(),
        reads: Vec::new(),
        fail: false,
    };
    let mut document = DocumentReader::open(
        &TiffCodec,
        source,
        OpenSpec::default(),
        DriveOptions::default(),
    )
    .unwrap();
    assert!(document.source().reads.is_empty());
    let roots = entries(document.query(query(Listing::Images)).unwrap());
    assert_eq!(roots.len(), 2);
    let root = match &roots[0] {
        Entry::Image(id) => id.clone(),
        _ => unreachable!(),
    };
    let children = entries(
        document
            .query(query(Listing::Children(root.clone())))
            .unwrap(),
    );
    assert_eq!(children.len(), 1);
    let levels = entries(
        document
            .query(query(Listing::Levels(root.clone())))
            .unwrap(),
    );
    assert_eq!(levels.len(), 1);
    let level = match &levels[0] {
        Entry::Level(level) => {
            assert_eq!((level.bounds.width, level.bounds.height), (16, 16));
            level.id.clone()
        }
        _ => unreachable!(),
    };
    assert!(
        document
            .source()
            .reads
            .iter()
            .all(|(offset, len)| offset + *len as u64 <= 512),
        "discovery read sample payload"
    );
    for (region, tile) in [
        (Bounds::new(17, 2, 11, 9).unwrap(), 1u64),
        (Bounds::new(2, 20, 5, 6).unwrap(), 2),
    ] {
        let before = document.source().reads.len();
        let mut sink = Collect::new();
        let receipt = document
            .decode(
                &Selection::new(Target::new(root.clone())).with_region(region),
                &DecodeSpec::new(),
                &mut sink,
            )
            .unwrap();
        assert_eq!(receipt.region, region);
        assert_eq!(
            (receipt.full_bounds.width, receipt.full_bounds.height),
            (32, 32)
        );
        let expected: Vec<_> = (0..region.height)
            .flat_map(|y| {
                (0..region.width).map(move |x| pixel(0, x + region.x as u32, y + region.y as u32))
            })
            .collect();
        assert_eq!(output(sink), expected);
        let reads = &document.source().reads[before..];
        assert!(
            reads.iter().all(|(offset, _)| *offset >= 8),
            "reopened the TIFF header"
        );
        let payload: Vec<_> = reads
            .iter()
            .filter(|(offset, len)| *offset < 1536 && offset + *len as u64 > 512)
            .collect();
        assert!(!payload.is_empty());
        assert!(
            payload
                .iter()
                .all(|(offset, len)| *offset >= 512 + tile * 256
                    && offset + *len as u64 <= 512 + (tile + 1) * 256),
            "read an unrelated tile: {payload:?}"
        );
    }
    let mut sink = Collect::new();
    let region = Bounds::new(3, 4, 7, 5).unwrap();
    document
        .decode(
            &Selection::new(Target::new(root).with_level(level)).with_region(region),
            &DecodeSpec::new(),
            &mut sink,
        )
        .unwrap();
    let expected: Vec<_> = (0..5)
        .flat_map(|y| (0..7).map(move |x| pixel(1, x + 3, y + 4)))
        .collect();
    assert_eq!(output(sink), expected);
}

#[test]
fn independent_libtiff_decodes_the_hand_authored_tiled_tiff() {
    if let Some(directory) = std::env::var_os("NATIVE_TIFF_OUTPUT") {
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            std::path::Path::new(&directory).join("tiled-collection.tiff"),
            fixture(),
        )
        .unwrap();
    }
    let directory = std::env::var_os("TMPDIR").expect("set TMPDIR to a disk-backed directory");
    let path = std::path::Path::new(&directory).join(format!(
        "reimagined-native-tiff-{}.tiff",
        std::process::id()
    ));
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .unwrap();
    file.write_all(&fixture()).unwrap();
    drop(file);
    // ImageMagick uses its independent libtiff delegate. FFmpeg 8.0.1
    // produced zero samples for this tiled fixture; retain that finding in
    // the qualification receipt rather than treating its exit status as proof.
    let result = Command::new("magick")
        .arg(format!("{}[0]", path.display()))
        .args(["-depth", "8", "gray:-"])
        .stdin(Stdio::null())
        .output()
        .expect("ImageMagick with the TIFF delegate is required");
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let expected: Vec<_> = (0..32)
        .flat_map(|y| (0..32).map(move |x| pixel(0, x, y)))
        .collect();
    assert_eq!(result.stdout, expected);
}

#[test]
fn source_failure_retains_the_native_transport_cause_and_drops_the_worker() {
    let source = Source {
        bytes: fixture(),
        reads: Vec::new(),
        fail: true,
    };
    let mut document = DocumentReader::open(
        &TiffCodec,
        source,
        OpenSpec::default(),
        DriveOptions::default(),
    )
    .unwrap();
    let error = document.query(query(Listing::Images)).unwrap_err();
    let source = zencodec::find_cause::<io::Error>(&error).unwrap();
    assert_eq!(source.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(source.to_string(), "native-tiff-source-cause");
    assert!(!document.is_open());
}
