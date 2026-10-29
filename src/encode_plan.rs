//! Preflight facts for the exact configured encoder job.

use alloc::sync::Arc;
use core::fmt;
use zenpixels::{
    ColorContext, PixelDescriptor, PixelSlice,
    sample::{ExactDepth, ExactDepthError},
};

use crate::{Cicp, encode::OutputTiming, output_patch::OutputAccess};

/// Input interpretation and destination access, declared before native startup.
///
/// `color` describes the samples actually supplied. Descriptive job metadata
/// must not override it. A codec must refuse contradictions it cannot resolve
/// without changing the requested interpretation or encoding configuration.
#[derive(Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct EncodeDeclaration<'a> {
    /// Full image width.
    pub width: u32,
    /// Full image height.
    pub height: u32,
    /// Descriptor of the samples supplied to the encoder.
    pub descriptor: PixelDescriptor,
    /// Authoritative current color context, when supplied.
    pub color: Option<&'a ColorContext>,
    /// Actual destination access; it does not authorize a quality change.
    pub access: OutputAccess,
    /// Optional assertion about the current canonical sample grids.
    /// `Some` requires verification; unsupported layouts refuse. The paired
    /// verifier supports U8/U16 gray, gray-alpha, RGB and RGBA/BGRA,
    /// including undefined padding. `None` makes no lower-depth assertion.
    /// This is not a codec fidelity or whole-input preflight certificate.
    pub exact_depth: Option<ExactDepth>,
}

impl fmt::Debug for EncodeDeclaration<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EncodeDeclaration")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("descriptor", &self.descriptor)
            .field("has_color_context", &self.color.is_some())
            .field("access", &self.access)
            .field("exact_depth", &self.exact_depth)
            .finish()
    }
}

impl<'a> EncodeDeclaration<'a> {
    /// Declare a forward destination and descriptor-only interpretation.
    pub const fn new(width: u32, height: u32, descriptor: PixelDescriptor) -> Self {
        Self {
            width,
            height,
            descriptor,
            color: None,
            access: OutputAccess::Forward,
            exact_depth: None,
        }
    }

    /// Bind the current pixel interpretation before any output header is made.
    pub const fn with_color(mut self, color: &'a ColorContext) -> Self {
        self.color = Some(color);
        self
    }

    /// Bind the destination's actual access mode.
    pub const fn with_access(mut self, access: OutputAccess) -> Self {
        self.access = access;
        self
    }

    /// Retain a sample-grid assertion without scanning or changing samples.
    /// Consumers must check actual samples before relying on this assertion.
    /// Planning can refuse an assertion whose layout the verifier cannot check,
    /// even when the codec could encode the same samples without that assertion.
    pub const fn with_exact_depth(mut self, exact: ExactDepth) -> Self {
        self.exact_depth = Some(exact);
        self
    }

    /// Validate the claim's storage, range and supported layout, without reading
    /// source samples. Success does not establish that actual samples comply.
    pub fn validate_exact_depth(&self) -> Result<(), ExactDepthError> {
        if let Some(exact) = self.exact_depth {
            // The paired verifier owns descriptor/layout rules. An empty view
            // checks them without allocating a raster or adding a source pass.
            let empty = PixelSlice::new(&[], 0, 0, 0, self.descriptor)
                .map_err(|_| ExactDepthError::DescriptorMismatch)?;
            exact.holds_for(&empty)?;
        }
        Ok(())
    }
}

/// Source passes declared before the first source read.
///
/// A finite admission budget requires a known maximum. A lower bound alone
/// cannot justify beginning a route that might exceed that budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourcePasses {
    min: u32,
    max: Option<u32>,
}

impl SourcePasses {
    /// Exactly one traversal.
    pub const ONCE: Self = Self {
        min: 1,
        max: Some(1),
    };

    /// Declare a positive minimum and an optional maximum no smaller than it.
    pub const fn new(min: u32, max: Option<u32>) -> Result<Self, EncodePlanError> {
        if min == 0 || matches!(max, Some(max) if max < min) {
            return Err(EncodePlanError::PassRange);
        }
        Ok(Self { min, max })
    }

    /// Minimum number of complete source traversals.
    pub const fn min(self) -> u32 {
        self.min
    }
    /// Maximum number of complete source traversals, if known before reading.
    pub const fn max(self) -> Option<u32> {
        self.max
    }
    /// Whether this complete range satisfies a finite pass budget.
    pub const fn fits(self, budget: u32) -> bool {
        matches!(self.max, Some(max) if max <= budget)
    }
}

