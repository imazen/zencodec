//! Storage contracts shared by image and video outputs.
//!
//! Exported through `crate::frame`. Mapping borrows the backend owner; neither
//! this module nor its owners depend on media sessions or I/O drivers.

use crate::frame::{FrameInterpretation, FrameTiming};
use alloc::{boxed::Box, sync::Arc};
use core::fmt;
use zenpixels::plane::AnyPlane;
use zenpixels::sample::SampleEncoding;

/// Facts about one presentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Presentation {
    /// A newly coded, visible key picture (rav1d-safe `is_keyframe`). Not a
    /// random-access promise; container random-access evidence remains separate.
    pub keyframe: bool,
    /// The picture belongs to an earlier presentation (AV1 show-existing).
    pub show_existing: bool,
}

impl Presentation {
    /// A new key picture.
    pub const KEYFRAME: Self = Self {
        keyframe: true,
        show_existing: false,
    };

    /// A replay of an earlier picture.
    pub const SHOW_EXISTING: Self = Self {
        keyframe: false,
        show_existing: true,
    };
}

pub use zenpixels::plane::PlaneError;

/// A picture's planes, mapped. Views borrow the mapping, which borrows the
/// picture, so storage stays put while it is read (rav1d-safe: "a mapped
/// plane owns a borrow guard").
pub trait MappedPlanes {
    /// Number of planes.
    fn plane_count(&self) -> usize;

    /// Plane `index`.
    fn plane(&self, index: usize) -> Option<AnyPlane<'_>>;

    /// The sample encoding plane `index` was written with, when the owner
    /// records one. Consumers compare it with the frame interpretation.
    fn encoding(&self, index: usize) -> Option<SampleEncoding> {
        let _ = index;
        None
    }
}

/// A picture could not be mapped.
#[derive(Debug)]
#[non_exhaustive]
pub enum MapError {
    /// The storage is not readable on the CPU.
    Unavailable,
    /// The backend refused. Preserve its concrete error and any location trace
    /// instead of formatting them into a message.
    Backend(Box<dyn core::error::Error + Send + Sync + 'static>),
}

impl fmt::Display for MapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => f.write_str("picture cannot be mapped"),
            Self::Backend(s) => write!(f, "backend mapping failed: {s}"),
        }
    }
}

impl core::error::Error for MapError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Unavailable => None,
            Self::Backend(error) => Some(&**error),
        }
    }
}

/// Backend-owned pixel storage: rav1d's refcounted picture, an owned set of
/// planes, a pool slot.
pub trait Picture: Send + Sync {
    /// Map the planes for reading.
    ///
    /// # Errors
    ///
    /// A [`MapError`] when the storage cannot be read on the CPU.
    fn map(&self) -> Result<Box<dyn MappedPlanes + '_>, MapError>;
}

/// One presentation of a decoded picture. Cheap to clone; outlives the
/// decoder; may share its picture with another frame.
#[derive(Clone)]
pub struct Frame {
    picture: Arc<dyn Picture>,
    interpretation: Arc<FrameInterpretation>,
    timing: FrameTiming,
    presentation: Presentation,
}

impl Frame {
    /// A frame over `picture`.
    pub fn new(
        picture: Arc<dyn Picture>,
        interpretation: Arc<FrameInterpretation>,
        timing: FrameTiming,
        presentation: Presentation,
    ) -> Self {
        Self {
            picture,
            interpretation,
            timing,
            presentation,
        }
    }

    /// The same picture and interpretation presented again with other
    /// timing (AV1 show-existing, VP8 altref replay).
    pub fn represent(&self, timing: FrameTiming, presentation: Presentation) -> Self {
        Self {
            picture: Arc::clone(&self.picture),
            interpretation: Arc::clone(&self.interpretation),
            timing,
            presentation,
        }
    }

    /// How to read the samples.
    pub fn interpretation(&self) -> &Arc<FrameInterpretation> {
        &self.interpretation
    }

    /// This presentation's timing.
    pub fn timing(&self) -> FrameTiming {
        self.timing
    }

    /// Facts about this presentation.
    pub fn presentation(&self) -> Presentation {
        self.presentation
    }

    /// Map the planes for reading.
    ///
    /// # Errors
    ///
    /// A [`MapError`] when the storage cannot be read on the CPU.
    pub fn map(&self) -> Result<Box<dyn MappedPlanes + '_>, MapError> {
        self.picture.map()
    }

    /// Whether both frames present the same picture storage.
    pub fn shares_picture(&self, other: &Frame) -> bool {
        Arc::ptr_eq(&self.picture, &other.picture)
    }

    /// The backing picture.
    pub fn picture(&self) -> &Arc<dyn Picture> {
        &self.picture
    }
}

impl fmt::Debug for Frame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Frame")
            .field("timing", &self.timing)
            .field("presentation", &self.presentation)
            .field("epoch", &self.interpretation.config_epoch)
            .finish_non_exhaustive()
    }
}
