//! A deliberately small indexed document for protocol qualification.
//!
//! `ZDOC` is not a production format. Its header names images and 32-byte level
//! records; each level is tightly packed native RGB8. Levels may have signed
//! origins and independent horizontal/vertical dimensions. The fixture retains
//! its parsed index and decodes selected rows without reopening it.

use std::sync::Arc;
use zencodec::{
    Bound, ByteRange, ErrorKind, FormatInfo, FrameHeader, ImageFormat, ImageInfo, Input,
    InputRequest, Layout, MetadataKinds, Pass, ResourceEstimate, Result, Unsupported, check,
    decode::{DecodeEvent, DecodePlan, DecodeSpec, DecodeStep, Decoder, Delivery, Sink},
    document::*,
    error_msg, unsupported,
};
use zenpixels::{PixelDescriptor, PixelSlice};

const HEADER: u64 = 12;
const RECORD: u64 = 32;
static FORMAT_INFO: FormatInfo = FormatInfo::new("zdoc", "image/x-zdoc", "zdoc", 4, |b| {
    b.starts_with(b"ZDOC")
});

/// Synthetic indexed-document codec.
pub struct CatalogCodec;

impl DocumentCodec for CatalogCodec {
    fn document(&self, spec: &OpenSpec) -> Result<Box<dyn Document>> {
        Ok(Box::new(Catalog {
            origin: Origin::new(),
            spec: spec.clone(),
            header: None,
            records: None,
            pending: None,
            requested: Vec::new(),
        }))
    }
}

#[derive(Clone, Debug)]
struct Record {
    image: u32,
    level: u32,
    bounds: Bounds,
    data: u64,
}

struct Catalog {
    origin: Origin,
    spec: OpenSpec,
    header: Option<(u32, u32)>,
    records: Option<Arc<[Record]>>,
    pending: Option<Query>,
    requested: Vec<InputRequest>,
}

fn number(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}
fn bad(message: &'static str) -> zencodec::Error {
    error_msg(ErrorKind::Malformed, message)
}

impl Catalog {
    fn record(&self, target: &Target) -> Result<&Record> {
        let image = self.origin.image_key(&target.image)?;
        let level = target
            .level
            .as_ref()
            .map(|level| self.origin.level_key(&target.image, level))
            .transpose()?
            .unwrap_or(0);
        self.records
            .as_ref()
            .and_then(|records| {
                records.iter().find(|record| {
                    u64::from(record.image) == image && u64::from(record.level) == level
                })
            })
            .ok_or_else(|| error_msg(ErrorKind::Protocol, "describe the target before decoding"))
    }
    fn answer(&self, query: &Query) -> Result<Answer> {
        match query {
            Query::Describe(target) => Ok(Answer::Description(Description {
                target: target.clone(),
                bounds: self.record(target)?.bounds,
                registration: None,
            })),
            Query::List {
                listing,
                cursor,
                limit,
            } => {
                let start = cursor
                    .as_ref()
                    .map(|cursor| self.origin.cursor_key(listing, cursor))
                    .transpose()?
                    .unwrap_or(0) as usize;
                let all: Vec<Entry> = match listing {
                    Listing::Images => (0..self.header.unwrap().0)
                        .map(|key| Entry::Image(self.origin.image(key.into())))
                        .collect(),
                    Listing::Children(_) => return Err(unsupported(Unsupported::Operation)),
                    Listing::Levels(image) => {
                        let key = self.origin.image_key(image)?;
                        self.records
                            .as_ref()
                            .unwrap()
                            .iter()
                            .filter(|record| u64::from(record.image) == key)
                            .map(|record| {
                                Ok(Entry::Level(Level {
                                    id: self.origin.level(image, record.level.into())?,
                                    bounds: record.bounds,
                                    registration: None,
                                }))
                            })
                            .collect::<Result<_>>()?
                    }
                    Listing::Channels(image) => ["R", "G", "B"]
                        .into_iter()
                        .enumerate()
                        .map(|(key, name)| {
                            Ok(Entry::Channel(Channel {
                                id: self.origin.channel(image, key as u64)?,
                                name: Arc::from(name),
                                encoding: None,
                            }))
                        })
                        .collect::<Result<_>>()?,
                };
                if start > all.len() {
                    return Err(error_msg(ErrorKind::Protocol, "cursor past listing"));
                }
                let end = (start + usize::from(limit.get())).min(all.len());
                let next = (end < all.len())
                    .then(|| self.origin.cursor(listing.clone(), end as u64))
                    .transpose()?;
                Ok(Answer::Page {
                    entries: all[start..end].to_vec(),
                    next,
                })
            }
        }
    }
}

