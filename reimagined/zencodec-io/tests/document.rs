//! Document requests against an indexed source with independently checked pixels.
mod common;

use std::{io, num::NonZeroU16, sync::Arc};
use zencodec::{ErrorKind, ResourceLimits, decode::DecodeSpec, document::*};
use zencodec_fixtures::document::CatalogCodec;
use zencodec_io::{Collect, DriveOptions, ReadAt, document::DocumentReader};

struct Trace {
    bytes: Vec<u8>,
    reads: Vec<(u64, usize)>,
}
impl ReadAt for Trace {
    fn read_at(&mut self, offset: u64, out: &mut [u8]) -> io::Result<usize> {
        let at = usize::try_from(offset).unwrap().min(self.bytes.len());
        let n = out.len().min(self.bytes.len() - at);
        self.reads.push((offset, n));
        out[..n].copy_from_slice(&self.bytes[at..at + n]);
        Ok(n)
    }
    fn total_len(&mut self) -> io::Result<Option<u64>> {
        Ok(Some(self.bytes.len() as u64))
    }
}

fn fixture() -> Trace {
    let shapes = [(9u32, 7u32), (5, 4), (5, 7), (9, 4)];
    let mut bytes = b"ZDOC".to_vec();
    bytes.extend(2u32.to_le_bytes());
    bytes.extend(8u32.to_le_bytes());
    let mut payload = Vec::new();
    for image in 0..2u32 {
        for (level, (w, h)) in shapes.iter().copied().enumerate() {
            bytes.extend(image.to_le_bytes());
            bytes.extend((level as u32).to_le_bytes());
            bytes.extend(w.to_le_bytes());
            bytes.extend(h.to_le_bytes());
            bytes.extend((-3i32).to_le_bytes());
            bytes.extend(11i32.to_le_bytes());
            bytes.extend((12u64 + 8 * 32 + payload.len() as u64).to_le_bytes());
            for y in 0..h {
                for x in 0..w {
                    for c in 0..3 {
                        payload.push(value(image, level as u32, x, y, c));
                    }
                }
            }
        }
    }
    bytes.extend(payload);
    Trace {
        bytes,
        reads: Vec::new(),
    }
}
fn value(image: u32, level: u32, x: u32, y: u32, c: u32) -> u8 {
    (image * 70 + level * 31 + x * 3 + y * 13 + c * 7) as u8
}
fn page(listing: Listing, limit: u16) -> Query {
    Query::list(listing, NonZeroU16::new(limit).unwrap())
}
fn entries(answer: Answer) -> (Vec<Entry>, Option<Cursor>) {
    match answer {
        Answer::Page { entries, next } => (entries, next),
        _ => panic!("not a page"),
    }
}
fn image(reader: &mut DocumentReader<Trace>) -> ImageId {
    match entries(reader.query(page(Listing::Images, 1)).unwrap())
        .0
        .remove(0)
    {
        Entry::Image(image) => image,
        _ => panic!("not an image"),
    }
}
fn reader() -> DocumentReader<Trace> {
    DocumentReader::open(
        &CatalogCodec,
        fixture(),
        OpenSpec::default(),
        DriveOptions::default(),
    )
    .unwrap()
}
fn samples(collect: Collect) -> Vec<u8> {
    let frames = collect.into_frames();
    assert_eq!(frames.len(), 1);
    let pixels = frames[0].pixels.packed().unwrap();
    (0..pixels.height())
        .flat_map(|y| pixels.as_slice().row(y)[..pixels.width() as usize * 3].to_vec())
        .collect()
}

#[test]
fn lazy_paged_discovery_retains_index_and_refuses_wrong_continuations() {
    let mut reader = reader();
    assert!(reader.source().reads.is_empty());
    let (roots, next) = entries(reader.query(page(Listing::Images, 1)).unwrap());
    assert_eq!(roots.len(), 1);
    assert_eq!(reader.source().reads, [(0, 12)]);
    let cursor = next.unwrap();
    let first = match &roots[0] {
        Entry::Image(id) => id.clone(),
        _ => unreachable!(),
    };
    assert!(
        reader
            .query(Query::List {
                listing: Listing::Levels(first.clone()),
                cursor: Some(cursor.clone()),
                limit: NonZeroU16::new(1).unwrap()
            })
            .is_err()
    );
    assert!(reader.is_open());
    assert_eq!(reader.source().reads.len(), 1);
    let (_, next) = entries(
        reader
            .query(Query::List {
                listing: Listing::Images,
                cursor: Some(cursor),
                limit: NonZeroU16::new(1).unwrap(),
            })
            .unwrap(),
    );
    assert!(next.is_none());
    assert_eq!(reader.source().reads.len(), 1);
    let (levels, _) = entries(reader.query(page(Listing::Levels(first), 4)).unwrap());
    let shapes: Vec<_> = levels
        .iter()
        .map(|level| match level {
            Entry::Level(level) => (level.bounds.width, level.bounds.height),
            _ => panic!("not a level"),
        })
        .collect();
    assert_eq!(shapes, [(9, 7), (5, 4), (5, 7), (9, 4)]);
    assert_eq!(reader.source().reads, [(0, 12), (12, 256)]);
}

