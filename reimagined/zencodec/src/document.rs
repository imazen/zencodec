//! An opened document, lazy discovery, and selection of native samples.
//!
//! A document retains indexes across requests. Discovery uses the same byte
//! requests as decoding; selecting an image produces the existing [`Decoder`]
//! rather than another pixel protocol. Containment, levels and channels are
//! distinct listings. A source must retain one revision across the operation.

use crate::{
    ErrorKind, Input, InputRequest, ResourceLimits, Result, SharedPulse, check,
    decode::{DecodeSpec, Decoder},
    error_msg,
};
use alloc::{boxed::Box, sync::Arc, vec::Vec};
use core::{fmt, num::NonZeroU16};

/// Identity namespace for one opened document. Not proof of transport revision.
#[derive(Clone, Default)]
pub struct Origin(Arc<()>);

/// An image belonging to one opened document, independent of file offset.
#[derive(Clone)]
pub struct ImageId {
    origin: Arc<()>,
    key: u64,
}

impl PartialEq for ImageId {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.origin, &other.origin) && self.key == other.key
    }
}
impl Eq for ImageId {}

impl fmt::Debug for ImageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImageId").finish_non_exhaustive()
    }
}

/// A native resolution of one image. Never an inferred reduction exponent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LevelId {
    image: ImageId,
    key: u64,
}

/// A native channel of one image. Its name is descriptive, not its identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelId {
    image: ImageId,
    key: u64,
}

impl Origin {
    /// Allocate a namespace for an open source and composition view.
    pub fn new() -> Self {
        Self::default()
    }
    /// Issue a stable image identity for a native key.
    pub fn image(&self, key: u64) -> ImageId {
        ImageId {
            origin: self.0.clone(),
            key,
        }
    }
    /// Recover a native image key; reject a foreign document first.
    pub fn image_key(&self, image: &ImageId) -> Result<u64> {
        if Arc::ptr_eq(&self.0, &image.origin) {
            Ok(image.key)
        } else {
            Err(error_msg(ErrorKind::Protocol, "foreign document image"))
        }
    }
    /// Issue a native level identity scoped to this image.
    pub fn level(&self, image: &ImageId, key: u64) -> Result<LevelId> {
        self.image_key(image)?;
        Ok(LevelId {
            image: image.clone(),
            key,
        })
    }
    /// Recover a native level key, checking document and parent image.
    pub fn level_key(&self, image: &ImageId, level: &LevelId) -> Result<u64> {
        self.image_key(image)?;
        if image == &level.image {
            Ok(level.key)
        } else {
            Err(error_msg(
                ErrorKind::Protocol,
                "level belongs to another image",
            ))
        }
    }
    /// Issue a native channel identity scoped to this image.
    pub fn channel(&self, image: &ImageId, key: u64) -> Result<ChannelId> {
        self.image_key(image)?;
        Ok(ChannelId {
            image: image.clone(),
            key,
        })
    }
    /// Recover a native channel key, checking document and parent image.
    pub fn channel_key(&self, image: &ImageId, channel: &ChannelId) -> Result<u64> {
        self.image_key(image)?;
        if image == &channel.image {
            Ok(channel.key)
        } else {
            Err(error_msg(
                ErrorKind::Protocol,
                "channel belongs to another image",
            ))
        }
    }
}

/// Exact bounds in a selected level's native stored sample coordinates.
/// Origins may be negative; orientation and display placement are independent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bounds {
    /// Left edge in native coordinates.
    pub x: i64,
    /// Top edge in native coordinates.
    pub y: i64,
    /// Number of sample columns.
    pub width: u32,
    /// Number of sample rows.
    pub height: u32,
}

impl Bounds {
    /// A nonempty rectangle whose end coordinates are representable.
    pub fn new(x: i64, y: i64, width: u32, height: u32) -> Result<Self> {
        let bounds = Self {
            x,
            y,
            width,
            height,
        };
        bounds.validate()?;
        Ok(bounds)
    }
    /// Validate geometry without clipping it.
    pub fn validate(self) -> Result<()> {
        if self.width == 0
            || self.height == 0
            || self.x.checked_add(i64::from(self.width)).is_none()
            || self.y.checked_add(i64::from(self.height)).is_none()
        {
            return Err(error_msg(ErrorKind::Protocol, "invalid stored bounds"));
        }
        Ok(())
    }
    /// Refuse a rectangle that is not wholly contained in these bounds.
    pub fn admit(self, region: Self) -> Result<()> {
        self.validate()?;
        region.validate()?;
        if region.x < self.x
            || region.y < self.y
            || region.x + i64::from(region.width) > self.x + i64::from(self.width)
            || region.y + i64::from(region.height) > self.y + i64::from(self.height)
        {
            return Err(error_msg(
                ErrorKind::Protocol,
                "region leaves selected level",
            ));
        }
        Ok(())
    }
}

