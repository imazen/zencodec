//! A sink that keeps every frame in memory.

use std::sync::Arc;
use zencodec::decode::Sink;
use zencodec::{ErrorKind, FrameHeader, Layout, Pass, Result, error, error_msg};
use zenpixels::plane::{AnyPlane, PlanarLayout, PlanarSlice, PlaneBuf, PlaneRef, PlaneSample};
use zenpixels::{ChannelType, PixelBuffer, PixelSliceMut};

/// One owned plane.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum OwnedPlane {
    /// 8-bit samples.
    U8(PlaneBuf<u8>),
    /// 16-bit samples.
    U16(PlaneBuf<u16>),
    /// Float samples.
    F32(PlaneBuf<f32>),
}

impl OwnedPlane {
    /// Borrow the plane.
    pub fn view(&self) -> AnyPlane<'_> {
        match self {
            Self::U8(p) => AnyPlane::U8(p.view()),
            Self::U16(p) => AnyPlane::U16(p.view()),
            Self::F32(p) => AnyPlane::F32(p.view()),
        }
    }
}

/// An owned planar frame: one buffer per plane, in layout order.
#[derive(Clone, Debug)]
pub struct PlanarPixels {
    /// The planes.
    pub planes: Vec<OwnedPlane>,
    /// What they are.
    pub layout: Arc<PlanarLayout>,
}

/// A frame's samples.
#[derive(Debug)]
#[non_exhaustive]
pub enum Pixels {
    /// Packed pixels.
    Packed(PixelBuffer),
    /// Separate planes.
    Planar(PlanarPixels),
}

impl Pixels {
    /// The packed buffer, if packed.
    pub fn packed(&self) -> Option<&PixelBuffer> {
        match self {
            Self::Packed(p) => Some(p),
            Self::Planar(_) => None,
        }
    }

    /// The planes, if planar.
    pub fn planar(&self) -> Option<&PlanarPixels> {
        match self {
            Self::Planar(p) => Some(p),
            Self::Packed(_) => None,
        }
    }
}

/// A collected frame.
#[derive(Debug)]
pub struct Frame {
    /// What the decoder declared.
    pub header: FrameHeader,
    /// The samples.
    pub pixels: Pixels,
}

struct Building {
    header: FrameHeader,
    pixels: Option<Pixels>,
    /// Final rows received so far; rows arrive in order.
    covered: u32,
}

/// Collects decoded frames. A frame given whole is kept without a copy.
#[derive(Default)]
pub struct Collect {
    frames: Vec<Frame>,
    current: Option<Building>,
}

impl Collect {
    /// An empty collector.
    pub fn new() -> Self {
        Self::default()
    }

    /// The frames collected so far.
    pub fn frames(&self) -> &[Frame] {
        &self.frames
    }

    /// Take the frames.
    pub fn into_frames(self) -> Vec<Frame> {
        self.frames
    }

    fn building(&mut self) -> Result<&mut Building> {
        self.current
            .as_mut()
            .ok_or_else(|| error_msg(ErrorKind::Protocol, "rows outside a frame"))
    }
}

fn copy_plane<T: PlaneSample>(
    src: &PlaneRef<'_, T>,
    dst: &mut PlaneBuf<T>,
    at: usize,
) -> Result<()> {
    for (r, row) in src.rows().enumerate() {
        dst.row_mut(at + r)
            .filter(|d| d.len() == row.len())
            .ok_or_else(|| error_msg(ErrorKind::Protocol, "planar strip outside the frame"))?
            .copy_from_slice(row);
    }
    Ok(())
}

fn new_plane<T: PlaneSample + Default>(width: usize, height: usize) -> Result<PlaneBuf<T>> {
    let len = width
        .checked_mul(height)
        .ok_or_else(|| error(ErrorKind::OutOfMemory))?;
    let mut samples = Vec::new();
    samples
        .try_reserve_exact(len)
        .map_err(|_| error(ErrorKind::OutOfMemory))?;
    samples.resize(len, T::default());
    PlaneBuf::new(samples, width, height, width).map_err(|_| error(ErrorKind::Internal))
}

impl Sink for Collect {
    fn begin(&mut self, frame: &FrameHeader) -> Result<()> {
        if self.current.is_some() {
            return Err(error_msg(ErrorKind::Protocol, "frame began inside a frame"));
        }
        self.current = Some(Building {
            header: frame.clone(),
            pixels: None,
            covered: 0,
        });
        Ok(())
    }

