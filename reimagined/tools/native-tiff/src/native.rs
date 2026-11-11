use super::bridge::{Operation, Progress};
use std::{
    collections::BTreeMap,
    io::{Read, Seek},
    sync::{Arc, Mutex},
};
use tiff::{
    ColorType,
    decoder::{Limits, TiffHeader},
    tags::{IfdPointer, Tag},
};
use zencodec::{
    Bound, ErrorKind, FormatInfo, FrameHeader, ImageFormat, ImageInfo, Input, InputRequest, Layout,
    MetadataKinds, ResourceEstimate, Result, SharedPulse, Unsupported, check, decode::*,
    document::*, error_msg, unsupported,
};
use zenpixels::{ColorPrimaries, PixelBuffer, PixelDescriptor, TransferFunction};

pub(super) struct TiffCodec;
struct State {
    header: Option<TiffHeader>,
    bounds: BTreeMap<u64, Bounds>,
}
struct TiffDocument {
    origin: Origin,
    spec: OpenSpec,
    state: Arc<Mutex<State>>,
    query: Option<Operation<Answer>>,
}
static FORMAT: FormatInfo =
    FormatInfo::new("tiff", "image/tiff", "tiff", 4, |b| b.starts_with(b"II*\0"));
fn native<E: std::error::Error + Send + Sync + 'static>(error: E) -> zencodec::Error {
    zencodec::CodecError::new(ErrorKind::Malformed)
        .with_codec("image-tiff")
        .with_native(error)
        .at()
}
fn invalid(message: &'static str) -> zencodec::Error {
    error_msg(ErrorKind::Protocol, message)
}
fn descriptor() -> PixelDescriptor {
    PixelDescriptor::GRAY8_SRGB
        .with_transfer(TransferFunction::Unknown)
        .with_primaries(ColorPrimaries::Unknown)
}
fn limits() -> Limits {
    let mut limits = Limits::default();
    limits.ifd_value_size = 64 * 1024;
    limits.decoding_buffer_size = 1024 * 1024;
    limits.intermediate_buffer_size = 1024 * 1024;
    limits
}

impl DocumentCodec for TiffCodec {
    fn document(&self, spec: &OpenSpec) -> Result<Box<dyn Document>> {
        Ok(Box::new(TiffDocument {
            origin: Origin::new(),
            spec: spec.clone(),
            state: Arc::new(Mutex::new(State {
                header: None,
                bounds: BTreeMap::new(),
            })),
            query: None,
        }))
    }
}

fn get_bounds<R: Read + Seek>(
    header: &TiffHeader,
    reader: &mut R,
    pointer: IfdPointer,
) -> Result<Bounds> {
    let mut decoder = header.open(reader).with_limits(limits());
    let directory = decoder.read_directory(pointer).map_err(native)?;
    let mut tags = decoder.read_directory_tags(&directory);
    let width = tags.get_tag_u32(Tag::ImageWidth).map_err(native)?;
    let height = tags.get_tag_u32(Tag::ImageLength).map_err(native)?;
    let bits = tags.get_tag_u32(Tag::BitsPerSample).map_err(native)?;
    let photo = tags
        .get_tag_u32(Tag::PhotometricInterpretation)
        .map_err(native)?;
    if bits != 8 || photo != 1 {
        return Err(unsupported(Unsupported::PixelFormat));
    }
    Bounds::new(0, 0, width, height)
}