/// An exact rational coordinate, with a positive denominator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rational {
    /// Signed numerator.
    pub numerator: i64,
    /// Positive denominator.
    pub denominator: core::num::NonZeroU32,
}

/// Established level-to-image sample registration: `image = level * scale + offset`.
/// `None` registration stays unknown, even when dimensions happen to match.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Registration {
    /// Horizontal scale.
    pub scale_x: Rational,
    /// Vertical scale; ripmap levels need not use the horizontal scale.
    pub scale_y: Rational,
    /// Horizontal origin offset.
    pub offset_x: Rational,
    /// Vertical origin offset.
    pub offset_y: Rational,
}

/// A selected native image and optional explicit level.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    /// Native image identity.
    pub image: ImageId,
    /// Native level identity; absent means the image's native default.
    pub level: Option<LevelId>,
}

impl Target {
    /// Select an image's native default level.
    pub fn new(image: ImageId) -> Self {
        Self { image, level: None }
    }
    /// Require this exact level of the selected image.
    pub fn with_level(mut self, level: LevelId) -> Self {
        self.level = Some(level);
        self
    }
    /// Check ownership before delegating to a native backend.
    pub fn validate(&self, origin: &Origin) -> Result<()> {
        origin.image_key(&self.image)?;
        if let Some(level) = &self.level {
            origin.level_key(&self.image, level)?;
        }
        Ok(())
    }
}

/// A native level, with its actual bounds and independently established mapping.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Level {
    /// Exact level identity.
    pub id: LevelId,
    /// Bounds in this level's coordinates.
    pub bounds: Bounds,
    /// Established sample mapping to the image; absent is unknown.
    pub registration: Option<Registration>,
}

/// A discoverable native channel. Storage/meaning remain native description facts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Channel {
    /// Exact native channel identity.
    pub id: ChannelId,
    /// Native channel name, retained verbatim for explicit inspection.
    pub name: Arc<str>,
    /// Established storage/code interpretation; absent is unknown, not normalized.
    pub encoding: Option<zenpixels::plane::PlaneEncoding>,
}

/// A listing relation. Reduced levels and channels are not containment children.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Listing {
    /// Top-level images, pages or parts.
    Images,
    /// Native containment, without assuming pyramid semantics.
    Children(ImageId),
    /// Explicit native resolutions of one image.
    Levels(ImageId),
    /// Native channels of one image.
    Channels(ImageId),
}

impl Listing {
    /// Check any parent identity against the opened document.
    pub fn validate(&self, origin: &Origin) -> Result<()> {
        match self {
            Self::Images => Ok(()),
            Self::Children(id) | Self::Levels(id) | Self::Channels(id) => {
                origin.image_key(id).map(|_| ())
            }
        }
    }
}

/// A continuation bound to one document and one exact listing relation.
#[derive(Clone)]
pub struct Cursor {
    origin: Arc<()>,
    listing: Listing,
    key: u64,
}

impl fmt::Debug for Cursor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Cursor").finish_non_exhaustive()
    }
}

impl Origin {
    /// Issue a continuation after a bounded page.
    pub fn cursor(&self, listing: Listing, key: u64) -> Result<Cursor> {
        listing.validate(self)?;
        Ok(Cursor {
            origin: self.0.clone(),
            listing,
            key,
        })
    }
    /// Recover the native continuation key; reject foreign or reused cursors.
    pub fn cursor_key(&self, listing: &Listing, cursor: &Cursor) -> Result<u64> {
        if !Arc::ptr_eq(&self.0, &cursor.origin) || listing != &cursor.listing {
            return Err(error_msg(
                ErrorKind::Protocol,
                "cursor belongs to another listing",
            ));
        }
        Ok(cursor.key)
    }
}

/// One member of a listing. The variant must match its requested relation.
#[derive(Clone, Debug)]
pub enum Entry {
    /// A top-level image or a containment child.
    Image(ImageId),
    /// A native resolution.
    Level(Level),
    /// A native channel.
    Channel(Channel),
}

/// Description before reconstructing samples. Not a complete metadata inventory.
#[derive(Clone, Debug)]
pub struct Description {
    /// Exact described image and level.
    pub target: Target,
    /// Actual native stored coordinates.
    pub bounds: Bounds,
    /// Established level-to-image registration; absence stays unknown.
    pub registration: Option<Registration>,
}

