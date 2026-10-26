//! A real TIFF directory graph over the candidate operation-bound identities.
use super::{
    collection::{Collection, CollectionError, located},
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
    input: R,
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
    pub fn open(mut input: R, limits: Allowances) -> Result<Self, Failure> {
        let header = TiffHeader::parse(&mut input)?;
        Ok(Self {
            input,
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
            if item == target {
                return Ok(true);
            }
            if seen.contains(&item) {
                continue;
            }
            seen.push(item);
            pending.extend(
                self.edges
                    .iter()
                    .filter(|(parent, _)| *parent == item)
                    .map(|(_, child)| child),
            );
        }
        Ok(false)
    }

    pub fn next(&mut self, cursor: &mut Cursor) -> Result<Option<Relation>, Failure> {
        self.origin
            .key(&cursor.owner)
            .map_err(|_| SelectionError::ForeignCursor)?;
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
        check: impl FnMut() -> Result<(), Failure>,
    ) -> Result<Region, Failure> {
        let key = self.origin.key(image)?;
        if composition != Composition::Composited {
            return Err(SelectionError::UnsupportedComposition.into());
        }
        tiff_region::decode_directory(
            &mut self.input,
            IfdPointer(key),
            region.into(),
            self.limits,
            check,
        )
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
        if view != Composition::Composited {
            return Err(located(SelectionError::UnsupportedComposition.into()));
        }
        Ok(Self {
            items: TiffItems::open(input, limits).map_err(located)?,
        })
    }
}
impl<R: Read + Seek> Collection for TiffCollection<R> {
    type Cursor = Cursor;
    fn view(&self) -> Composition {
        Composition::Composited
    }
    fn images(&mut self) -> Result<Cursor, zencodec::At<CollectionError>> {
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
        self.items
            .decode(image, request.region, Composition::Composited, || Ok(()))
            .and_then(Region::into_pixels)
            .map(super::tiff_pixels::StoredRaster::into_image)
            .map_err(located)
    }
}