/// How an encoder acquires input, independently of when encoded bytes appear.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum InputRequirement {
    /// One ordered row traversal through `push_rows`. This may still buffer
    /// the whole image internally; it is not a retained-memory bound.
    Rows,
    /// A complete raster through `encode`. A row driver cannot execute it.
    WholeRaster,
    /// Encoder-driven repeated traversal. Requires a replay-capable driver;
    /// ordinary `push_rows` does not implement this protocol.
    Replay(SourcePasses),
}

/// The pass in which output may first become available.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum OutputPhase {
    /// Bytes may appear in any pass; timing counts rows within an emitting pass.
    AnyPass,
    /// No bytes before the final source pass. Timing counts rows within that
    /// pass, not rows accumulated across analysis passes.
    FinalPass,
}

/// Materialized color carriers the encoder commits to emitting.
///
/// Unlike a color-policy intent, ICC bytes here are already available. This
/// does not certify that a profile is free of private data or qualify other
/// metadata channels. `EncodePlan::carriers == None` means unknown, not absent.
#[derive(Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct ColorCarriers {
    /// Exact ICC payload, excluding container framing; `None` means none emitted.
    pub icc: Option<Arc<[u8]>>,
    /// Native CICP carrier, if emitted.
    pub cicp: Option<Cicp>,
}

impl fmt::Debug for ColorCarriers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ColorCarriers")
            .field("icc_bytes", &self.icc.as_ref().map(|icc| icc.len()))
            .field("cicp", &self.cicp)
            .finish()
    }
}

impl ColorCarriers {
    /// Declare exact materialized carriers, including confirmed absence.
    pub fn new(icc: Option<Arc<[u8]>>, cicp: Option<Cicp>) -> Self {
        Self { icc, cicp }
    }
}

/// Spatial effects of this retained job, separate from compression fidelity.
///
/// Applying `applied` to the supplied raster, then interpreting its output
/// orientation, must preserve the input's displayed orientation. Dimensions
/// describe visible stored samples, not codec block padding or a video aperture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct OrientationPlan {
    /// Intrinsic orientation of the supplied samples, from effective metadata.
    pub input: crate::Orientation,
    /// Spatial transform the encoder applies to the samples themselves.
    pub applied: crate::Orientation,
    /// Intrinsic orientation carried by the encoded output.
    pub output: crate::Orientation,
}
impl OrientationPlan {
    /// Keep sample order and carry the intrinsic orientation.
    pub const fn preserved(input: crate::Orientation) -> Self {
        Self {
            input,
            applied: crate::Orientation::Identity,
            output: input,
        }
    }
    /// Transform samples to their displayed orientation and carry identity.
    pub const fn baked(input: crate::Orientation) -> Self {
        Self {
            input,
            applied: input,
            output: crate::Orientation::Identity,
        }
    }
    /// Visible stored geometry after the reported spatial transform.
    pub const fn stored_dimensions(self, width: u32, height: u32) -> (u32, u32) {
        self.applied.output_dimensions(width, height)
    }
    /// Refuse contradictory transforms; this does not inspect native output.
    pub fn validate(self) -> Result<(), EncodePlanError> {
        if self.applied.then(self.output) != self.input {
            return Err(EncodePlanError::Orientation);
        }
        Ok(())
    }
}

/// Facts from the retained effective job, before native pixel/workspace startup.
///
/// Planning may validate and materialize bounded metadata. It must not read
/// source pixels, publish bytes, start workers or allocate native image state.
/// These facts do not imply a workspace or output-wait memory bound.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct EncodePlan {
    /// Executable input protocol.
    pub input: InputRequirement,
    /// Byte availability, independent of input retention.
    pub timing: OutputTiming,
    /// Which source pass can begin producing bytes.
    pub output_phase: OutputPhase,
    /// Exact output color carriers, or unreported evidence.
    pub carriers: Option<ColorCarriers>,
    /// Qualified spatial effects; None is unknown, never implicit preservation.
    pub orientation: Option<OrientationPlan>,
}

impl EncodePlan {
    /// A single ordered row pass, with color-carrier evidence initially unknown.
    pub const fn rows(timing: OutputTiming) -> Self {
        Self {
            input: InputRequirement::Rows,
            timing,
            output_phase: OutputPhase::FinalPass,
            carriers: None,
            orientation: None,
        }
    }

    /// A complete-raster input with output returned at finish.
    pub const fn whole_raster() -> Self {
        Self {
            input: InputRequirement::WholeRaster,
            timing: OutputTiming::AT_FINISH,
            output_phase: OutputPhase::FinalPass,
            carriers: None,
            orientation: None,
        }
    }