/// A bounded discovery request, or inspection of an already-issued identity.
#[derive(Clone, Debug)]
pub enum Query {
    /// Read a bounded page of a native relation.
    List {
        /// Native relation to enumerate.
        listing: Listing,
        /// Resume only this exact listing.
        cursor: Option<Cursor>,
        /// Maximum entries returned by this operation.
        limit: NonZeroU16,
    },
    /// Establish selected geometry without allocating reconstructed samples.
    Describe(Target),
}

impl Query {
    /// Start a bounded listing with no continuation.
    pub fn list(listing: Listing, limit: NonZeroU16) -> Self {
        Self::List {
            listing,
            cursor: None,
            limit,
        }
    }
    /// Validate all operation-scoped identities before native effects.
    pub fn validate(&self, origin: &Origin) -> Result<()> {
        match self {
            Self::List {
                listing, cursor, ..
            } => {
                listing.validate(origin)?;
                if let Some(cursor) = cursor {
                    origin.cursor_key(listing, cursor)?;
                }
                Ok(())
            }
            Self::Describe(target) => target.validate(origin),
        }
    }
    /// Validate a returned answer, including relation, ownership and page size.
    pub fn validate_answer(&self, origin: &Origin, answer: &Answer) -> Result<()> {
        match (self, answer) {
            (Self::Describe(target), Answer::Description(description))
                if target == &description.target =>
            {
                description.bounds.validate()
            }
            (Self::List { listing, limit, .. }, Answer::Page { entries, next }) => {
                if entries.len() > usize::from(limit.get()) {
                    return Err(error_msg(
                        ErrorKind::Protocol,
                        "document exceeded page limit",
                    ));
                }
                for entry in entries {
                    match (listing, entry) {
                        (Listing::Images | Listing::Children(_), Entry::Image(id)) => {
                            origin.image_key(id)?;
                        }
                        (Listing::Levels(image), Entry::Level(level)) => {
                            origin.level_key(image, &level.id)?;
                            level.bounds.validate()?;
                        }
                        (Listing::Channels(image), Entry::Channel(channel)) => {
                            origin.channel_key(image, &channel.id)?;
                        }
                        _ => {
                            return Err(error_msg(
                                ErrorKind::Protocol,
                                "document returned another relation",
                            ));
                        }
                    }
                }
                if let Some(cursor) = next {
                    origin.cursor_key(listing, cursor)?;
                }
                Ok(())
            }
            _ => Err(error_msg(
                ErrorKind::Protocol,
                "document returned another answer",
            )),
        }
    }
}

/// Owned completion of one metadata operation. Exhaustion differs from refusal.
#[derive(Clone, Debug)]
pub enum Answer {
    /// Bounded entries and optional continuation; no continuation means exhaustion.
    Page {
        /// Entries returned in this operation.
        entries: Vec<Entry>,
        /// Continuation for this exact listing.
        next: Option<Cursor>,
    },
    /// Established selected geometry.
    Description(Description),
}

/// One discovery step. No pixel sink is involved in metadata discovery.
#[derive(Debug)]
pub enum QueryEvent {
    /// The next absolute byte request is not yet satisfied.
    NeedInput,
    /// Work can continue without more bytes; drivers may yield here.
    Continue,
    /// This query completed; the document remains open for another query.
    Complete(Answer),
}

/// Prefix consumption and discovery progress from one step.
#[derive(Debug)]
pub struct QueryStep {
    /// Bytes consumed from the lent input window.
    pub consumed: usize,
    /// Current progress or owned completion.
    pub event: QueryEvent,
}

/// Exact native channel selection. Unsupported selections must refuse.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Channels {
    /// All channels in the backend's qualified native output.
    #[default]
    All,
    /// These exact channel identities in this order; never implicit RGB conversion.
    Only(Arc<[ChannelId]>),
}

/// Required selected samples. Region coordinates belong to the selected level.
#[derive(Clone, Debug)]
pub struct Selection {
    /// Image and optional explicit level.
    pub target: Target,
    /// Exact stored ROI; absent requests the entire selected level.
    pub region: Option<Bounds>,
    /// Exact channel selection.
    pub channels: Channels,
}

