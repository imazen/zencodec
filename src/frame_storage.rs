//! Storage contracts shared by image and video outputs.
//!
//! Exported through `crate::frame`. Mapping borrows the backend owner; neither
//! this module nor its owners depend on media sessions or I/O drivers.

use crate::frame::{FrameInterpretation, FrameTiming};
use alloc::{boxed::Box, sync::Arc};
use core::fmt;
use zenpixels::ColorContext;
use zenpixels::plane::AnyPlane;
use zenpixels::plane::{LayoutError, PlanarSlice, PlaneEncoding};

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

    /// Encoding actually written to plane `index`. Required for every present plane;
    /// `None` is valid only past the end. Checked frame mapping compares this with
    /// the immutable interpretation before exposing samples.
    fn encoding(&self, index: usize) -> Option<PlaneEncoding>;
}

/// A picture could not be mapped.
#[non_exhaustive]
pub enum MapError {
    /// The storage is not readable on the CPU.
    Unavailable,
    /// The coded crop/render declaration is contradictory.
    InvalidGeometry,
    /// Storage shape, role, type or alpha interpretation disagrees with the layout.
    Layout(LayoutError),
    /// A present owner omitted or contradicted the declared sample encoding.
    Component {
        /// Index in the retained layout.
        index: usize,
        /// Active sample-word dimensions required by the interpretation.
        expected_dimensions: (usize, usize),
        /// Active sample-word dimensions returned by the owner.
        actual_dimensions: (usize, usize),
        /// Encoding required by the interpretation.
        expected_encoding: PlaneEncoding,
        /// Encoding recorded by the owner; absence is not evidence.
        actual_encoding: Option<PlaneEncoding>,
        /// Actual storage type, independent of the owner's encoding claim.
        actual_storage: zenpixels::ChannelType,
    },
    /// Borrowed-plane handle storage could not be reserved.
    AllocationFailed,
    /// The backend refused. Preserve its concrete error and any location trace
    /// instead of formatting them into a message.
    Backend(Box<dyn core::error::Error + Send + Sync + 'static>),
}

impl fmt::Debug for MapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Backend(_) => f.debug_tuple("Backend").field(&"cause retained").finish(),
            Self::Component {
                index,
                expected_dimensions,
                actual_dimensions,
                expected_encoding,
                actual_encoding,
                actual_storage,
            } => f
                .debug_struct("Component")
                .field("index", index)
                .field("expected_dimensions", expected_dimensions)
                .field("actual_dimensions", actual_dimensions)
                .field("expected_encoding", expected_encoding)
                .field("actual_encoding", actual_encoding)
                .field("actual_storage", actual_storage)
                .finish(),
            Self::Layout(error) => f.debug_tuple("Layout").field(error).finish(),
            _ => fmt::Display::fmt(self, f),
        }
    }
}

impl fmt::Display for MapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => f.write_str("picture cannot be mapped"),
            Self::Backend(_) => f.write_str("backend mapping failed"),
            Self::InvalidGeometry => f.write_str("invalid native frame geometry"),
            Self::Layout(error) => error.fmt(f),
            Self::Component { index, .. } => write!(
                f,
                "native plane {index} encoding disagrees with interpretation"
            ),
            Self::AllocationFailed => f.write_str("could not reserve borrowed plane handles"),
        }
    }
}