#[test]
fn repeated_rois_use_native_level_coordinates_and_read_only_their_rows() {
    let mut reader = reader();
    let image = image(&mut reader);
    let levels = entries(
        reader
            .query(page(Listing::Levels(image.clone()), 4))
            .unwrap(),
    )
    .0;
    let level = match &levels[2] {
        Entry::Level(level) => level.id.clone(),
        _ => unreachable!(),
    };
    let target = Target::new(image).with_level(level);
    let _ = reader.query(Query::Describe(target.clone())).unwrap();
    let before = reader.source().reads.len();
    for region in [
        Bounds::new(-2, 12, 3, 2).unwrap(),
        Bounds::new(-3, 15, 2, 3).unwrap(),
    ] {
        let selection = Selection::new(target.clone()).with_region(region);
        let mut collect = Collect::new();
        let receipt = reader
            .decode(&selection, &DecodeSpec::new(), &mut collect)
            .unwrap();
        assert_eq!(receipt.region, region);
        assert_eq!(receipt.full_bounds, Bounds::new(-3, 11, 5, 7).unwrap());
        assert_eq!(receipt.registration, None);
        let expected: Vec<_> = (0..region.height)
            .flat_map(|y| {
                (0..region.width).flat_map(move |x| {
                    (0..3).map(move |c| {
                        value(
                            0,
                            2,
                            x + (region.x + 3) as u32,
                            y + (region.y - 11) as u32,
                            c,
                        )
                    })
                })
            })
            .collect();
        assert_eq!(samples(collect), expected);
    }
    assert_eq!(reader.source().reads.len() - before, 5);
    assert!(
        reader.source().reads[before..]
            .iter()
            .all(|(offset, len)| *offset >= 268 && *len <= 9)
    );
}

#[test]
fn images_levels_channels_and_cursors_are_document_scoped() {
    let mut first = reader();
    let mut second = reader();
    let id = image(&mut first);
    let before = second.source().reads.len();
    assert!(
        second
            .query(Query::Describe(Target::new(id.clone())))
            .is_err()
    );
    assert_eq!(second.source().reads.len(), before);
    assert!(second.is_open());
    let (channels, _) = entries(first.query(page(Listing::Channels(id.clone()), 3)).unwrap());
    let selected: Arc<[_]> = channels
        .into_iter()
        .map(|entry| match entry {
            Entry::Channel(channel) => channel.id,
            _ => unreachable!(),
        })
        .collect();
    let target = Target::new(id);
    first.query(Query::Describe(target.clone())).unwrap();
    let mut collect = Collect::new();
    let request = Selection::new(target.clone()).with_channels(selected.clone());
    let receipt = first
        .decode(&request, &DecodeSpec::new(), &mut collect)
        .unwrap();
    assert_eq!(receipt.channels, Channels::Only(selected.clone()));
    let one = Selection::new(target).with_channels(Arc::from(&selected[..1]));
    let reads = first.source().reads.len();
    let error = first
        .decode(&one, &DecodeSpec::new(), &mut Collect::new())
        .unwrap_err();
    assert!(matches!(error.error().kind(), ErrorKind::Unsupported(_)));
    assert_eq!(first.source().reads.len(), reads);
    assert!(first.is_open());
}

#[test]
fn roi_is_exact_and_does_not_reduce_full_image_resource_requirements() {
    let mut reader = reader();
    let id = image(&mut reader);
    let target = Target::new(id);
    reader.query(Query::Describe(target.clone())).unwrap();
    let before = reader.source().reads.len();
    let outside = Selection::new(target.clone()).with_region(Bounds::new(-4, 11, 2, 2).unwrap());
    assert!(
        reader
            .decode(&outside, &DecodeSpec::new(), &mut Collect::new())
            .is_err()
    );
    assert_eq!(reader.source().reads.len(), before);
    let tiny = Selection::new(target).with_region(Bounds::new(-3, 11, 1, 1).unwrap());
    let spec = DecodeSpec::new().with_limits(ResourceLimits::none().with_max_pixels(62));
    assert!(reader.decode(&tiny, &spec, &mut Collect::new()).is_err());
    assert_eq!(reader.source().reads.len(), before);
}

