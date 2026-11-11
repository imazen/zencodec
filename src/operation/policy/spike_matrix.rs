//! Differential selector oracle plus a private one-policy signature prototype.
//! Synthetic facts test policy logic; they are not codec memory/privacy proofs.
use super::*;
use crate::{
    decode::{DecodeBoundary, InterpretationState},
    encode::{EncodeDeclaration, EncodePlan, OutputTiming},
    frame::{FrameGeometry, FrameTiming},
    image::{ImageGeometry, ImageInterpretation, ImageOutput, Rect},
    operation::{
        BoundPolicy, EncodeEvidence, EncodeRouteFacts, EncodeRouteOffer, FrameConversionOffer,
        FrameFacts, FrameRoute, FrameRouteFacts, FrameRouteOffer, FrameRouteState,
        ImageConversionOffer, InputAcquisition, OperationPolicy, OutputSchedule,
        OutputScheduleFacts, OutputScheduleOffer, OutputSource, ReplayEvidence, resolve_transcode,
    },
};
use alloc::{sync::Arc, vec};
use zenpixels::{
    ChannelType, Orientation, PixelBuffer,
    plane::{PlanarLayout, PlaneEncoding, PlaneRole, PlaneSpec},
    sample::{ChromaSubsampling, SampleEncoding},
};