impl Document for Catalog {
    fn origin(&self) -> &Origin {
        &self.origin
    }
    fn spec(&self) -> &OpenSpec {
        &self.spec
    }
    fn start(&mut self, query: Query) -> Result<()> {
        if self.pending.is_some() {
            return Err(error_msg(ErrorKind::Protocol, "query already in flight"));
        }
        query.validate(&self.origin)?;
        if matches!(
            query,
            Query::List {
                listing: Listing::Children(_),
                ..
            }
        ) {
            return Err(unsupported(Unsupported::Operation));
        }
        check(&self.spec.pulse)?;
        self.pending = Some(query);
        self.requested.clear();
        match self.header {
            None => self
                .requested
                .push(InputRequest::Range(ByteRange::new(0, HEADER))),
            Some((_, count))
                if self.records.is_none()
                    && !matches!(
                        self.pending,
                        Some(Query::List {
                            listing: Listing::Images | Listing::Channels(_),
                            ..
                        })
                    ) =>
            {
                self.requested.push(InputRequest::Range(ByteRange::new(
                    HEADER,
                    u64::from(count) * RECORD,
                )));
            }
            _ => {}
        }
        Ok(())
    }
    fn requested(&self) -> &[InputRequest] {
        &self.requested
    }
    fn step(&mut self, input: Input<'_>) -> Result<QueryStep> {
        check(&self.spec.pulse)?;
        if self.pending.is_none() {
            return Err(error_msg(
                ErrorKind::Protocol,
                "no document query in flight",
            ));
        }
        let mut consumed = 0;
        if let Some(request) = self.requested.first() {
            if !request.is_met_by(&input) {
                return Ok(QueryStep {
                    consumed: 0,
                    event: QueryEvent::NeedInput,
                });
            }
            match self.header {
                None => {
                    if input.bytes.len() < HEADER as usize || &input.bytes[..4] != b"ZDOC" {
                        return Err(bad("bad document header"));
                    }
                    let images = number(input.bytes, 4);
                    let records = number(input.bytes, 8);
                    if images == 0 || images > 64 || records == 0 || records > 256 {
                        return Err(bad("fixture index bounds"));
                    }
                    self.header = Some((images, records));
                    consumed = HEADER as usize;
                    if !matches!(
                        self.pending,
                        Some(Query::List {
                            listing: Listing::Images | Listing::Channels(_),
                            ..
                        })
                    ) {
                        self.requested = vec![InputRequest::Range(ByteRange::new(
                            HEADER,
                            u64::from(records) * RECORD,
                        ))];
                        return Ok(QueryStep {
                            consumed,
                            event: QueryEvent::NeedInput,
                        });
                    }
                }
                Some((_, count)) => {
                    let count = count as usize;
                    if input.bytes.len() < count * RECORD as usize {
                        return Err(bad("truncated fixture index"));
                    }
                    let mut records = Vec::new();
                    for bytes in input.bytes[..count * RECORD as usize].as_chunks::<32>().0 {
                        let bounds = Bounds::new(
                            i64::from(i32::from_le_bytes(bytes[16..20].try_into().unwrap())),
                            i64::from(i32::from_le_bytes(bytes[20..24].try_into().unwrap())),
                            number(bytes, 8),
                            number(bytes, 12),
                        )?;
                        records.push(Record {
                            image: number(bytes, 0),
                            level: number(bytes, 4),
                            bounds,
                            data: u64::from_le_bytes(bytes[24..32].try_into().unwrap()),
                        });
                    }
                    self.records = Some(Arc::from(records));
                    consumed = count * RECORD as usize;
                }
            }
        }
        self.requested.clear();
        let query = self.pending.take().unwrap();
        let answer = self.answer(&query)?;
        Ok(QueryStep {
            consumed,
            event: QueryEvent::Complete(answer),
        })
    }
    fn decoder(&mut self, selection: &Selection, spec: &DecodeSpec) -> Result<SelectedDecode> {
        selection.validate(&self.origin)?;
        if let Channels::Only(channels) = &selection.channels {
            let keys = channels
                .iter()
                .map(|channel| self.origin.channel_key(&selection.target.image, channel))
                .collect::<Result<Vec<_>>>()?;
            if keys != [0, 1, 2] {
                return Err(unsupported(Unsupported::Operation));
            }
        }
        let record = self.record(&selection.target)?.clone();
        let region = selection.region.unwrap_or(record.bounds);
        record.bounds.admit(region)?;
        let descriptor = spec
            .accept
            .choose(PixelDescriptor::RGB8_SRGB, &[])
            .map_err(unsupported)?;
        let selected = Selected {
            target: selection.target.clone(),
            full_bounds: record.bounds,
            region,
            channels: selection.channels.clone(),
            registration: None,
        };
        selected.validate(selection, spec.limits)?;
        let plan = DecodePlan::new(
            zencodec::Raster::packed(region.width, region.height, descriptor),
            Delivery::Strips,
            ResourceEstimate::serial(Bound::AtMost(0)),
        );
        let info = ImageInfo::new(
            ImageFormat::new(&FORMAT_INFO),
            record.bounds.width,
            record.bounds.height,
            Layout::Packed(descriptor),
        )
        .with_metadata(Default::default(), MetadataKinds::ALL);
        Ok(SelectedDecode {
            selected,
            decoder: Box::new(RegionDecoder {
                record,
                region,
                plan,
                info,
                spec: spec.clone(),
                requested: Vec::new(),
                y: 0,
                phase: 0,
            }),
        })
    }
}

