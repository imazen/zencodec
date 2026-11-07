//! Private unified owner experiment. No production API or policy admission.
use crate::{component_grid::Grid, component_storage::Plane};
use std::{boxed::Box, cell::Cell, error::Error as StdError, fmt, sync::Arc};
use zencodec::{
    frame::{FrameTiming, Presentation},
    image::Meaning,
};
use zenpixels::{ChannelType, ColorContext, PixelDescriptor, PixelSlice, sample::SampleEncoding};

type Cause = Box<dyn StdError + Send + Sync>;
pub(crate) enum Error {
    Backend(Cause),
    Count,
    Missing(usize),
    Changed(usize),
    Terminal,
    Stopped(enough::StopReason),
}
impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Backend(_) => f.write_str("native image mapping failed"),
            Self::Count => f.write_str("mapped plane count changed"),
            Self::Missing(i) => write!(f, "declared plane {i} is missing"),
            Self::Changed(i) => write!(f, "plane {i} interpretation changed"),
            Self::Terminal => f.write_str("image mapping is terminal"),
            Self::Stopped(_) => f.write_str("image mapping stopped"),
        }
    }
}
impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Backend(e) => Some(&**e),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Representation {
    Packed {
        descriptor: PixelDescriptor,
        color: Option<Arc<ColorContext>>,
    },
    Component {
        channel: ChannelType,
        encoding: Option<SampleEncoding>,
        grid: Grid,
    },
}
#[derive(Clone, Debug)]
pub(crate) struct Spec {
    pub meaning: Meaning,
    pub dimensions: (usize, usize),
    pub representation: Representation,
}

pub(crate) enum Samples<'a> {
    Packed(PixelSlice<'a>),
    Component {
        plane: Plane<'a>,
        encoding: Option<SampleEncoding>,
        grid: Grid,
    },
}
impl Samples<'_> {
    fn agrees(&self, expected: &Spec) -> bool {
        match (self, &expected.representation) {
            (Self::Packed(p), Representation::Packed { descriptor, color }) => {
                (p.width() as usize, p.rows() as usize) == expected.dimensions
                    && p.descriptor() == *descriptor
                    && match (p.color_context(), color.as_ref()) {
                        (None, None) => true,
                        (Some(a), Some(b)) => Arc::ptr_eq(a, b) || a == b,
                        _ => false,
                    }
            }
            (
                Self::Component {
                    plane,
                    encoding,
                    grid,
                },
                Representation::Component {
                    channel,
                    encoding: want,
                    grid: registration,
                },
            ) => {
                plane.dimensions() == expected.dimensions
                    && plane.channel() == *channel
                    && encoding == want
                    && grid == registration
                    && encoding.is_none_or(|e| e.storage() == plane.channel())
            }
            _ => false,
        }
    }
}
/// Guard lifetime is tied to owner. A returned plane borrows this guard.
pub(crate) trait Mapping {
    fn plane_count(&self) -> usize;
    fn plane(
        &self,
        index: usize,
        pulse: Option<&dyn zencodec::Pulse>,
    ) -> Result<Option<Samples<'_>>, Cause>;
}
pub(crate) trait Owner: Send + Sync {
    fn map(&self, pulse: Option<&dyn zencodec::Pulse>) -> Result<Box<dyn Mapping + '_>, Cause>;
}
struct Held {
    owner: Box<dyn Owner>,
    specs: Box<[Spec]>,
}
#[derive(Clone)]
pub(crate) struct Image(Arc<Held>);
impl Image {
    // Construction does not map or claim to admit prior native allocations.
    pub(crate) fn new(owner: Box<dyn Owner>, specs: Box<[Spec]>) -> Self {
        Self(Arc::new(Held { owner, specs }))
    }
    pub(crate) fn map<'a>(
        &'a self,
        pulse: Option<&'a dyn zencodec::Pulse>,
    ) -> Result<CheckedMap<'a>, Error> {
        check(pulse)?;
        let native = self.0.owner.map(pulse).map_err(Error::Backend)?;
        check(pulse)?;
        if native.plane_count() != self.0.specs.len() {
            return Err(Error::Count);
        }
        Ok(CheckedMap {
            native,
            specs: &self.0.specs,
            failed: Cell::new(false),
            pulse,
        })
    }
    pub(crate) fn planes(&self) -> &[Spec] {
        &self.0.specs
    }
    pub(crate) fn same_storage(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
pub(crate) struct CheckedPlane<'a> {
    pub meaning: &'a Meaning,
    pub samples: Samples<'a>,
}
pub(crate) struct CheckedMap<'a> {
    native: Box<dyn Mapping + 'a>,
    specs: &'a [Spec],
    failed: Cell<bool>,
    pulse: Option<&'a dyn zencodec::Pulse>,
}
impl CheckedMap<'_> {
    pub(crate) fn plane_count(&self) -> usize {
        self.specs.len()
    }
    pub(crate) fn plane(&self, index: usize) -> Result<Option<CheckedPlane<'_>>, Error> {
        if self.failed.get() {
            return Err(Error::Terminal);
        }
        let Some(expected) = self.specs.get(index) else {
            return Ok(None);
        };
        self.failed.set(true);
        let result = (|| {
            check(self.pulse)?;
            let samples = self
                .native
                .plane(index, self.pulse)
                .map_err(Error::Backend)?
                .ok_or(Error::Missing(index))?;
            check(self.pulse)?;
            if !samples.agrees(expected) {
                return Err(Error::Changed(index));
            }
            Ok(Some(CheckedPlane {
                meaning: &expected.meaning,
                samples,
            }))
        })();
        if result.is_ok() {
            self.failed.set(false);
        }
        result
    }
}
fn check(pulse: Option<&dyn zencodec::Pulse>) -> Result<(), Error> {
    pulse.map_or(Ok(()), |pulse| pulse.check().map_err(Error::Stopped))
}
/// Presentation adds timing/flags to an existing image; no second mapping API.
pub(crate) struct Frame {
    pub image: Image,
    pub timing: FrameTiming,
    pub flags: Presentation,
}
impl Frame {
    pub(crate) fn represent(&self, timing: FrameTiming, flags: Presentation) -> Self {
        Self {
            image: self.image.clone(),
            timing,
            flags,
        }
    }
}