fn answer<R: Read + Seek>(
    mut reader: R,
    state: &Arc<Mutex<State>>,
    origin: &Origin,
    query: Query,
) -> Result<Answer> {
    let mut state = state
        .lock()
        .map_err(|_| invalid("poisoned native document"))?;
    if state.header.is_none() {
        state.header = Some(TiffHeader::parse(&mut reader).map_err(native)?);
    }
    let header = state.header.as_ref().unwrap();
    match query {
        Query::Describe(target) => {
            let pointer = IfdPointer(match &target.level {
                Some(level) => origin.level_key(&target.image, level)?,
                None => origin.image_key(&target.image)?,
            });
            let bounds = get_bounds(header, &mut reader, pointer)?;
            state.bounds.insert(pointer.0, bounds);
            Ok(Answer::Description(Description {
                target,
                bounds,
                registration: None,
            }))
        }
        Query::List {
            listing,
            cursor,
            limit,
        } => {
            let start = cursor
                .as_ref()
                .map(|cursor| origin.cursor_key(&listing, cursor))
                .transpose()?;
            let mut entries = Vec::new();
            let next = match &listing {
                Listing::Images => {
                    let mut pointer = IfdPointer(start.unwrap_or(header.first_ifd.0));
                    let mut seen = Vec::new();
                    let mut decoder = header.open(&mut reader).with_limits(limits());
                    while pointer.0 != 0 && entries.len() < usize::from(limit.get()) {
                        if seen.contains(&pointer) {
                            return Err(invalid("cyclic TIFF main directories"));
                        }
                        seen.push(pointer);
                        let directory = decoder.read_directory(pointer).map_err(native)?;
                        entries.push(Entry::Image(origin.image(pointer.0)));
                        pointer = directory.next().unwrap_or(IfdPointer(0));
                    }
                    (pointer.0 != 0)
                        .then(|| origin.cursor(listing.clone(), pointer.0))
                        .transpose()?
                }
                Listing::Children(image) | Listing::Levels(image) => {
                    let parent = origin.image_key(image)?;
                    let mut decoder = header.open(&mut reader).with_limits(limits());
                    let directory = decoder.read_directory(IfdPointer(parent)).map_err(native)?;
                    let children = decoder
                        .read_directory_tags(&directory)
                        .find_tag(Tag::SubIfd)
                        .map_err(native)?
                        .map(|value| value.into_ifd_vec())
                        .transpose()
                        .map_err(native)?
                        .unwrap_or_default();
                    let begin = start.unwrap_or(0) as usize;
                    if children.len() > 1024 || begin > children.len() {
                        return Err(invalid("native child discovery bound"));
                    }
                    let mut position = begin;
                    while position < children.len() && entries.len() < usize::from(limit.get()) {
                        let pointer = children[position];
                        position += 1;
                        if pointer.0 == parent {
                            return Err(invalid("cyclic TIFF containment"));
                        }
                        if matches!(listing, Listing::Children(_)) {
                            entries.push(Entry::Image(origin.image(pointer.0)));
                        } else {
                            let child = decoder.read_directory(pointer).map_err(native)?;
                            let flags = decoder
                                .read_directory_tags(&child)
                                .find_tag(Tag::NewSubfileType)
                                .map_err(native)?
                                .map(|value| value.into_u32())
                                .transpose()
                                .map_err(native)?
                                .unwrap_or(0);
                            if flags & 1 != 0 && flags & 4 == 0 {
                                let bounds = get_bounds(header, &mut reader, pointer)?;
                                entries.push(Entry::Level(Level {
                                    id: origin.level(image, pointer.0)?,
                                    bounds,
                                    registration: None,
                                }));
                                // The parser will be recreated below only for another child.
                                decoder = header.open(&mut reader).with_limits(limits());
                            }
                        }
                    }
                    (position < children.len())
                        .then(|| origin.cursor(listing.clone(), position as u64))
                        .transpose()?
                }
                Listing::Channels(image) => {
                    let bounds =
                        get_bounds(header, &mut reader, IfdPointer(origin.image_key(image)?))?;
                    let _ = bounds;
                    if start.unwrap_or(0) == 0 {
                        entries.push(Entry::Channel(Channel {
                            id: origin.channel(image, 0)?,
                            name: Arc::from("Y"),
                            encoding: None,
                        }));
                    }
                    None
                }
            };
            Ok(Answer::Page { entries, next })
        }
    }
}

