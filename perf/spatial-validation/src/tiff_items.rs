//! A real TIFF directory graph over the candidate operation-bound identities.
use super::{
    collection::{Collection, CollectionError, located},
    control::{CheckedReader, Control},
    description::Description,
    image_items::{
        Composition, ImageId, ImageRequest, Origin, RegionRequest, Relation, RelationKind,
        SelectionError,
    },
    tiff_region::{self, Allowances, Failure, Region},
};
use std::io::{Read, Seek};
use tiff::{
    decoder::{Limits, TiffHeader},
    tags::{IfdPointer, Tag},
};

pub struct TiffItems<R> {
    input: CheckedReader<R>,
    control: Control,
    header: TiffHeader,
    origin: Origin,
    limits: Allowances,
    edges: Vec<(u64, u64)>,
    discoveries: usize,
}

pub struct Cursor {
    owner: ImageId,
    parent: Option<ImageId>,
    next: Option<IfdPointer>,
    children: Vec<IfdPointer>,
    index: usize,
    visited: Vec<IfdPointer>,
}

impl<R: Read + Seek> TiffItems<R> {
    pub fn open(input: R, limits: Allowances) -> Result<Self, Failure> {
        Self::open_controlled(input, limits, Control::default())
    }

    fn open_controlled(input: R, limits: Allowances, control: Control) -> Result<Self, Failure> {
        control.check()?;
        let mut input = control.reader(input);
        let header = TiffHeader::parse(&mut input)?;
        control.check()?;
        Ok(Self {
            input,
            control,
            header,
            limits,
            origin: Origin::default(),
            edges: Vec::new(),
            discoveries: 0,
        })
    }

    /// Discovery is lazy: constructing the cursor performs no directory read.
    pub fn images(&self) -> Cursor {
        Cursor {
            owner: self.origin.image(0),
            parent: None,
            next: (self.header.first_ifd.0 != 0).then_some(self.header.first_ifd),
            children: Vec::new(),
            index: 0,
            visited: Vec::new(),
        }
    }

    fn native_limits(&self) -> Limits {
        let mut limits = Limits::default();
        limits.ifd_value_size = self.limits.tag;
        limits.decoding_buffer_size = self.limits.unit;
        limits.intermediate_buffer_size = self.limits.unit;
        limits
    }

    /// Read only the selected directory's SubIFD list, bounded by the native tag
    /// allowance. No pixel headers or child directories are decoded here.
    pub fn children(&mut self, parent: &ImageId) -> Result<Cursor, Failure> {
        let key = self.origin.key(parent)?;
        self.control.check()?;
        let limits = self.native_limits();
        let mut decoder = self.header.open(&mut self.input).with_limits(limits);
        let directory = decoder.read_directory(IfdPointer(key))?;
        let children = decoder
            .read_directory_tags(&directory)
            .find_tag(Tag::SubIfd)?
            .map(|value| value.into_ifd_vec())
            .transpose()?
            .unwrap_or_default();
        if children.len() > self.limits.discoveries {
            return Err(SelectionError::DiscoveryLimit.into());
        }
        self.control.check()?;
        Ok(Cursor {
            owner: self.origin.image(0),
            parent: Some(parent.clone()),
            next: children.first().copied(),
            children,
            index: 0,
            visited: Vec::new(),
        })
    }

    fn contains_path(&self, start: u64, target: u64) -> Result<bool, Failure> {
        let capacity = self
            .edges
            .len()
            .checked_add(1)
            .ok_or(SelectionError::DiscoveryLimit)?;
        let mut pending = Vec::new();
        pending.try_reserve_exact(capacity)?;
        pending.push(start);
        let mut seen = Vec::new();
        seen.try_reserve_exact(capacity)?;
        while let Some(item) = pending.pop() {
            self.control.check()?;
            if item == target {
                return Ok(true);
            }
            if seen.contains(&item) {
                continue;
            }
            seen.push(item);
            for &(parent, child) in &self.edges {
                self.control.check()?;
                if parent == item {
                    pending.push(child);
                }
            }
        }
        Ok(false)
    }

    pub fn next(&mut self, cursor: &mut Cursor) -> Result<Option<Relation>, Failure> {
        self.origin
            .key(&cursor.owner)
            .map_err(|_| SelectionError::ForeignCursor)?;
        self.control.check()?;
        let Some(pointer) = cursor.next else {
            return Ok(None);
        };
        if self.discoveries >= self.limits.discoveries
            || (cursor.parent.is_none() && cursor.index >= self.limits.pages as usize)
        {
            return Err(SelectionError::DiscoveryLimit.into());
        }
        if cursor.parent.is_none() && cursor.visited.contains(&pointer) {
            return Err(SelectionError::Cycle.into());
        }
        if let Some(parent) = &cursor.parent {
            let parent = self.origin.key(parent)?;
            if self.contains_path(pointer.0, parent)? {
                return Err(SelectionError::Cycle.into());
            }
        }
        let next = if cursor.parent.is_some() {
            cursor.children.get(cursor.index + 1).copied()
        } else {
            let limits = self.native_limits();
            self.header
                .open(&mut self.input)
                .with_limits(limits)
                .read_directory(pointer)?
                .next()
        };
        // All fallible storage growth precedes committing the discovery state.
        let edge = cursor
            .parent
            .as_ref()
            .map(|parent| self.origin.key(parent).map(|key| (key, pointer.0)))
            .transpose()?;
        let new_edge = edge.filter(|edge| !self.edges.contains(edge));
        if new_edge.is_some() {
            self.edges.try_reserve_exact(1)?;
        }
        if cursor.parent.is_none() {
            cursor.visited.try_reserve_exact(1)?;
        }
        self.control.check()?;
        if let Some(edge) = new_edge {
            self.edges.push(edge);
        }
        self.discoveries += 1;
        if cursor.parent.is_none() {
            cursor.visited.push(pointer);
        }
        cursor.index += 1;
        cursor.next = next;
        Ok(Some(Relation {
            parent: cursor.parent.clone(),
            image: self.origin.image(pointer.0),
            kind: if cursor.parent.is_some() {
                RelationKind::SubImage
            } else {
                RelationKind::TopLevel
            },
        }))
    }