fn outcome(decision: Decision) -> (Option<u8>, Option<Decline>) {
    match decision {
        Decision::Choose(id) => (Some(id.index), None),
        Decision::Decline(reason) => (None, Some(*reason.error())),
        Decision::UsePreset => panic!("built-in delegated"),
    }
}
fn compare(mut policy: PolicySet, question: Question<'_>, bits: u16, mask: u64) {
    let old = outcome(policy.baseline_builtin(question).unwrap());
    let new = outcome(policy.resolve_builtin(question).unwrap());
    assert_eq!(new, old, "bits={bits} mask={mask} question={question:?}");
}
fn policies(bits: u16) -> PolicySet {
    let mut p = PolicySet::new()
        .prefer_lossless(bits & 1 != 0)
        .allow_assumed_chroma_siting(bits & 2 != 0)
        .allow_opaque_profiles(bits & 4 == 0)
        .require_extended_range(bits & 8 != 0)
        .require_exact_replay(bits & 16 != 0)
        .allow_raw_memory_staging(bits & 32 == 0);
    if bits & 64 != 0 {
        p = p.with_workspace_limit(16);
    }
    if bits & 128 != 0 {
        p = p.with_output_wait_limit(16);
    }
    if bits & 256 != 0 {
        p = p.with_max_source_passes(1);
    }
    if bits & 512 != 0 {
        p = p.require_output_schedule(OutputSchedule::FinishFirst);
    }
    if bits & 1024 != 0 {
        p = p.require_metadata(MetadataTransfer::DiscardDescriptive);
    }
    if bits & 2048 != 0 {
        p = p.require_metadata(MetadataTransfer::PreserveReported);
    }
    p
}
fn facts() -> InterpretationFacts {
    InterpretationFacts::new(
        2,
        1,
        Some(PixelDescriptor::GRAY8),
        InterpretationState::Final,
    )
}
fn encoders() -> [EncodeOffer; 3] {
    core::array::from_fn(|i| EncodeOffer {
        format: ImageFormat::Png,
        resolution: None,
        preference_index: i,
        fidelity: [
            Some(Fidelity::codec_quality(80.0)),
            None,
            Some(Fidelity::Lossless),
        ][i],
    })
}
fn transcodes() -> [TranscodeOffer; 3] {
    let enc = encoders();
    core::array::from_fn(|i| {
        let mut t = TranscodeOffer::new(
            enc[i],
            PixelDescriptor::GRAY8,
            if i == 0 {
                MetadataTransfer::PreserveReported
            } else {
                MetadataTransfer::DiscardDescriptive
            },
        );
        t.retains_opaque_profile = [Some(true), None, Some(false)][i];
        t.workspace_upper_bound = [Some(17), None, Some(16)][i];
        t.output_schedule = Some(schedules()[i]);
        t
    })
}
fn schedules() -> [OutputScheduleOffer; 3] {
    core::array::from_fn(|i| {
        let mut s = OutputScheduleOffer::new(if i == 0 {
            OutputSchedule::Live
        } else {
            OutputSchedule::FinishFirst
        });
        s.wait_retained_upper_bound = [Some(17), None, Some(16)][i];
        s
    })
}
#[test]
fn all_nine_domains_match_frozen_selector_across_184320_policy_choice_cases() {
    let interpretation = Arc::new(FrameInterpretation::new(
        FrameGeometry::full(2, 1),
        PlanarLayout::new(
            vec![PlaneSpec::new(
                PlaneRole::Luma,
                PlaneEncoding::Code(SampleEncoding::new(ChannelType::U8, 8, 0).unwrap()),
            )],
            ChromaSubsampling::Mono,
            ChromaSiting::Unknown,
        )
        .unwrap(),
    ));
    let geometry = ImageGeometry {
        canvas_width: 2,
        canvas_height: 1,
        region: Rect {
            x: 0,
            y: 0,
            width: 2,
            height: 1,
        },
    };
    let image = ImageOutput::packed(
        PixelBuffer::from_vec(vec![13, 37], 2, 1, PixelDescriptor::GRAY8).unwrap(),
        vec![],
        ImageInterpretation::packed(geometry, Orientation::Identity),
    )
    .unwrap();
    let image_facts = ImageFacts::new(&image);
    let frame_facts = FrameRouteFacts {
        input: FrameFacts::new(&interpretation, FrameTiming::default()),
        output: FrameRouteState::default(),
    };
    let enc = encoders();
    let transcode = transcodes();
    let schedule = schedules();
    let publication = [PublicationOffer::ReturnHeld; 3];
    let boundary = [
        BoundaryOffer {
            action: BoundaryAction::ContinueReading,
        },
        BoundaryOffer {
            action: BoundaryAction::AdoptInterpretation,
        },
        BoundaryOffer {
            action: BoundaryAction::AdoptInterpretation,
        },
    ];
    let conversion = core::array::from_fn::<_, 3, _>(|i| {
        let mut c =
            ImageConversionOffer::new(PixelDescriptor::GRAY8, geometry, Orientation::Identity);
        c.workspace_upper_bound = [Some(17), None, Some(16)][i];
        c.preserves_extended_range = i == 2;
        c.assumed_chroma_siting = [
            Some(ChromaSiting::Unknown),
            Some(ChromaSiting::Center),
            None,
        ][i];
        c
    });
    let frame_routes = core::array::from_fn::<_, 3, _>(|i| {
        let mut r = FrameRouteOffer::new(FrameRoute::Continue);
        r.fidelity = enc[i].fidelity;
        r.workspace_upper_bound = conversion[i].workspace_upper_bound;
        let mut c = FrameConversionOffer::new(interpretation.clone(), FrameTiming::default());
        c.preserves_samples = i == 2;
        c.preserves_extended_range = i == 2;
        c.assumed_chroma_siting = conversion[i].assumed_chroma_siting;
        r.conversion = Some(c);
        r.output_schedule = Some(schedule[i]);
        r
    });
    let declaration = EncodeDeclaration::new(2, 1, PixelDescriptor::GRAY8);
    let plan = EncodePlan::rows(OutputTiming::streaming(0));
    let mut evidence = EncodeEvidence::new(enc[2]);
    evidence.metadata = Some(MetadataTransfer::DiscardDescriptive);
    evidence.retains_opaque_profile = Some(false);
    let route_facts = EncodeRouteFacts::new(&declaration, &plan, evidence);
    let routes = core::array::from_fn::<_, 3, _>(|i| {
        let mut r = EncodeRouteOffer::new(
            [
                InputAcquisition::Unknown,
                InputAcquisition::MemorySnapshot,
                InputAcquisition::DirectRows,
            ][i],
            schedule[i],
        );
        r.workspace_upper_bound = conversion[i].workspace_upper_bound;
        r.preserves_extended_range = Some(i == 2);
        r.replay = ReplayEvidence::Unknown;
        r
    });
    let mut cases = 0;
    for bits in 0..4096 {
        let p = policies(bits);
        for mask in [0, 1, 2, 4, 7] {
            macro_rules! check {
                ($variant:ident { $($name:ident : $value:expr),* }, $entries:expr) => {{
                    compare(p, Question::$variant { $($name: $value,)* choices: Choices {
                        ticket: 1, entries: $entries, admitted: mask } }, bits, mask);
                    cases += 1;
                }};
            }
            check!(
                Encode {
                    facts: EncodeFacts {
                        width: 2,
                        height: 1,
                        descriptor: PixelDescriptor::GRAY8
                    }
                },
                &enc
            );
            check!(Boundary { facts: facts() }, &boundary);
            check!(Transcode { facts: facts() }, &transcode);
            check!(
                OutputSchedule {
                    facts: OutputScheduleFacts::new(plan.timing, OutputSource::PullRows)
                },
                &schedule
            );
            check!(
                Image {
                    image: &image,
                    facts: &image_facts
                },
                &publication
            );
            check!(
                Frame {
                    interpretation: &interpretation,
                    timing: FrameTiming::default()
                },
                &publication
            );
            check!(
                ImageConversion {
                    image: &image,
                    facts: &image_facts
                },
                &conversion
            );
            check!(
                FrameRoute {
                    interpretation: &interpretation,
                    facts: &frame_facts
                },
                &frame_routes
            );
            check!(
                EncodeRoute {
                    declaration: &declaration,
                    plan: &plan,
                    facts: route_facts
                },
                &routes
            );
        }
    }
    assert_eq!(cases, 184320);
}