    /// Repeated encoder-driven source passes. This declares facts; it does not
    /// make the existing row-push protocol capable of replay.
    pub const fn replay(
        passes: SourcePasses,
        timing: OutputTiming,
        output_phase: OutputPhase,
    ) -> Self {
        Self {
            input: InputRequirement::Replay(passes),
            timing,
            output_phase,
            carriers: None,
            orientation: None,
        }
    }

    /// Bind exact color carriers. No privacy conclusion follows from this alone.
    pub fn with_carriers(mut self, carriers: ColorCarriers) -> Self {
        self.carriers = Some(carriers);
        self
    }

    /// Bind spatial effects from the same retained job. No memory bound follows.
    pub const fn with_orientation(mut self, orientation: OrientationPlan) -> Self {
        self.orientation = Some(orientation);
        self
    }

    /// Check geometry, output lag and patch access before native startup.
    /// Codec-specific validation remains the planning implementation's duty.
    pub fn validate(&self, declaration: &EncodeDeclaration<'_>) -> Result<(), EncodePlanError> {
        if let Some(orientation) = self.orientation {
            orientation.validate()?;
        }
        declaration
            .validate_exact_depth()
            .map_err(|_| EncodePlanError::ExactDepth)?;
        if declaration.width == 0 || declaration.height == 0 {
            return Err(EncodePlanError::Dimensions);
        }
        if self
            .timing
            .lag_rows()
            .is_some_and(|lag| lag >= declaration.height)
        {
            return Err(EncodePlanError::OutputLag);
        }
        if self.timing.patches_at_finish() && declaration.access != OutputAccess::SeekBack {
            return Err(EncodePlanError::DestinationAccess);
        }
        if self.input == InputRequirement::WholeRaster && self.timing.streams() {
            return Err(EncodePlanError::InputProtocol);
        }
        Ok(())
    }

    /// Verify startup reproduced every admitted fact before any source read or
    /// publication. Callers retain the same job and declaration between calls.
    pub fn validate_started(&self, observed: &Self) -> Result<(), EncodePlanError> {
        if self != observed {
            return Err(EncodePlanError::Changed);
        }
        Ok(())
    }
}

/// A contradictory or changed encode plan. Backend errors keep their own type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EncodePlanError {
    /// Zero dimensions.
    Dimensions,
    /// A nonpositive or reversed pass range.
    PassRange,
    /// Streaming lag cannot hold the entire image.
    OutputLag,
    /// Required patches cannot be applied to this destination.
    DestinationAccess,
    /// The reported input and output protocols disagree.
    InputProtocol,
    /// Native startup differs from the admitted plan.
    Changed,
    /// Exact-depth assertion contradicts the descriptor or uses an unsupported layout.
    /// [`EncodeDeclaration::validate_exact_depth`] retains the detailed verifier error.
    ExactDepth,
    /// Applied and carried orientations contradict the input interpretation.
    Orientation,
}

