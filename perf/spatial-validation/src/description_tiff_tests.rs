use super::*;
use crate::description::Describe;
use std::{cell::Cell, ops::Range};

/// An independent I/O guard: native directory parsing establishes the encoded
/// payload ranges, and every read intersecting them fails before copying bytes.
struct NoPixels {
    input: Cursor<Vec<u8>>,
    ranges: Vec<Range<u64>>,
    denied: Rc<Cell<usize>>,
    enabled: Rc<Cell<bool>>,
}
impl NoPixels {
    fn new(bytes: Vec<u8>) -> Self {
        let mut input = Cursor::new(bytes);
        let header = TiffHeader::parse(&mut input).unwrap();
        let mut pending = vec![header.first_ifd];
        let mut seen = Vec::new();
        let mut ranges = Vec::new();
        let mut decoder = header.open(&mut input);
        while let Some(pointer) = pending.pop() {
            if seen.contains(&pointer) {
                continue;
            }
            seen.push(pointer);
            let directory = decoder.read_directory(pointer).unwrap();
            pending.extend(directory.next());
            let mut tags = decoder.read_directory_tags(&directory);
            pending.extend(tags.find_tag(Tag::SubIfd).unwrap()
                .map(|v| v.into_ifd_vec().unwrap()).unwrap_or_default());
            let offsets = tags.get_tag(Tag::TileOffsets).unwrap().into_u64_vec().unwrap();
            let counts = tags.get_tag(Tag::TileByteCounts).unwrap().into_u64_vec().unwrap();
            assert_eq!(offsets.len(), counts.len());
            ranges.extend(offsets.into_iter().zip(counts).map(|(at, count)| at..at.checked_add(count).unwrap()));
        }
        input.set_position(0);
        Self { input, ranges, denied: Rc::new(Cell::new(0)), enabled: Rc::new(Cell::new(true)) }
    }
}
impl Read for NoPixels {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let start = self.input.position();
        let end = start.checked_add(out.len() as u64).unwrap();
        if self.enabled.get() && self.ranges.iter().any(|r| start < r.end && end > r.start) {
            self.denied.set(self.denied.get() + 1);
            return Err(io::Error::other("pixel payload read forbidden"));
        }
        self.input.read(out)
    }
}
impl Seek for NoPixels {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> { self.input.seek(to) }
}

#[test]
fn description_tiff_nested_shared_graph_never_reads_pixel_payloads() {
    for big in [false, true] {
        for planar in [false, true] {
            let (bytes, refs) = nested_fixture(big, planar, false);
            let input = NoPixels::new(bytes.clone());
            let denied = input.denied.clone();
            let enabled = input.enabled.clone();
            let mut images = TiffCollection::open(input, Composition::Composited, Allowances::default()).unwrap();
            let mut roots = images.images().unwrap();
            let first = images.next(&mut roots).unwrap().unwrap();
            let second = images.next(&mut roots).unwrap().unwrap();
            assert!(images.next(&mut roots).unwrap().is_none());
            let mut children = images.children(&first).unwrap();
            let branch = images.next(&mut children).unwrap().unwrap();
            let mut children = images.children(&branch).unwrap();
            let leaf = images.next(&mut children).unwrap().unwrap();
            let mut children = images.children(&second).unwrap();
            assert_eq!(images.next(&mut children).unwrap().unwrap(), leaf);
            // Out-of-order, repeated descriptions cannot advance discovery.
            for (id, expected) in [(&leaf, &refs[2]), (&first, &refs[0]), (&second, &refs[1]), (&branch, &refs[1]), (&leaf, &refs[2])] {
                let description = images.describe(id).unwrap();
                assert_eq!(description.stored_size, (expected.width, expected.height));
                assert_eq!(description.stored_orientation, zenpixels::Orientation::from_exif(1));
                assert_eq!(description.placement, None);
            }
            assert_eq!(denied.get(), 0);
            assert!(images.next(&mut roots).unwrap().is_none());
            // Positive control proves that the guard really prevents decoding.
            images.decode(&leaf, &ImageRequest::new()).expect_err("pixel guard failed");
            assert!(denied.get() > 0);
            enabled.set(false);
            let output = images.decode(&leaf, &ImageRequest::new()).unwrap();
            let description = images.describe(&leaf).unwrap();
            assert_eq!(description.stored_size, (output.interpretation().geometry.canvas_width, output.interpretation().geometry.canvas_height));
            output.with_pixels(|p| assert_eq!(p.as_contiguous_bytes().unwrap(), refs[2].bytes)).unwrap();
            let mut other = TiffCollection::open(Cursor::new(bytes), Composition::Composited, Allowances::default()).unwrap();
            let error = other.describe(&leaf).unwrap_err();
            assert_eq!(zencodec::find_cause::<SelectionError>(&error), Some(&SelectionError::ForeignImage));
        }
    }
}