    fn lend(&mut self, y: u32, rows: u32, pass: Pass) -> Result<PixelSliceMut<'_>> {
        let building = self.building()?;
        let raster = &building.header.raster;
        let Some(descriptor) = raster.descriptor() else {
            return Err(error_msg(
                ErrorKind::Protocol,
                "packed rows for a planar frame",
            ));
        };
        if rows == 0 || y.checked_add(rows).is_none_or(|end| end > raster.height) {
            return Err(error_msg(ErrorKind::Protocol, "rows outside the frame"));
        }
        if pass == Pass::Final {
            if y != building.covered {
                return Err(error_msg(ErrorKind::Protocol, "final rows out of order"));
            }
            building.covered += rows;
        }
        if building.pixels.is_none() {
            let buffer = PixelBuffer::try_new(raster.width, raster.height, descriptor)
                .map_err(|_| error(ErrorKind::OutOfMemory))?;
            building.pixels = Some(Pixels::Packed(buffer));
        }
        match building.pixels.as_mut() {
            Some(Pixels::Packed(buffer)) => Ok(buffer.rows_mut(y, rows)),
            _ => Err(error_msg(
                ErrorKind::Protocol,
                "packed rows after planar rows",
            )),
        }
    }

    fn frame(&mut self, pixels: PixelBuffer) -> Result<()> {
        let building = self.building()?;
        let raster = &building.header.raster;
        if building.pixels.is_some() || building.covered != 0 {
            return Err(error_msg(ErrorKind::Protocol, "whole frame after rows"));
        }
        if Some(pixels.descriptor()) != raster.descriptor()
            || pixels.width() != raster.width
            || pixels.height() != raster.height
        {
            return Err(error_msg(
                ErrorKind::Protocol,
                "frame does not match its header",
            ));
        }
        building.covered = raster.height;
        building.pixels = Some(Pixels::Packed(pixels));
        Ok(())
    }

    fn planes(&mut self, y: u32, strip: PlanarSlice<'_>) -> Result<()> {
        let building = self.building()?;
        let raster = &building.header.raster;
        let Layout::Planar { planes: layout, .. } = &raster.layout else {
            return Err(error_msg(
                ErrorKind::Protocol,
                "planar rows for a packed frame",
            ));
        };
        if *strip.layout() != **layout
            || strip.width() != raster.width
            || y != building.covered
            || strip.height() > raster.height - y
        {
            return Err(error_msg(
                ErrorKind::Protocol,
                "planar strip does not fit the frame",
            ));
        }
        if building.pixels.is_none() {
            let mut planes = Vec::with_capacity(layout.planes().len());
            for (index, spec) in layout.planes().iter().enumerate() {
                let (w, h) = layout
                    .plane_size(index, raster.width, raster.height)
                    .ok_or_else(|| error(ErrorKind::Internal))?;
                let (w, h) = (w as usize * spec.role().components(), h as usize);
                planes.push(match spec.encoding().channel_type() {
                    ChannelType::U8 => OwnedPlane::U8(new_plane(w, h)?),
                    ChannelType::U16 => OwnedPlane::U16(new_plane(w, h)?),
                    ChannelType::F32 => OwnedPlane::F32(new_plane(w, h)?),
                    _ => return Err(error_msg(ErrorKind::Internal, "unknown plane sample type")),
                });
            }
            building.pixels = Some(Pixels::Planar(PlanarPixels {
                planes,
                layout: layout.clone(),
            }));
        }
        let Some(Pixels::Planar(owned)) = building.pixels.as_mut() else {
            return Err(error_msg(
                ErrorKind::Protocol,
                "planar rows after packed rows",
            ));
        };
        for (index, (src, dst)) in strip.planes().zip(&mut owned.planes).enumerate() {
            // Rows of this plane that precede the strip.
            let at = layout
                .plane_size(index, raster.width, y)
                .ok_or_else(|| error(ErrorKind::Internal))?
                .1 as usize;
            match (src, dst) {
                (AnyPlane::U8(s), OwnedPlane::U8(d)) => copy_plane(&s, d, at)?,
                (AnyPlane::U16(s), OwnedPlane::U16(d)) => copy_plane(&s, d, at)?,
                (AnyPlane::F32(s), OwnedPlane::F32(d)) => copy_plane(&s, d, at)?,
                _ => return Err(error_msg(ErrorKind::Protocol, "plane sample type changed")),
            }
        }
        building.covered += strip.height();
        Ok(())
    }

    fn end(&mut self) -> Result<()> {
        let building = self
            .current
            .take()
            .ok_or_else(|| error_msg(ErrorKind::Protocol, "frame ended outside a frame"))?;
        if building.covered != building.header.raster.height {
            return Err(error_msg(
                ErrorKind::Protocol,
                "frame ended with rows missing",
            ));
        }
        let pixels = match building.pixels {
            Some(pixels) => pixels,
            // A frame with no rows still has a shape.
            None => match building.header.raster.descriptor() {
                Some(descriptor) => Pixels::Packed(
                    PixelBuffer::try_new(
                        building.header.raster.width,
                        building.header.raster.height,
                        descriptor,
                    )
                    .map_err(|_| error(ErrorKind::OutOfMemory))?,
                ),
                None => return Err(error_msg(ErrorKind::Protocol, "planar frame with no rows")),
            },
        };
        self.frames.push(Frame {
            header: building.header,
            pixels,
        });
        Ok(())
    }
}