// Candidate modern signature: no independently configured fallback preset.
// This facade prototypes caller syntax; it does not itself remove old exports.
fn transcode<P: OperationPolicy>(
    offers: &[TranscodeOffer],
    policy: &mut P,
) -> Result<(usize, DecisionRecord), At<PolicyFailure>> {
    resolve_transcode(
        DecodeBoundary::new(facts()).unwrap(),
        offers,
        policy,
        PolicySet::new(),
        None,
    )
}
#[test]
fn single_policy_keeps_hard_rules_with_a_direct_callback_and_late_evidence() {
    let requirements = PolicySet::new()
        .require_metadata(MetadataTransfer::DiscardDescriptive)
        .allow_opaque_profiles(false)
        .with_workspace_limit(16);
    let offers = transcodes();
    let mut policy = BoundPolicy::new(
        requirements,
        |q: Question<'_>| -> Result<Decision, At<Infallible>> {
            let Question::Transcode { choices, .. } = q else {
                panic!("unexpected question")
            };
            assert_eq!(choices.len(), 1);
            Ok(Decision::choose(choices.first().unwrap()))
        },
    );
    let (index, record) = transcode(&offers, &mut policy).unwrap();
    assert_eq!((index, record.admitted), (2, 1));
    let mut unknown = offers[2];
    unknown.retains_opaque_profile = None;
    assert!(transcode(&[unknown], &mut policy).is_err());
    unknown.retains_opaque_profile = Some(false);
    assert!(transcode(&[unknown], &mut policy).is_ok());
    // Early discovery may gather evidence; final adoption cannot waive it.
    let early = DecodeBoundary::new(InterpretationFacts::new(
        2,
        1,
        None,
        InterpretationState::Pending,
    ))
    .unwrap();
    assert!(resolve_boundary(early, &mut { requirements }, PolicySet::new(), None).is_ok());
    assert!(
        resolve_boundary(
            DecodeBoundary::new(facts()).unwrap(),
            &mut { requirements },
            PolicySet::new(),
            None
        )
        .is_err()
    );
}
#[test]
fn deleting_a_second_requirement_without_an_explicit_merge_weakens_the_contract() {
    let offers = transcodes();
    let newer = PolicySet::new().require_metadata(MetadataTransfer::DiscardDescriptive);
    let older = PolicySet::new().require_metadata(MetadataTransfer::PreserveReported);
    assert!(
        resolve_transcode(
            DecodeBoundary::new(facts()).unwrap(),
            &offers,
            &mut { newer },
            older,
            None
        )
        .is_err()
    );
    // A mechanical removal incorrectly succeeds. Migration needs to reject the
    // contradictory pair, not silently choose either policy as the survivor.
    assert!(transcode(&offers, &mut { newer }).is_ok());
}
#[test]
fn direct_decide_cannot_treat_weakly_admitted_choices_as_strict_evidence() {
    let offers = transcodes();
    let q = Question::Transcode {
        facts: facts(),
        choices: Choices {
            ticket: 1,
            entries: &offers[..2],
            admitted: 3,
        },
    };
    let mut strict = PolicySet::new().allow_opaque_profiles(false);
    assert!(matches!(strict.decide(q).unwrap(), Decision::Decline(_)));
}