impl Document for TiffDocument {
    fn origin(&self) -> &Origin {
        &self.origin
    }
    fn spec(&self) -> &OpenSpec {
        &self.spec
    }
    fn start(&mut self, query: Query) -> Result<()> {
        if self.query.is_some() {
            return Err(invalid("native query already active"));
        }
        query.validate(&self.origin)?;
        check(&self.spec.pulse)?;
        let state = self.state.clone();
        let origin = self.origin.clone();
        self.query = Some(Operation::new(self.spec.pulse.clone(), move |reader| {
            answer(reader, &state, &origin, query)
        }));
        Ok(())
    }
    fn requested(&self) -> &[InputRequest] {
        self.query.as_ref().map_or(&[], Operation::requested)
    }
    fn step(&mut self, input: Input<'_>) -> Result<QueryStep> {
        check(&self.spec.pulse)?;
        let (consumed, progress) = self
            .query
            .as_mut()
            .ok_or_else(|| invalid("no active native query"))?
            .step(input)?;
        let event = match progress {
            Progress::Need => QueryEvent::NeedInput,
            Progress::Continue => QueryEvent::Continue,
            Progress::Done(answer) => {
                self.query.take();
                QueryEvent::Complete(answer)
            }
        };
        Ok(QueryStep { consumed, event })
    }
    fn decoder(&mut self, selection: &Selection, spec: &DecodeSpec) -> Result<SelectedDecode> {
        selection.validate(&self.origin)?;
        if !spec.metadata_first.is_empty() {
            return Err(unsupported(Unsupported::Metadata));
        }
        if let Channels::Only(channels) = &selection.channels
            && (channels.len() != 1
                || self
                    .origin
                    .channel_key(&selection.target.image, &channels[0])?
                    != 0)
        {
            return Err(unsupported(Unsupported::Operation));
        }
        let pointer = IfdPointer(match &selection.target.level {
            Some(level) => self.origin.level_key(&selection.target.image, level)?,
            None => self.origin.image_key(&selection.target.image)?,
        });
        let bounds = *self
            .state
            .lock()
            .map_err(|_| invalid("poisoned native state"))?
            .bounds
            .get(&pointer.0)
            .ok_or_else(|| invalid("describe TIFF target before decode"))?;
        let region = selection.region.unwrap_or(bounds);
        bounds.admit(region)?;
        let output = spec.accept.choose(descriptor(), &[]).map_err(unsupported)?;
        let selected = Selected {
            target: selection.target.clone(),
            full_bounds: bounds,
            region,
            channels: selection.channels.clone(),
            registration: None,
        };
        selected.validate(selection, spec.limits)?;
        let plan = DecodePlan::new(
            zencodec::Raster::packed(region.width, region.height, output),
            Delivery::WholeFrame,
            ResourceEstimate::serial(Bound::Unknown),
        );
        let info = ImageInfo::new(
            ImageFormat::new(&FORMAT),
            bounds.width,
            bounds.height,
            Layout::Packed(output),
        )
        .with_metadata(Default::default(), MetadataKinds::NONE);
        Ok(SelectedDecode {
            selected,
            decoder: Box::new(TiffDecoder {
                state: self.state.clone(),
                pointer,
                region,
                plan,
                info,
                pulse: spec.pulse.clone(),
                phase: 0,
                operation: None,
            }),
        })
    }
}

