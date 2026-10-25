use super::alloc::boxed::Box;
use super::types::*;
use core::{convert::Infallible, error::Error, fmt};
use zencodec::At;

pub(crate) trait PolicyResolver {
    type Error: Error + Send + Sync + 'static;
    fn resolve(&mut self, question: Question<'_>) -> Result<Decision, At<Self::Error>>;
}

impl<F, E> PolicyResolver for F
where
    F: FnMut(Question<'_>) -> Result<Decision, At<E>>,
    E: Error + Send + Sync + 'static,
{
    type Error = E;
    fn resolve(&mut self, question: Question<'_>) -> Result<Decision, At<E>> {
        self(question)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum ExecutionPreference {
    FirstAllowed,
    BufferedUpTo(u64),
    Sequential,
    SmallestBound,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PolicySet {
    pub execution: ExecutionPreference,
    pub metadata: MetadataAction,
    pub prefer_lossless: bool,
}
impl Default for PolicySet {
    fn default() -> Self {
        Self {
            execution: ExecutionPreference::FirstAllowed,
            metadata: MetadataAction::FilterPublic,
            prefer_lossless: true,
        }
    }
}
impl PolicyResolver for PolicySet {
    type Error = Infallible;
    fn resolve(&mut self, question: Question<'_>) -> Result<Decision, At<Infallible>> {
        let Some(choices) = question.choices() else {
            return Ok(Decision::reject(Decline::UnknownQuestion));
        };
        let preferred = match question {
            Question::Execution { .. } => match self.execution {
                ExecutionPreference::FirstAllowed => choices.first(),
                ExecutionPreference::BufferedUpTo(cap) => choices
                    .iter()
                    .find(|c| {
                        c.value.action == Action::Execution(Execution::Buffered)
                            && c.value.ram.bound().is_some_and(|n| n <= cap)
                    })
                    .or_else(|| {
                        choices
                            .iter()
                            .find(|c| c.value.action == Action::Execution(Execution::Sequential))
                    }),
                ExecutionPreference::Sequential => choices
                    .iter()
                    .find(|c| c.value.action == Action::Execution(Execution::Sequential)),
                ExecutionPreference::SmallestBound => choices
                    .iter()
                    .min_by_key(|c| c.value.ram.bound().unwrap_or(u64::MAX)),
            },
            Question::Metadata { .. } => choices
                .iter()
                .find(|c| c.value.action == Action::Metadata(self.metadata)),
            Question::Fidelity { .. } if self.prefer_lossless => choices
                .iter()
                .find(|c| c.value.action == Action::Fidelity(zencodec::encode::Fidelity::Lossless)),
            _ => choices.first(),
        };
        // These are preferences. Required effects were filtered by the engine.
        Ok(preferred
            .or_else(|| choices.first())
            .map(Decision::choose)
            .unwrap_or_else(|| Decision::reject(Decline::NoPreferredChoice)))
    }
}

/// Erasure happens only on Err. Its source is the original concrete error;
/// ordinary Display intentionally does not expose application error payloads.
pub(crate) struct ErasedError(Box<dyn Error + Send + Sync>);
impl fmt::Debug for ErasedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ErasedPolicyError")
    }
}
impl fmt::Display for ErasedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("policy callback error")
    }
}
impl Error for ErasedError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.0.as_ref())
    }
}

pub(crate) struct Erased<R>(pub R);
impl<R: PolicyResolver> PolicyResolver for Erased<R> {
    type Error = ErasedError;
    fn resolve(&mut self, question: Question<'_>) -> Result<Decision, At<ErasedError>> {
        self.0.resolve(question).map_err(|e| {
            e.map_error(|e| ErasedError(Box::new(e)))
                .at_named("policy callback erasure")
        })
    }
}
