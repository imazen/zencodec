//! Resolve publication of the exact owned selected-image output.
use super::{
    DecisionContext, FrameFacts, Outcome, PolicyFailure, PolicyResolver, PolicySet,
    PublicationOffer, Question, Selection, policy,
};
use crate::{
    At,
    frame::{FrameTiming, Presentation},
    image::{ImageGeometry, ImageOutput, Placement},
};
use zenpixels::{Orientation, PixelDescriptor};

/// Bounded diagnostic facts, without pixel bytes, names, profiles or source
/// payloads. They describe the held output; they do not establish privacy or a
/// native workspace bound. Full source details remain accessible in the question.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct ImageFacts {
    /// Stored output extent within the selected canvas.
    pub geometry: ImageGeometry,
    /// Separate presentation placement, when established by the adapter.
    pub placement: Option<Placement>,
    /// Residual orientation, if the adapter establishes one.
    pub orientation: Option<Orientation>,
    /// Actual packed pixel descriptor, absent for native component codes.
    pub packed: Option<PixelDescriptor>,
    /// Declared emitted planes. Mapping checks the native storage separately.
    pub planes: usize,
    /// Number of retained auxiliary planes.
    pub auxiliaries: usize,
    /// Spot planes already applied to returned color.
    pub applied_spot_colors: usize,
    /// Number of opaque typed source extensions, not a privacy classification.
    pub source_details: usize,
    /// Exact presentation timing, when established.
    pub timing: Option<FrameTiming>,
    /// Native presentation facts, when established.
    pub presentation: Option<Presentation>,
    /// Native component interpretation, when supplied by a frame adapter.
    pub frame: Option<FrameFacts>,
}
impl ImageFacts {
    /// Inspect retained metadata without mapping, converting or copying samples.
    pub fn new(image: &ImageOutput) -> Self {
        let interpretation = image.interpretation();
        let samples = image.sample_facts();
        Self {
            geometry: interpretation.geometry,
            placement: interpretation.placement,
            orientation: interpretation.orientation,
            packed: samples.packed,
            planes: samples.planes,
            auxiliaries: samples.auxiliaries,
            applied_spot_colors: samples.applied_spot_colors,
            source_details: interpretation.source_details.len(),
            timing: interpretation.timing,
            presentation: interpretation.presentation,
            frame: interpretation
                .signal
                .as_ref()
                .map(|signal| FrameFacts::new(signal, interpretation.timing.unwrap_or_default())),
        }
    }
}

/// Apply the operation's existing resolver before returning a selected output.
///
/// Consumes the owner so refusal, cancellation, callback failure, stale choice
/// or unwind drops the unpublished output. Success returns that same owner and
/// its decision record. Other clones retained by the caller remain independent.
/// Call for each selected output; item identity or an equal epoch does not permit
/// reusing a prior choice. This follows reconstruction and is not allocation
/// admission, display conversion or a whole-container privacy boundary.
pub fn resolve_image<R: PolicyResolver + ?Sized>(
    image: ImageOutput,
    resolver: &mut R,
    preset: PolicySet,
    stop: &dyn enough::Stop,
) -> Result<Outcome<ImageOutput>, At<PolicyFailure>> {
    let facts = ImageFacts::new(&image);
    let (_, record) = policy::choose(
        &[PublicationOffer::ReturnHeld],
        DecisionContext::Image(facts),
        |choices| Question::Image {
            image: &image,
            facts: &facts,
            choices,
        },
        Selection::Image,
        resolver,
        preset,
        stop,
    )?;
    let mut report = super::OperationReport::default();
    report.record(record);
    Ok(Outcome::from_parts(image, report))
}