#[test]
fn bounded_input_refuses_before_read_and_unknown_workspace_before_factory() {
    let mut spec = OpenSpec::default();
    spec.limits.max_input_bytes = Some(11);
    let mut reader =
        DocumentReader::open(&CatalogCodec, fixture(), spec, DriveOptions::default()).unwrap();
    assert!(reader.query(page(Listing::Images, 1)).is_err());
    assert!(reader.source().reads.is_empty());
    assert!(!reader.is_open());
    let mut spec = OpenSpec::default();
    spec.limits.max_memory_bytes = Some(1000000);
    assert!(DocumentReader::open(&CatalogCodec, fixture(), spec, DriveOptions::default()).is_err());
}

#[test]
fn opaque_ids_do_not_alias_equal_native_keys_from_different_origins() {
    let a = Origin::new();
    let b = Origin::new();
    assert_ne!(a.image(1), b.image(1));
    assert_eq!(a.image(1), a.image(1));
    let id = a.image(1);
    let other = a.image(2);
    let level = a.level(&id, 0).unwrap();
    assert!(Target::new(other).with_level(level).validate(&a).is_err());
}

#[derive(Clone, Copy)]
enum Mutation {
    MissingRows,
    SwallowedSink,
    IgnoredOutputRequirement,
}
struct MutatingCodec(Mutation);
struct MutatingDocument {
    inner: Box<dyn Document>,
    mode: Mutation,
}
struct MutatingDecoder {
    inner: Box<dyn zencodec::decode::Decoder>,
    mode: Mutation,
    adopted: bool,
}
impl DocumentCodec for MutatingCodec {
    fn document(&self, spec: &OpenSpec) -> zencodec::Result<Box<dyn Document>> {
        Ok(Box::new(MutatingDocument {
            inner: CatalogCodec.document(spec)?,
            mode: self.0,
        }))
    }
}
impl Document for MutatingDocument {
    fn origin(&self) -> &Origin {
        self.inner.origin()
    }
    fn spec(&self) -> &OpenSpec {
        self.inner.spec()
    }
    fn start(&mut self, query: Query) -> zencodec::Result<()> {
        self.inner.start(query)
    }
    fn requested(&self) -> &[zencodec::InputRequest] {
        self.inner.requested()
    }
    fn step(&mut self, input: zencodec::Input<'_>) -> zencodec::Result<QueryStep> {
        self.inner.step(input)
    }
    fn decoder(
        &mut self,
        selection: &Selection,
        spec: &DecodeSpec,
    ) -> zencodec::Result<SelectedDecode> {
        let mut native_spec = spec.clone();
        if matches!(self.mode, Mutation::IgnoredOutputRequirement) {
            native_spec.accept = zencodec::decode::Accept::native();
        }
        let prepared = self.inner.decoder(selection, &native_spec)?;
        Ok(SelectedDecode {
            selected: prepared.selected,
            decoder: Box::new(MutatingDecoder {
                inner: prepared.decoder,
                mode: self.mode,
                adopted: false,
            }),
        })
    }
}
impl zencodec::decode::Decoder for MutatingDecoder {
    fn requested(&self) -> &[zencodec::InputRequest] {
        self.inner.requested()
    }
    fn info(&self) -> Option<&zencodec::ImageInfo> {
        self.inner.info()
    }
    fn plan(&self) -> Option<&zencodec::decode::DecodePlan> {
        self.inner.plan()
    }
    fn step(
        &mut self,
        input: zencodec::Input<'_>,
        sink: &mut dyn zencodec::decode::Sink,
    ) -> zencodec::Result<zencodec::decode::DecodeStep> {
        if self.adopted {
            match self.mode {
                Mutation::MissingRows => {
                    return Ok(zencodec::decode::DecodeStep::new(
                        0,
                        zencodec::decode::DecodeEvent::Done,
                    ));
                }
                Mutation::IgnoredOutputRequirement => {}
                Mutation::SwallowedSink => {
                    let _ = self.inner.step(input, sink);
                    return Ok(zencodec::decode::DecodeStep::new(
                        0,
                        zencodec::decode::DecodeEvent::Done,
                    ));
                }
            }
        }
        let result = self.inner.step(input, sink)?;
        self.adopted = result.event == zencodec::decode::DecodeEvent::Info;
        Ok(result)
    }
}

