//! Decoded frames and owned software planes.
//!
//! Shared frame owners and mapping contracts live in [`zencodec::frame`]. The
//! existing media import paths name those exact types. Software plane storage
//! stays here because accounted allocations use `zencodec-io`.

use alloc::{boxed::Box, vec::Vec};
use zenpixels::plane::{AnyPlane, Plane, PlaneBuf, PlaneEncoding};
use zenpixels::sample::SampleEncoding;

pub use zencodec::frame::{
    ColorClaim, Crop, Frame, FrameGeometry, FrameInterpretation, FrameTiming, MapError,
    MappedPlanes, Picture, PlaneError, PlaneRole, Presentation,
};

/// One owned plane in its native words. Plane geometry (width, height,
/// stride in samples) comes from [`zenpixels::plane::Plane`].
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum OwnedPlane {
    /// 8-bit words.
    U8(PlaneBuf<u8>),
    /// 16-bit words.
    U16(PlaneBuf<u16>),
    /// Floating-point samples.
    F32(PlaneBuf<f32>),
    /// Shared 8-bit storage admitted before allocation. A frame, plane clone
    /// or retained view keeps the entire backing allocation charged.
    AccountedU8(Plane<zencodec_io::storage::SharedStorage<u8>>),
    /// Shared 16-bit storage, preserving native word alignment and encoding.
    AccountedU16(Plane<zencodec_io::storage::SharedStorage<u16>>),
    /// Shared floating-point storage admitted before allocation.
    AccountedF32(Plane<zencodec_io::storage::SharedStorage<f32>>),
}

impl OwnedPlane {
    /// Borrow the plane without copying.
    pub fn view(&self) -> AnyPlane<'_> {
        match self {
            Self::U8(p) => AnyPlane::U8(p.view()),
            Self::U16(p) => AnyPlane::U16(p.view()),
            Self::F32(p) => AnyPlane::F32(p.view()),
            Self::AccountedU8(p) => AnyPlane::U8(p.view()),
            Self::AccountedU16(p) => AnyPlane::U16(p.view()),
            Self::AccountedF32(p) => AnyPlane::F32(p.view()),
        }
    }
}

/// Owned planes in their native words: the [`Picture`] of a software
/// decoder.
#[derive(Clone, Debug)]
pub struct OwnedPlanes {
    planes: Vec<(OwnedPlane, PlaneEncoding)>,
}

/// Owned planes disagree with their sample encoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum OwnedPlanesError {
    /// A plane's word type disagrees with the sample encoding.
    StorageMismatch {
        /// The plane.
        index: usize,
    },
}

impl core::fmt::Display for OwnedPlanesError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::StorageMismatch { index } => {
                write!(
                    f,
                    "plane {index} storage disagrees with its sample encoding"
                )
            }
        }
    }
}

impl core::error::Error for OwnedPlanesError {}

impl OwnedPlanes {
    /// Planes sharing one sample encoding. Plane geometry was checked when
    /// each plane was built.
    ///
    /// # Errors
    ///
    /// [`OwnedPlanesError::StorageMismatch`] when a plane's word type
    /// disagrees with `encoding`.
    pub fn new(
        planes: Vec<OwnedPlane>,
        encoding: SampleEncoding,
    ) -> Result<Self, OwnedPlanesError> {
        Self::from_encoded_planes(
            planes
                .into_iter()
                .map(|p| (p, PlaneEncoding::Code(encoding)))
                .collect(),
        )
    }

    /// Adopt independently encoded planes without copying their sample storage.
    ///
    /// # Errors
    /// Refuses a plane whose sample type disagrees with its encoding.
    pub fn from_encoded_planes(
        planes: Vec<(OwnedPlane, PlaneEncoding)>,
    ) -> Result<Self, OwnedPlanesError> {
        for (index, (plane, encoding)) in planes.iter().enumerate() {
            if plane.view().channel_type() != encoding.channel_type() {
                return Err(OwnedPlanesError::StorageMismatch { index });
            }
        }
        Ok(Self { planes })
    }

    /// Encoding of the selected plane; absent only past the last plane.
    pub fn encoding(&self, index: usize) -> Option<PlaneEncoding> {
        self.planes.get(index).map(|p| p.1)
    }
}

struct OwnedMapped<'a>(&'a OwnedPlanes);

impl MappedPlanes for OwnedMapped<'_> {
    fn plane_count(&self) -> usize {
        self.0.planes.len()
    }

    fn plane(&self, index: usize) -> Option<AnyPlane<'_>> {
        self.0.planes.get(index).map(|p| p.0.view())
    }

    fn encoding(&self, index: usize) -> Option<PlaneEncoding> {
        self.0.encoding(index)
    }
}

impl Picture for OwnedPlanes {
    fn map(&self) -> Result<Box<dyn MappedPlanes + '_>, MapError> {
        Ok(Box::new(OwnedMapped(self)))
    }
}