    pub fn decode(
        &mut self,
        image: &ImageId,
        region: impl Into<RegionRequest>,
        composition: Composition,
        mut check: impl FnMut() -> Result<(), Failure>,
    ) -> Result<Region, Failure> {
        let key = self.origin.key(image)?;
        if composition != Composition::Composited {
            return Err(SelectionError::UnsupportedComposition.into());
        }
        self.control.check()?;
        let control = &self.control;
        tiff_region::decode_directory(
            &mut self.input,
            IfdPointer(key),
            region.into(),
            self.limits,
            || {
                control.check()?;
                check()
            },
        )
    }

    fn describe(&mut self, image: &ImageId) -> Result<Description, Failure> {
        let key = self.origin.key(image)?;
        self.control.check()?;
        let limits = self.native_limits();
        let mut decoder = self.header.open(&mut self.input).with_limits(limits);
        let directory = decoder.read_directory(IfdPointer(key))?;
        let mut tags = decoder.read_directory_tags(&directory);
        // Inspect directory facts without initializing the raster decoder. A
        // successful description does not promise support for its compression.
        let stored_size = (
            tags.get_tag(Tag::ImageWidth)?.into_u32()?,
            tags.get_tag(Tag::ImageLength)?.into_u32()?,
        );
        let orientation = tags
            .find_tag(Tag::Orientation)?
            .map(|value| value.into_u32())
            .transpose()?
            .unwrap_or(1);
        let stored_orientation = u8::try_from(orientation)
            .ok()
            .and_then(zenpixels::Orientation::from_exif)
            .ok_or_else(|| std::io::Error::other("invalid stored TIFF orientation"))?;
        self.control.check()?;
        let description = Description::new(stored_size).with_orientation(stored_orientation);
        description.validate()?;
        Ok(description)
    }
}

/// Composition fixes the discovered view. Geometry belongs to each decode so
/// differently sized pages and children can share one opened source operation.
pub struct TiffCollection<R> {
    items: TiffItems<R>,
}

impl<R: Read + Seek> TiffCollection<R> {
    pub fn open(
        input: R,
        view: Composition,
        limits: Allowances,
    ) -> Result<Self, zencodec::At<CollectionError>> {
        Self::open_controlled(input, view, limits, Control::default())
    }
    pub fn open_with_stop(
        input: R,
        view: Composition,
        limits: Allowances,
        stop: zencodec::StopToken,
    ) -> Result<Self, zencodec::At<CollectionError>> {
        Self::open_controlled(input, view, limits, Control::new(stop))
    }
    fn open_controlled(
        input: R,
        view: Composition,
        limits: Allowances,
        control: Control,
    ) -> Result<Self, zencodec::At<CollectionError>> {
        if view != Composition::Composited {
            return Err(located(SelectionError::UnsupportedComposition.into()));
        }
        Ok(Self {
            items: TiffItems::open_controlled(input, limits, control).map_err(located)?,
        })
    }
}
impl<R: Read + Seek> Collection for TiffCollection<R> {
    fn describe(&mut self, image: &ImageId) -> Result<Description, zencodec::At<CollectionError>> {
        self.items.describe(image).map_err(located)
    }

    type Cursor = Cursor;
    fn view(&self) -> Composition {
        Composition::Composited
    }
    fn images(&mut self) -> Result<Cursor, zencodec::At<CollectionError>> {
        self.items.control.check().map_err(located)?;
        Ok(self.items.images())
    }
    fn children(&mut self, image: &ImageId) -> Result<Cursor, zencodec::At<CollectionError>> {
        self.items.children(image).map_err(located)
    }
    fn next(
        &mut self,
        cursor: &mut Cursor,
    ) -> Result<Option<ImageId>, zencodec::At<CollectionError>> {
        self.items
            .next(cursor)
            .map(|item| item.map(|v| v.image))
            .map_err(located)
    }
    fn decode(
        &mut self,
        image: &ImageId,
        request: &ImageRequest,
    ) -> Result<super::image_output::ImageOutput, zencodec::At<CollectionError>> {
        let image = self
            .items
            .decode(image, request.region, Composition::Composited, || Ok(()))
            .and_then(Region::into_pixels)
            .and_then(super::tiff_pixels::StoredRaster::into_image)
            .map_err(located)?;
        self.items.control.check().map_err(located)?;
        Ok(image)
    }
}
