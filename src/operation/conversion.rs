//! Policy before executing a retained image conversion route.
use super::{
    DecisionContext, DecisionRecord, ImageFacts, OperationPolicy, PolicyFailure, PolicySet,
    Question, Selection, policy,
};
use crate::{
    At,
    frame::ChromaSiting,
    image::{ImageGeometry, ImageOutput},
};
use zenpixels::{Orientation, PixelDescriptor};

/// Effects of one prepared conversion of the exact held image.
///
/// The adapter retains its input, conversion mechanism, and destination/scratch reservations
/// while policy runs. This is neither a capability advertisement nor permission
/// to rebuild a different plan after selection. Execution can still fail.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct ImageConversionOffer {
    /// Actual output pixel representation and compact color interpretation.
    /// Profiles/context must also satisfy the caller's configured target.
    pub output: PixelDescriptor,
    /// Stored output extent, before residual orientation.
    pub geometry: ImageGeometry,
    /// Residual orientation after this conversion; no inferred rotation.
    pub orientation: Orientation,
    /// A concrete chroma location assumed because the source did not declare
    /// one. None means no such assumption; an existing declaration is never
    /// overridden. Unknown is not a realizable assumed location.
    pub assumed_chroma_siting: Option<ChromaSiting>,
    /// True only when the route guarantees retention of extended-range values.
    /// False includes clipping, integer saturation and an unqualified guarantee;
    /// it is not a lossless-precision or HDR-rendering certificate.
    pub preserves_extended_range: bool,
    /// Managed destination and scratch capacity already reserved for this route.
    /// It need not be allocated or initialized until the callback accepts.
    /// Alternatives may share storage. Source/native workspace is excluded.
    pub admitted_bytes: usize,
    /// Qualified additional conversion workspace bound, if known. Unknown is
    /// not zero; this is not a whole-operation memory bound.
    pub workspace_upper_bound: Option<u64>,
}
impl ImageConversionOffer {
    /// Describe the target; assumptions must be explicit and resource/extended
    /// range guarantees start unqualified until supplied by the prepared adapter.
    pub const fn new(
        output: PixelDescriptor,
        geometry: ImageGeometry,
        orientation: Orientation,
    ) -> Self {
        Self {
            output,
            geometry,
            orientation,
            assumed_chroma_siting: None,
            preserves_extended_range: false,
            admitted_bytes: 0,
            workspace_upper_bound: None,
        }
    }
}

/// Choose a retained conversion before writing its destination pixels.
///
/// Adapter API: offers must satisfy the request, refer to this input and name
/// actual retained plans. Execute only the selected plan without fallback;
/// preserve this record on execution failure. This function verifies policy
/// tokens/cancellation, not conversion math or adapter factual claims. It does
/// not certify metadata/privacy treatment or publish an intermediate image.
pub fn resolve_image_conversion<R: OperationPolicy + ?Sized>(
    image: &ImageOutput,
    offers: &[ImageConversionOffer],
    resolver: &mut R,
    preset: PolicySet,
    pulse: Option<&dyn crate::Pulse>,
) -> Result<(usize, DecisionRecord), At<PolicyFailure>> {
    let pulse = pulse.unwrap_or(&crate::NoPulse);
    let facts = ImageFacts::new(image);
    policy::choose(
        offers,
        DecisionContext::ImageConversion(facts),
        |choices| Question::ImageConversion {
            image,
            facts: &facts,
            choices,
        },
        Selection::ImageConversion,
        resolver,
        preset,
        pulse,
    )
}