// Prototype migration of two sources into one bound policy. Preferences follow
// the existing bound-policy precedence; mandatory constraints intersect.
// Deliberately private: conflict timing and diagnostics need API review.
fn intersect(older: PolicySet, bound: PolicySet) -> Result<PolicySet, PolicyRequirement> {
    fn same<T: Copy + PartialEq>(
        a: Option<T>,
        b: Option<T>,
        requirement: PolicyRequirement,
    ) -> Result<Option<T>, PolicyRequirement> {
        match (a, b) {
            (Some(a), Some(b)) if a != b => Err(requirement),
            (Some(a), _) => Ok(Some(a)),
            (_, b) => Ok(b),
        }
    }
    fn minimum<T: Copy + Ord>(a: Option<T>, b: Option<T>) -> Option<T> {
        match (a, b) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (Some(a), _) => Some(a),
            (_, b) => b,
        }
    }
    Ok(PolicySet {
        metadata: same(older.metadata, bound.metadata, PolicyRequirement::Metadata)?,
        output_schedule: same(
            older.output_schedule,
            bound.output_schedule,
            PolicyRequirement::OutputSchedule,
        )?,
        allow_opaque_profiles: older.allow_opaque_profiles && bound.allow_opaque_profiles,
        workspace_limit: minimum(older.workspace_limit, bound.workspace_limit),
        output_wait_limit: minimum(older.output_wait_limit, bound.output_wait_limit),
        max_source_passes: minimum(older.max_source_passes, bound.max_source_passes),
        require_extended_range: older.require_extended_range || bound.require_extended_range,
        exact_replay: older.exact_replay || bound.exact_replay,
        allow_raw_memory_staging: older.allow_raw_memory_staging && bound.allow_raw_memory_staging,
        ..bound
    })
}
fn summary(
    result: Result<(usize, DecisionRecord), At<PolicyFailure>>,
) -> (
    Option<usize>,
    DecisionCode,
    alloc::vec::Vec<(PolicyRequirement, RequirementEvidence)>,
) {
    match result {
        Ok((index, record)) => (
            Some(index),
            record.code,
            record.unmet_requirements.iter().collect(),
        ),
        Err(error) => {
            let record = error.error().record();
            (
                None,
                record.code,
                record.unmet_requirements.iter().collect(),
            )
        }
    }
}
#[test]
fn explicit_intersection_matches_two_source_transcode_for_4096_policy_pairs() {
    let offers = transcodes();
    for bits in 0..4096u16 {
        let older = policies(bits);
        // A permutation retains each of the 4096 presets on both sides.
        let bound = policies((bits.wrapping_mul(1549).wrapping_add(811)) & 4095);
        let old = resolve_transcode(
            DecodeBoundary::new(facts()).unwrap(),
            &offers,
            &mut BoundPolicy::new(
                bound,
                |_: Question<'_>| -> Result<Decision, At<Infallible>> { Ok(Decision::UsePreset) },
            ),
            older,
            None,
        );
        match intersect(older, bound) {
            Ok(merged) => {
                let new = transcode(
                    &offers,
                    &mut BoundPolicy::new(
                        merged,
                        |_: Question<'_>| -> Result<Decision, At<Infallible>> {
                            Ok(Decision::UsePreset)
                        },
                    ),
                );
                assert_eq!(summary(new), summary(old), "pair {bits}");
            }
            Err(requirement) => {
                let error = old.unwrap_err();
                assert!(
                    error
                        .error()
                        .record()
                        .unmet_requirements
                        .iter()
                        .any(|(r, _)| r == requirement)
                );
            }
        }
    }
}
#[test]
fn eager_intersection_changes_legal_discovery_timing_for_conflicting_metadata() {
    let older = PolicySet::new().require_metadata(MetadataTransfer::PreserveReported);
    let bound = PolicySet::new().require_metadata(MetadataTransfer::DiscardDescriptive);
    assert_eq!(
        intersect(older, bound).unwrap_err(),
        PolicyRequirement::Metadata
    );
    let early = DecodeBoundary::new(InterpretationFacts::new(
        2,
        1,
        None,
        InterpretationState::Pending,
    ))
    .unwrap();
    // The existing two-source contract can still discover facts before final
    // adoption. A fallible merge rejects sooner; that is an observable change.
    assert!(resolve_boundary(early, &mut { bound }, older, None).is_ok());
}