#[test]
fn native_done_cannot_skip_the_selected_rows_and_finish() {
    let mut reader = DocumentReader::open(
        &MutatingCodec(Mutation::MissingRows),
        fixture(),
        OpenSpec::default(),
        DriveOptions::default(),
    )
    .unwrap();
    let id = image(&mut reader);
    let target = Target::new(id);
    reader.query(Query::Describe(target.clone())).unwrap();
    let result = reader.decode(
        &Selection::new(target),
        &DecodeSpec::new(),
        &mut Collect::new(),
    );
    assert!(
        result.is_err(),
        "native Done omitted the complete selected frame"
    );
    assert!(!reader.is_open());
}

struct FailingSink;
impl zencodec::decode::Sink for FailingSink {
    fn begin(&mut self, _: &zencodec::FrameHeader) -> zencodec::Result<()> {
        Err(zencodec::CodecError::new(ErrorKind::Sink)
            .with_native(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "selected-sink-cause",
            ))
            .at())
    }
    fn lend(
        &mut self,
        _: u32,
        _: u32,
        _: zencodec::Pass,
    ) -> zencodec::Result<zenpixels::PixelSliceMut<'_>> {
        panic!("sink effect after failure")
    }
}

#[test]
fn a_swallowed_sink_failure_keeps_its_original_cause_and_poisoned_session() {
    let mut reader = DocumentReader::open(
        &MutatingCodec(Mutation::SwallowedSink),
        fixture(),
        OpenSpec::default(),
        DriveOptions::default(),
    )
    .unwrap();
    let id = image(&mut reader);
    let target = Target::new(id);
    reader.query(Query::Describe(target.clone())).unwrap();
    let error = reader
        .decode(
            &Selection::new(target),
            &DecodeSpec::new(),
            &mut FailingSink,
        )
        .expect_err("swallowed sink failure");
    let cause = zencodec::find_cause::<io::Error>(&error).expect("retained native sink error");
    assert_eq!(cause.kind(), io::ErrorKind::BrokenPipe);
    assert_eq!(cause.to_string(), "selected-sink-cause");
    assert!(!reader.is_open());
}

#[test]
fn pre_cancelled_discovery_is_terminal_before_any_source_read() {
    let switch = common::Switch::default();
    let mut reader = DocumentReader::open(
        &CatalogCodec,
        fixture(),
        OpenSpec {
            pulse: switch.pulse(),
            ..OpenSpec::default()
        },
        DriveOptions::default(),
    )
    .unwrap();
    switch.stop();
    let error = reader.query(page(Listing::Images, 1)).unwrap_err();
    assert_eq!(
        error.error().kind(),
        ErrorKind::Stopped(zencodec::StopReason::Cancelled)
    );
    assert!(reader.source().reads.is_empty());
    assert!(
        !reader.is_open(),
        "cancelled document retained its native owner"
    );
}

#[test]
fn a_selection_pulse_cannot_replace_the_opened_document_cancellation() {
    let switch = common::Switch::default();
    let mut reader = DocumentReader::open(
        &CatalogCodec,
        fixture(),
        OpenSpec {
            pulse: switch.pulse(),
            ..OpenSpec::default()
        },
        DriveOptions::default(),
    )
    .unwrap();
    let id = image(&mut reader);
    let before = reader.source().reads.len();
    switch.stop();
    let mut sink = Collect::new();
    let error = reader
        .decode(
            &Selection::new(Target::new(id)),
            &DecodeSpec::new(),
            &mut sink,
        )
        .unwrap_err();
    assert_eq!(
        error.error().kind(),
        ErrorKind::Stopped(zencodec::StopReason::Cancelled)
    );
    assert_eq!(reader.source().reads.len(), before);
    assert!(sink.into_frames().is_empty());
    assert!(
        !reader.is_open(),
        "cancelled selection retained its native owner"
    );
}

#[test]
fn ignored_required_output_refuses_before_any_sink_effect() {
    let mut reader = DocumentReader::open(
        &MutatingCodec(Mutation::IgnoredOutputRequirement),
        fixture(),
        OpenSpec::default(),
        DriveOptions::default(),
    )
    .unwrap();
    let id = image(&mut reader);
    let mut sink = Collect::new();
    let spec = DecodeSpec::new().with_accept(zencodec::decode::Accept::require(&[
        zenpixels::PixelDescriptor::RGBA8_SRGB,
    ]));
    let error = reader
        .decode(&Selection::new(Target::new(id)), &spec, &mut sink)
        .unwrap_err();
    assert_eq!(error.error().kind(), ErrorKind::Protocol);
    assert!(sink.into_frames().is_empty());
    assert!(!reader.is_open());
}
