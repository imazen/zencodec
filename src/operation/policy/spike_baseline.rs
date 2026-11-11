//! Frozen PR208 selector, test-only differential oracle for the spike.
use super::*;
impl PolicySet {
    fn baseline_permits_output_schedule(
        self,
        offer: Option<&crate::operation::OutputScheduleOffer>,
    ) -> bool {
        if self.output_schedule.is_none() && self.output_wait_limit.is_none() {
            return true;
        }
        offer.is_some_and(|offer| {
            self.output_schedule.is_none_or(|s| s == offer.schedule)
                && self.output_wait_limit.is_none_or(|limit| {
                    offer
                        .wait_retained_upper_bound
                        .is_some_and(|bound| bound <= limit)
                })
        })
    }
    fn baseline_permits_transcode(self, offer: &TranscodeOffer) -> bool {
        self.baseline_permits_output_schedule(offer.output_schedule.as_ref())
            && self.metadata.is_none_or(|m| m == offer.metadata)
            && (self.allow_opaque_profiles || offer.retains_opaque_profile == Some(false))
            && self.workspace_limit.is_none_or(|limit| {
                offer
                    .workspace_upper_bound
                    .is_some_and(|bound| bound <= limit)
            })
    }
    pub(super) fn baseline_builtin(
        &mut self,
        question: Question<'_>,
    ) -> Result<Decision, At<Infallible>> {
        if (self.output_schedule.is_some() || self.output_wait_limit.is_some())
            && !matches!(
                question,
                Question::OutputSchedule { .. }
                    | Question::EncodeRoute { .. }
                    | Question::Transcode { .. }
                    | Question::FrameRoute { .. }
                    | Question::Boundary { .. }
            )
        {
            return Ok(Decision::decline(Decline::UnsupportedQuestion));
        }
        if self.require_extended_range
            && !matches!(
                question,
                Question::ImageConversion { .. }
                    | Question::EncodeRoute { .. }
                    | Question::FrameRoute { .. }
                    | Question::Boundary { .. }
            )
        {
            return Ok(Decision::decline(Decline::UnsupportedQuestion));
        }
        if (self.max_source_passes.is_some() || self.exact_replay || !self.allow_raw_memory_staging)
            && !matches!(
                question,
                Question::EncodeRoute { .. } | Question::Boundary { .. }
            )
        {
            return Ok(Decision::decline(Decline::UnsupportedQuestion));
        }
        let choices = match question {
            Question::EncodeRoute {
                declaration,
                plan,
                facts,
                choices,
            } => {
                // A single configured job: fidelity is identical across these
                // acquisition/output alternatives. Preserve request order.
                return Ok(choices
                    .iter()
                    .find(|choice| {
                        self.unmet(Question::EncodeRoute {
                            declaration,
                            plan,
                            facts,
                            choices: Choices {
                                ticket: choices.ticket,
                                entries: choices.entries,
                                admitted: 1u64 << choice.id.index,
                            },
                        })
                        .is_empty()
                    })
                    .map(Decision::choose)
                    .unwrap_or_else(|| Decision::decline(Decline::NoPreferredChoice)));
            }
            Question::OutputSchedule { choices, .. } => {
                if self.metadata.is_some()
                    || !self.allow_opaque_profiles
                    || self.workspace_limit.is_some()
                {
                    return Ok(Decision::decline(Decline::UnsupportedQuestion));
                }
                return Ok(choices
                    .iter()
                    .find(|c| {
                        let offer = c.value();
                        self.output_schedule.is_none_or(|s| s == offer.schedule)
                            && self.output_wait_limit.is_none_or(|limit| {
                                offer
                                    .wait_retained_upper_bound
                                    .is_some_and(|bound| bound <= limit)
                            })
                    })
                    .map(Decision::choose)
                    .unwrap_or_else(|| Decision::decline(Decline::NoPreferredChoice)));
            }
            Question::ImageConversion { choices, .. } => {
                // Conversion alone does not certify metadata or opaque-profile
                // treatment. Do not silently ignore an existing obligation.
                if self.metadata.is_some() || !self.allow_opaque_profiles {
                    return Ok(Decision::decline(Decline::UnsupportedQuestion));
                }
                return Ok(choices
                    .iter()
                    .find(|choice| {
                        let offer = choice.value();
                        (offer.assumed_chroma_siting.is_none() || self.allow_assumed_siting)
                            && offer.assumed_chroma_siting != Some(ChromaSiting::Unknown)
                            && (!self.require_extended_range || offer.preserves_extended_range)
                            && self.workspace_limit.is_none_or(|limit| {
                                offer
                                    .workspace_upper_bound
                                    .is_some_and(|bound| bound <= limit)
                            })
                    })
                    .map(Decision::choose)
                    .unwrap_or_else(|| Decision::decline(Decline::NoPreferredChoice)));
            }
            Question::FrameRoute { choices, .. } => {
                // Encoding continuity does not establish metadata treatment or
                // whole-container disclosure evidence.
                if self.metadata.is_some() || !self.allow_opaque_profiles {
                    return Ok(Decision::decline(Decline::UnsupportedQuestion));
                }
                let permitted = || {
                    choices.iter().filter(|choice| {
                        let conversion = choice.value().conversion.as_ref();
                        let conversion_permitted = conversion.is_none_or(|c| {
                            (c.assumed_chroma_siting.is_none() || self.allow_assumed_siting)
                                && c.assumed_chroma_siting != Some(ChromaSiting::Unknown)
                        }) && (!self.require_extended_range
                            || (choice.value().fidelity == Some(Fidelity::Lossless)
                                && conversion.is_some_and(|c| c.preserves_extended_range)));
                        conversion_permitted
                            && self.baseline_permits_output_schedule(
                                choice.value().output_schedule.as_ref(),
                            )
                            && self.workspace_limit.is_none_or(|limit| {
                                choice
                                    .value()
                                    .workspace_upper_bound
                                    .is_some_and(|bound| bound <= limit)
                            })
                    })
                };
                let selected = if self.prefer_lossless {
                    // Prefer a qualified complete route before considering
                    // encoder-only evidence. This is still a preference, not
                    // permission to violate any required bound.
                    permitted()
                        .find(|choice| {
                            choice.value().fidelity == Some(Fidelity::Lossless)
                                && choice
                                    .value()
                                    .conversion
                                    .as_ref()
                                    .is_some_and(|c| c.preserves_samples)
                        })
                        .or_else(|| {
                            permitted()
                                .find(|choice| choice.value().fidelity == Some(Fidelity::Lossless))
                        })
                        .or_else(|| permitted().next())
                } else {
                    permitted().next()
                };
                return Ok(selected
                    .map(Decision::choose)
                    .unwrap_or_else(|| Decision::decline(Decline::NoPreferredChoice)));
            }
            Question::Frame { choices, .. } | Question::Image { choices, .. } => {
                // Native publication has no evidence for these transcode-only
                // obligations. A preset must not silently ignore them.
                if self.metadata.is_some()
                    || !self.allow_opaque_profiles
                    || self.workspace_limit.is_some()
                {
                    return Ok(Decision::decline(Decline::UnsupportedQuestion));
                }
                return Ok(choices
                    .first()
                    .map(Decision::choose)
                    .unwrap_or_else(|| Decision::decline(Decline::NoPreferredChoice)));
            }
            Question::Transcode { choices, .. } => {
                let permitted = || {
                    choices
                        .iter()
                        .filter(|c| self.baseline_permits_transcode(c.value()))
                };
                let choice = if self.prefer_lossless {
                    permitted()
                        .find(|c| c.value().encoder.fidelity == Some(Fidelity::Lossless))
                        .or_else(|| permitted().next())
                } else {
                    permitted().next()
                };
                return Ok(choice
                    .map(Decision::choose)
                    .unwrap_or_else(|| Decision::decline(Decline::NoPreferredChoice)));
            }
            Question::Encode { choices, .. } => {
                if self.metadata.is_some()
                    || !self.allow_opaque_profiles
                    || self.workspace_limit.is_some()
                {
                    return Ok(Decision::decline(Decline::UnsupportedQuestion));
                }
                choices
            }
            Question::Boundary { choices, .. } => {
                // Discovery may continue, but adopting interpretation permits
                // native output immediately. There need not be another policy
                // question before rows reach a sink, so requirements lacking
                // evidence here must not silently disappear at adoption.
                let unproven = self.metadata.is_some()
                    || !self.allow_opaque_profiles
                    || self.workspace_limit.is_some()
                    || self.require_extended_range
                    || self.output_schedule.is_some()
                    || self.output_wait_limit.is_some()
                    || self.max_source_passes.is_some()
                    || self.exact_replay
                    || !self.allow_raw_memory_staging;
                return Ok(choices
                    .iter()
                    .find(|choice| {
                        !unproven || choice.value().action == BoundaryAction::ContinueReading
                    })
                    .map(Decision::choose)
                    .unwrap_or_else(|| {
                        Decision::decline(if unproven {
                            Decline::UnsupportedQuestion
                        } else {
                            Decline::NoPreferredChoice
                        })
                    }));
            }
        };
        let choice = if self.prefer_lossless {
            choices
                .iter()
                .find(|choice| choice.value().fidelity == Some(Fidelity::Lossless))
                .or_else(|| choices.first())
        } else {
            choices.first()
        };
        Ok(choice
            .map(Decision::choose)
            .unwrap_or_else(|| Decision::decline(Decline::NoPreferredChoice)))
    }
}