impl core::error::Error for MapError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Backend(error) => Some(&**error),
            Self::Layout(error) => Some(error),
            _ => None,
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
        self.map_controlled(None)
    }

    /// Lend a checked CPU planar view while retaining its native mapping owner.
    /// No samples are copied. The callback cannot return a view that outlives the loan.
    /// Missing current color remains unknown. Pulse checks surround native mapping;
    /// work inside the callback controls its own cancellation/error precedence.
    ///
    /// # Errors
    /// Original native errors, cancellation, or contradictory storage/interpretation.
    ///
    /// The mapping loan cannot escape:
    /// ```compile_fail
    /// use zencodec::frame::Frame;
    /// use zenpixels::plane::PlanarSlice;
    /// fn escape(frame: &Frame) -> PlanarSlice<'_> {
    ///     frame.with_planar(None, |view| view).unwrap()
    /// }
    /// ```
    pub fn with_planar<T>(
        &self,
        pulse: Option<&dyn crate::Pulse>,
        callback: impl FnOnce(PlanarSlice<'_>) -> T,
    ) -> Result<T, MapError> {
        let mapped = self.map_controlled(pulse)?;
        let mut planes = alloc::vec::Vec::new();
        planes
            .try_reserve_exact(mapped.plane_count())
            .map_err(|_| MapError::AllocationFailed)?;
        for index in 0..mapped.plane_count() {
            planes.push(
                mapped
                    .plane(index)
                    .ok_or(MapError::Layout(LayoutError::PlaneIndex {
                        index,
                        count: mapped.plane_count(),
                    }))?,
            );
        }
        let unknown = ColorContext::default();
        let context = self
            .interpretation
            .color_context
            .as_deref()
            .unwrap_or(&unknown);
        let view = PlanarSlice::new(
            &planes,
            &self.interpretation.layout,
            self.interpretation.geometry.coded_width,
            self.interpretation.geometry.coded_height,
            context,
            self.interpretation.alpha_mode,
        )
        .map_err(MapError::Layout)?;
        Ok(callback(view))
    }

    fn map_controlled(
        &self,
        pulse: Option<&dyn crate::Pulse>,
    ) -> Result<Box<dyn MappedPlanes + '_>, MapError> {
        check(pulse)?;
        let interpretation = &self.interpretation;
        let geometry = interpretation.geometry;
        if geometry
            .crop
            .x
            .checked_add(geometry.crop.width)
            .is_none_or(|end| end > geometry.coded_width)
            || geometry
                .crop
                .y
                .checked_add(geometry.crop.height)
                .is_none_or(|end| end > geometry.coded_height)
            || geometry
                .render
                .is_some_and(|(width, height)| width == 0 || height == 0)
        {
            return Err(MapError::InvalidGeometry);
        }
        let has_alpha = interpretation
            .layout
            .roles()
            .any(|role| role == zenpixels::plane::PlaneRole::Alpha);
        if has_alpha != interpretation.alpha_mode.is_some()
            || interpretation
                .alpha_mode
                .is_some_and(|alpha| !alpha.has_alpha())
        {
            return Err(MapError::Layout(LayoutError::AlphaInterpretation));
        }
        let mapped = self.picture.map()?;
        check(pulse)?;
        if mapped.plane_count() != interpretation.layout.planes().len() {
            return Err(MapError::Layout(LayoutError::PlaneCount {
                expected: interpretation.layout.planes().len(),
                actual: mapped.plane_count(),
            }));
        }
        for (index, spec) in interpretation.layout.planes().iter().enumerate() {
            let plane = mapped
                .plane(index)
                .ok_or(MapError::Layout(LayoutError::PlaneIndex {
                    index,
                    count: mapped.plane_count(),
                }))?;
            let shape = interpretation.layout.check_plane(
                index,
                &plane,
                geometry.coded_width,
                geometry.coded_height,
            );
            let actual_encoding = mapped.encoding(index);
            if shape.is_err() || actual_encoding != Some(spec.encoding()) {
                let (width, height) = interpretation
                    .layout
                    .plane_size(index, geometry.coded_width, geometry.coded_height)
                    .expect("index from validated layout");
                return Err(MapError::Component {
                    index,
                    expected_dimensions: (
                        (width as usize).saturating_mul(spec.role().components()),
                        height as usize,
                    ),
                    actual_dimensions: (plane.width(), plane.height()),
                    expected_encoding: spec.encoding(),
                    actual_encoding,
                    actual_storage: plane.channel_type(),
                });
            }
        }
        Ok(mapped)
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

fn check(pulse: Option<&dyn crate::Pulse>) -> Result<(), MapError> {
    if let Some(pulse) = pulse {
        pulse.check().map_err(|reason| {
            MapError::Backend(Box::new(whereat::at!(crate::CodecError::new(
                None,
                crate::ErrorCategory::Stopped(reason)
            ))))
        })?;
    }
    Ok(())
}