struct TiffDecoder {
    state: Arc<Mutex<State>>,
    pointer: IfdPointer,
    region: Bounds,
    plan: DecodePlan,
    info: ImageInfo,
    pulse: SharedPulse,
    phase: u8,
    operation: Option<Operation<Vec<u8>>>,
}
fn decode_region<R: Read + Seek>(
    mut reader: R,
    state: &Arc<Mutex<State>>,
    pointer: IfdPointer,
    region: Bounds,
    pulse: &SharedPulse,
) -> Result<Vec<u8>> {
    let state = state.lock().map_err(|_| invalid("poisoned native state"))?;
    let original = state.header.as_ref().unwrap();
    let header = TiffHeader {
        byte_order: original.byte_order,
        variant: original.variant,
        first_ifd: pointer,
    };
    let mut decoder = header.open(&mut reader).with_limits(limits());
    decoder.next_image().map_err(native)?;
    if decoder.colortype().map_err(native)? != ColorType::Gray(8) {
        return Err(unsupported(Unsupported::PixelFormat));
    }
    let (width, height) = decoder.dimensions().map_err(native)?;
    Bounds::new(0, 0, width, height)?.admit(region)?;
    let (cw, ch) = decoder.chunk_dimensions();
    if cw == 0 || ch == 0 {
        return Err(invalid("invalid TIFF unit size"));
    }
    let right = region.x as u32 + region.width;
    let bottom = region.y as u32 + region.height;
    let mut output = Vec::new();
    let length = (region.width as usize)
        .checked_mul(region.height as usize)
        .ok_or_else(|| invalid("ROI output overflow"))?;
    output
        .try_reserve_exact(length)
        .map_err(|_| error_msg(ErrorKind::OutOfMemory, "ROI allocation failed"))?;
    output.resize(length, 0);
    for ty in region.y as u32 / ch..=(bottom - 1) / ch {
        for tx in region.x as u32 / cw..=(right - 1) / cw {
            check(pulse)?;
            let unit = ty * width.div_ceil(cw) + tx;
            let layout = decoder
                .image_coding_unit_layout(tiff::decoder::TiffCodingUnit(unit))
                .map_err(native)?;
            let stride = layout
                .row_stride
                .ok_or_else(|| invalid("native unit has no row stride"))?
                .get();
            let tiff::decoder::DecodingSampleBuffer::U8(bytes) =
                decoder.read_chunk(unit).map_err(native)?
            else {
                return Err(unsupported(Unsupported::PixelFormat));
            };
            let left = (region.x as u32).max(tx * cw);
            let top = (region.y as u32).max(ty * ch);
            let r = right.min((tx + 1) * cw);
            let b = bottom.min((ty + 1) * ch);
            for y in top..b {
                let source = (y - ty * ch) as usize * stride + (left - tx * cw) as usize;
                let dest = (y - region.y as u32) as usize * region.width as usize
                    + (left - region.x as u32) as usize;
                output[dest..dest + (r - left) as usize]
                    .copy_from_slice(&bytes[source..source + (r - left) as usize]);
            }
        }
    }
    Ok(output)
}

impl Decoder for TiffDecoder {
    fn requested(&self) -> &[InputRequest] {
        self.operation.as_ref().map_or(&[], Operation::requested)
    }
    fn info(&self) -> Option<&ImageInfo> {
        (self.phase > 0).then_some(&self.info)
    }
    fn plan(&self) -> Option<&DecodePlan> {
        (self.phase > 0).then_some(&self.plan)
    }
    fn step(&mut self, input: Input<'_>, sink: &mut dyn Sink) -> Result<DecodeStep> {
        check(&self.pulse)?;
        if self.phase == 0 {
            self.phase = 1;
            return Ok(DecodeStep::new(0, DecodeEvent::Info));
        }
        if self.phase != 1 {
            return Err(invalid("native selected decoder is terminal"));
        }
        if self.operation.is_none() {
            let state = self.state.clone();
            let pointer = self.pointer;
            let region = self.region;
            let pulse = self.pulse.clone();
            self.operation = Some(Operation::new(self.pulse.clone(), move |reader| {
                decode_region(reader, &state, pointer, region, &pulse)
            }));
        }
        let (consumed, progress) = self.operation.as_mut().unwrap().step(input)?;
        let event = match progress {
            Progress::Need => DecodeEvent::NeedInput,
            Progress::Continue => DecodeEvent::Continue,
            Progress::Done(bytes) => {
                check(&self.pulse)?;
                let pixels = PixelBuffer::from_vec(
                    bytes,
                    self.region.width,
                    self.region.height,
                    descriptor(),
                )
                .map_err(|_| invalid("native output geometry"))?;
                sink.begin(&FrameHeader::still(self.plan.raster.clone()))?;
                sink.frame(pixels)?;
                sink.end()?;
                self.operation.take();
                self.phase = 2;
                DecodeEvent::Done
            }
        };
        Ok(DecodeStep::new(consumed, event))
    }
}
