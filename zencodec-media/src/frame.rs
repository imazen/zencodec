//! Decoded frames and owned software planes.
//!
//! Shared frame owners and mapping contracts live in [`zencodec::frame`]. The
//! existing media import paths name those exact types. Software plane storage
//! stays here because accounted allocations use `zencodec-io`.

use alloc::{boxed::Box, vec::Vec};

pub use zencodec::frame::{
    ChromaSiting, ColorClaim, Crop, Frame, FrameGeometry, FrameInterpretation, FrameTiming,
    MapError, MappedPlanes, Picture, PlaneError, PlaneRole, PlaneSamples, PlaneView, Presentation,
    SampleEncoding, SampleEncodingError, Subsampling,
};

/// Words of one owned plane.
#[derive(Clone, Debug)]
pub enum PlaneData {
    /// 8-bit words.
    U8(Vec<u8>),
    /// 16-bit words.
    U16(Vec<u16>),
    /// Shared 8-bit storage admitted before allocation. A frame, plane clone
    /// or retained view keeps the entire backing allocation charged.
    AccountedU8(zencodec_io::storage::SharedStorage<u8>),
    /// Shared 16-bit storage, preserving native word alignment and encoding.
    AccountedU16(zencodec_io::storage::SharedStorage<u16>),
}

/// One owned plane.
#[derive(Clone, Debug)]
pub struct OwnedPlane {
    /// The words.
    pub data: PlaneData,
    /// Width in samples.
    pub width: usize,
    /// Height in rows.
    pub height: usize,
    /// Distance between rows, in bytes.
    pub stride_bytes: usize,
}

/// Owned planes in their native words: the [`Picture`] of a software
/// decoder.
#[derive(Clone, Debug)]
pub struct OwnedPlanes {
    planes: Vec<OwnedPlane>,
    encoding: SampleEncoding,
}

impl OwnedPlanes {
    /// Planes sharing one sample encoding.
    ///
    /// # Errors
    ///
    /// A [`PlaneError`] when a plane's storage does not fit its geometry.
    pub fn new(planes: Vec<OwnedPlane>, encoding: SampleEncoding) -> Result<Self, PlaneError> {
        let me = Self { planes, encoding };
        for i in 0..me.planes.len() {
            me.view(i)?;
        }
        Ok(me)
    }

    fn view(&self, i: usize) -> Result<PlaneView<'_>, PlaneError> {
        let p = &self.planes[i];
        let samples = match &p.data {
            PlaneData::U8(v) => PlaneSamples::U8(v),
            PlaneData::U16(v) => PlaneSamples::U16(v),
            PlaneData::AccountedU8(v) => PlaneSamples::U8(v.as_slice()),
            PlaneData::AccountedU16(v) => PlaneSamples::U16(v.as_slice()),
        };
        PlaneView::new(samples, p.width, p.height, p.stride_bytes, self.encoding)
    }
}

struct OwnedMapped<'a>(&'a OwnedPlanes);

impl MappedPlanes for OwnedMapped<'_> {
    fn plane_count(&self) -> usize {
        self.0.planes.len()
    }

    fn plane(&self, index: usize) -> Option<PlaneView<'_>> {
        if index >= self.0.planes.len() {
            return None;
        }
        self.0.view(index).ok()
    }
}

impl Picture for OwnedPlanes {
    fn map(&self) -> Result<Box<dyn MappedPlanes + '_>, MapError> {
        Ok(Box::new(OwnedMapped(self)))
    }
}