#[test]
fn description_tiff_inspection_does_not_promise_supported_compression() {
    let mut bytes = Cursor::new(Vec::new());
    let mut encoder = TiffEncoder::new(&mut bytes).unwrap();
    let mut directory = encoder.image_directory().unwrap();
    directory.write_tag(Tag::ImageWidth, 100_000u32).unwrap();
    directory.write_tag(Tag::ImageLength, 100_001u32).unwrap();
    directory.write_tag(Tag::Compression, 65000u16).unwrap();
    directory.finish().unwrap();
    let mut images = TiffCollection::open(Cursor::new(bytes.into_inner()), Composition::Composited, Allowances::default()).unwrap();
    let mut cursor = images.images().unwrap();
    let id = images.next(&mut cursor).unwrap().unwrap();
    assert_eq!(images.describe(&id).unwrap().stored_size, (100_000, 100_001));
    images.decode(&id, &ImageRequest::new()).expect_err("description promised raster support");
}

#[test]
fn description_tiff_orientation_is_per_item_and_invalid_values_refuse() {
    for orientation in 0u16..=9 {
        let mut bytes = Cursor::new(Vec::new());
        let mut encoder = TiffEncoder::new_big(&mut bytes).unwrap();
        let mut image = encoder.new_image::<tiff::encoder::colortype::RGB8>(2, 1).unwrap();
        image.encoder().write_tag(Tag::Orientation, orientation).unwrap();
        image.write_data(&[8, 17, 29, 47, 89, 131]).unwrap();
        let mut images = TiffCollection::open(Cursor::new(bytes.into_inner()), Composition::Composited, Allowances::default()).unwrap();
        let mut cursor = images.images().unwrap();
        let id = images.next(&mut cursor).unwrap().unwrap();
        match zenpixels::Orientation::from_exif(orientation as u8) {
            Some(expected) => {
                let description = images.describe(&id).unwrap();
                assert_eq!(description.stored_size, (2, 1));
                assert_eq!(description.stored_orientation, Some(expected));
                assert_eq!(images.decode(&id, &ImageRequest::new()).unwrap().interpretation().orientation, Some(expected));
            }
            None => { images.describe(&id).expect_err("invalid orientation was hidden"); }
        }
    }
}

#[test]
fn description_tiff_cancellation_at_every_inspection_seam() {
    for big in [false, true] {
        let (bytes, _) = nested_fixture(big, true, false);
        let run = |stop: &CheckStop, at: Option<usize>| -> Result<_, zencodec::At<crate::collection::CollectionError>> {
            let mut images = TiffCollection::open_with_stop(Cursor::new(&bytes), Composition::Composited, Allowances::default(), stop.token())?;
            let mut cursor = images.images()?;
            let id = images.next(&mut cursor)?.unwrap();
            let before = stop.calls();
            if let Some(at) = at { stop.after(at); }
            let result = images.describe(&id)?;
            Ok((result, stop.calls() - before))
        };
        let (_, checkpoints) = run(&CheckStop::default(), None).unwrap();
        assert!(checkpoints > 2 && checkpoints < 1024);
        for reason in [zencodec::enough::StopReason::Cancelled, zencodec::enough::StopReason::TimedOut] {
            for at in 0..checkpoints {
                let stop = CheckStop::with_reason(reason);
                let error = run(&stop, Some(at)).expect_err("stopped description escaped");
                assert_eq!(zencodec::find_cause::<zencodec::CodecError>(&error).unwrap().category(), zencodec::ErrorCategory::Stopped(reason));
            }
        }
    }
}