impl fmt::Display for EncodePlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Orientation => "encode spatial effects change the displayed orientation",
            Self::Dimensions => "encode declaration has zero dimensions",
            Self::PassRange => "encode source-pass range is invalid",
            Self::OutputLag => "encode output lag holds the entire image",
            Self::DestinationAccess => "encode patches require seek-back output",
            Self::InputProtocol => "encode input and output protocols disagree",
            Self::Changed => "encode startup changed the admitted plan",
            Self::ExactDepth => "encode exact-depth claim is incompatible with its descriptor",
        })
    }
}
impl core::error::Error for EncodePlanError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_diagnostics_do_not_dump_opaque_profiles() {
        let context = ColorContext::from_icc(alloc::vec![88; 2048]);
        let declaration =
            EncodeDeclaration::new(3, 4, PixelDescriptor::RGB8_SRGB).with_color(&context);
        let carriers = ColorCarriers::new(Some(Arc::from([88; 2048])), None);
        let rendered = alloc::format!("{declaration:?} {carriers:?}");
        assert!(rendered.len() < 1024);
        assert!(!rendered.contains("88, 88"));
        assert!(rendered.contains("2048"));
    }

    #[test]
    fn finite_budget_requires_the_maximum_not_the_minimum() {
        assert!(SourcePasses::new(0, Some(1)).is_err());
        assert!(SourcePasses::new(5, Some(2)).is_err());
        let bounded = SourcePasses::new(2, Some(5)).unwrap();
        assert!(!bounded.fits(2));
        assert!(bounded.fits(5));
        assert!(!SourcePasses::new(2, None).unwrap().fits(u32::MAX));
        assert!(SourcePasses::ONCE.fits(1));
        assert!(!SourcePasses::ONCE.fits(0));
    }

    #[test]
    fn declaration_refuses_impossible_lag_and_patch_access() {
        let declaration = EncodeDeclaration::new(7, 9, PixelDescriptor::RGB8_SRGB);
        let plan = EncodePlan::rows(OutputTiming::streaming(9));
        assert_eq!(plan.validate(&declaration), Err(EncodePlanError::OutputLag));
        let plan = EncodePlan::rows(OutputTiming::AT_FINISH.with_patches());
        assert_eq!(
            plan.validate(&declaration),
            Err(EncodePlanError::DestinationAccess)
        );
        assert!(
            plan.validate(&declaration.with_access(OutputAccess::SeekBack))
                .is_ok()
        );
        let mut raster = EncodePlan::whole_raster();
        raster.timing = OutputTiming::streaming(0);
        assert_eq!(
            raster.validate(&declaration),
            Err(EncodePlanError::InputProtocol)
        );
    }

    #[test]
    fn startup_comparison_includes_input_phase_and_exact_carriers() {
        let plan = EncodePlan::rows(OutputTiming::AT_FINISH).with_carriers(ColorCarriers::new(
            Some(Arc::from([1, 2, 3])),
            Some(Cicp::DISPLAY_P3),
        ));
        assert!(plan.validate_started(&plan.clone()).is_ok());
        let mut changed = plan.clone();
        changed.carriers.as_mut().unwrap().icc = Some(Arc::from([1, 2, 4]));
        assert_eq!(
            plan.validate_started(&changed),
            Err(EncodePlanError::Changed)
        );
        let mut changed = plan.clone();
        changed.input = InputRequirement::WholeRaster;
        assert_eq!(
            plan.validate_started(&changed),
            Err(EncodePlanError::Changed)
        );
        let mut changed = plan.clone();
        changed.output_phase = OutputPhase::AnyPass;
        assert_eq!(
            plan.validate_started(&changed),
            Err(EncodePlanError::Changed)
        );
        let mut changed = plan.clone();
        changed.carriers = None;
        assert_eq!(
            plan.validate_started(&changed),
            Err(EncodePlanError::Changed)
        );
        assert_ne!(
            EncodePlan::whole_raster().carriers,
            Some(ColorCarriers::default())
        );
    }
}

#[cfg(test)]
mod orientation_tests {
    use super::*;
    use crate::Orientation;
    const ALL: [Orientation; 8] = [
        Orientation::Identity,
        Orientation::FlipH,
        Orientation::Rotate180,
        Orientation::FlipV,
        Orientation::Transpose,
        Orientation::Rotate90,
        Orientation::Transverse,
        Orientation::Rotate270,
    ];

    #[test]
    fn orientation_plan_matches_coordinate_composition() {
        for input in ALL {
            for applied in ALL {
                for output in ALL {
                    let plan = OrientationPlan {
                        input,
                        applied,
                        output,
                    };
                    let (w, h) = plan.stored_dimensions(3, 2);
                    let mut same = output.output_dimensions(w, h) == input.output_dimensions(3, 2);
                    for y in 0..2 {
                        for x in 0..3 {
                            let (sx, sy) = applied.forward_map(x, y, 3, 2);
                            same &=
                                output.forward_map(sx, sy, w, h) == input.forward_map(x, y, 3, 2);
                        }
                    }
                    assert_eq!(plan.validate().is_ok(), same, "{plan:?}");
                }
            }
        }
    }

    #[test]
    fn spatial_evidence_is_unknown_by_default_and_stable_through_startup() {
        let declaration = EncodeDeclaration::new(3, 2, PixelDescriptor::RGB8_SRGB);
        let empty = EncodePlan::whole_raster();
        assert_eq!(empty.orientation, None);
        for input in ALL {
            let preserve = empty
                .clone()
                .with_orientation(OrientationPlan::preserved(input));
            let bake = empty
                .clone()
                .with_orientation(OrientationPlan::baked(input));
            assert_eq!(
                preserve.orientation.unwrap().stored_dimensions(3, 2),
                (3, 2)
            );
            assert_eq!(
                bake.orientation.unwrap().stored_dimensions(3, 2),
                input.output_dimensions(3, 2)
            );
            preserve.validate(&declaration).unwrap();
            bake.validate(&declaration).unwrap();
            assert_eq!(
                preserve.validate_started(&empty),
                Err(EncodePlanError::Changed)
            );
            if input != Orientation::Identity {
                assert_eq!(
                    preserve.validate_started(&bake),
                    Err(EncodePlanError::Changed)
                );
            }
        }
    }
}