impl Selection {
    /// All qualified native samples of this target.
    pub fn new(target: Target) -> Self {
        Self {
            target,
            region: None,
            channels: Channels::All,
        }
    }
    /// Require an exact region, without clipping.
    pub fn with_region(mut self, region: Bounds) -> Self {
        self.region = Some(region);
        self
    }
    /// Require exact native channels in this order.
    pub fn with_channels(mut self, channels: Arc<[ChannelId]>) -> Self {
        self.channels = Channels::Only(channels);
        self
    }
    /// Refuse foreign/duplicate channels before native decoding begins.
    pub fn validate(&self, origin: &Origin) -> Result<()> {
        self.target.validate(origin)?;
        if let Some(region) = self.region {
            region.validate()?;
        }
        if let Channels::Only(channels) = &self.channels {
            if channels.is_empty() {
                return Err(error_msg(ErrorKind::Protocol, "empty channel selection"));
            }
            for (index, channel) in channels.iter().enumerate() {
                origin.channel_key(&self.target.image, channel)?;
                if channels[..index].contains(channel) {
                    return Err(error_msg(
                        ErrorKind::Protocol,
                        "duplicate channel selection",
                    ));
                }
            }
        }
        Ok(())
    }
}

/// Immutable selection receipt retained alongside the ordinary decoder.
#[derive(Clone, Debug)]
pub struct Selected {
    /// Exact image and level selected.
    pub target: Target,
    /// Full native bounds before the ROI.
    pub full_bounds: Bounds,
    /// Actual delivered extent in native level coordinates.
    pub region: Bounds,
    /// Actual channels, with no silent selection substitution.
    pub channels: Channels,
    /// Established level-to-image mapping; unknown stays unknown.
    pub registration: Option<Registration>,
}

impl Selected {
    /// Check identity, full-image limits and exact request matching before effects.
    pub fn validate(&self, requested: &Selection, limits: ResourceLimits) -> Result<()> {
        self.full_bounds.admit(self.region)?;
        if self.target != requested.target
            || self.channels != requested.channels
            || self.region != requested.region.unwrap_or(self.full_bounds)
        {
            return Err(error_msg(
                ErrorKind::Protocol,
                "selection receipt differs from request",
            ));
        }
        limits.check_dimensions(self.full_bounds.width, self.full_bounds.height)
    }
}

/// One selected operation: coordinate receipt plus the existing pixel protocol.
pub struct SelectedDecode {
    /// Immutable native selection facts. Not evidence of efficient partial decoding.
    pub selected: Selected,
    /// The ordinary decoder, driven by the existing byte and row interfaces.
    pub decoder: Box<dyn Decoder>,
}

/// Controls fixed for the opened document and all metadata operations.
#[derive(Clone, Debug, Default)]
pub struct OpenSpec {
    /// Source/native limits. Unknown native memory must refuse a configured cap.
    pub limits: ResourceLimits,
    /// One retained progress/cancellation condition; drivers use the same handle.
    pub pulse: SharedPulse,
}

impl OpenSpec {
    /// Bind per-selection preferences while preserving all opened-document gates
    /// and both cancellation conditions. Progress belongs to the selection pulse.
    pub fn bind_decode(&self, requested: &DecodeSpec) -> Result<DecodeSpec> {
        use how_far::Pulse as _;
        let mut spec = requested.clone();
        spec.limits = spec.limits.intersection(self.limits);
        spec.pulse = how_far::WithStop::new(&requested.pulse, self.pulse.clone())
            .share()
            .map_err(|_| {
                error_msg(
                    ErrorKind::Internal,
                    "owned operation pulse could not be retained",
                )
            })?;
        Ok(spec)
    }
}

/// Shareable native options for opening a browsable document.
pub trait DocumentCodec: Send + Sync + 'static {
    /// Construct a cheap document owner, without source reads or pixel allocations.
    fn document(&self, spec: &OpenSpec) -> Result<Box<dyn Document>>;
}

/// A retained document with one metadata query in flight at a time.
///
/// `start` performs no input I/O. A successful completion leaves the document
/// reusable. Source failures/cancellation are terminal in the driver. Invalid
/// selections and unsupported queries refuse before affecting a current query.
pub trait Document: Send + 'static {
    /// Namespace of every issued image, level, channel and continuation.
    fn origin(&self) -> &Origin;
    /// Retained controls used by both discovery and the driver.
    fn spec(&self) -> &OpenSpec;
    /// Begin one query; refuse if another query is still in flight.
    fn start(&mut self, query: Query) -> Result<()>;
    /// Absolute windows needed by the next discovery step.
    fn requested(&self) -> &[InputRequest];
    /// Advance discovery without pixel reconstruction or blocking input I/O.
    fn step(&mut self, input: Input<'_>) -> Result<QueryStep>;
    /// Prepare the existing decoder protocol for exact native selection.
    /// No pixel reads/publication are allowed until the returned decoder is driven.
    fn decoder(&mut self, selection: &Selection, spec: &DecodeSpec) -> Result<SelectedDecode>;
}

/// Validate a query and shared cancellation before a driver starts native work.
pub fn admit_query(document: &dyn Document, query: &Query) -> Result<()> {
    check(&document.spec().pulse)?;
    query.validate(document.origin())
}