struct RegionDecoder {
    record: Record,
    region: Bounds,
    plan: DecodePlan,
    info: ImageInfo,
    spec: DecodeSpec,
    requested: Vec<InputRequest>,
    y: u32,
    phase: u8,
}
impl RegionDecoder {
    fn request(&mut self) {
        let x = (self.region.x - self.record.bounds.x) as u64;
        let y = (self.region.y - self.record.bounds.y) as u64 + u64::from(self.y);
        self.requested = vec![InputRequest::Range(ByteRange::new(
            self.record.data + (y * u64::from(self.record.bounds.width) + x) * 3,
            u64::from(self.region.width) * 3,
        ))];
    }
}
impl Decoder for RegionDecoder {
    fn requested(&self) -> &[InputRequest] {
        &self.requested
    }
    fn info(&self) -> Option<&ImageInfo> {
        (self.phase > 0).then_some(&self.info)
    }
    fn plan(&self) -> Option<&DecodePlan> {
        (self.phase > 0).then_some(&self.plan)
    }
    fn step(&mut self, input: Input<'_>, sink: &mut dyn Sink) -> Result<DecodeStep> {
        check(&self.spec.pulse)?;
        match self.phase {
            0 => {
                self.phase = 1;
                self.request();
                Ok(DecodeStep::new(0, DecodeEvent::Info))
            }
            1 | 2 => {
                if !self.requested[0].is_met_by(&input) {
                    return Ok(DecodeStep::new(0, DecodeEvent::NeedInput));
                }
                if self.phase == 1 {
                    sink.begin(&FrameHeader::still(self.plan.raster.clone()))?;
                    self.phase = 2;
                }
                let size = self.region.width as usize * 3;
                if input.bytes.len() < size {
                    return Err(bad("truncated ROI row"));
                }
                let rows = PixelSlice::new(
                    &input.bytes[..size],
                    self.region.width,
                    1,
                    size,
                    PixelDescriptor::RGB8_SRGB,
                )
                .map_err(|_| bad("bad ROI geometry"))?;
                sink.rows(self.y, rows, Pass::Final)?;
                self.y += 1;
                if self.y == self.region.height {
                    sink.end()?;
                    self.phase = 3;
                    self.requested.clear();
                    Ok(DecodeStep::new(size, DecodeEvent::Done))
                } else {
                    self.request();
                    Ok(DecodeStep::new(size, DecodeEvent::Rows))
                }
            }
            _ => Err(error_msg(
                ErrorKind::Protocol,
                "selected decoder is terminal",
            )),
        }
    }
}
