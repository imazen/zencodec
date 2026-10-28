//! Requirements belong to the operation, outside its replaceable callback.
use super::{Decision, PolicyResolver, PolicySet, Question};
use crate::At;
use core::{convert::Infallible, error::Error, fmt};

/// One operation's mandatory requirements and application selection callback.
///
/// Wrap logging or other callback decorators inside this binding. The binding
/// does not implement [`PolicyResolver`], so forwarding a callback cannot lose
/// its requirements. Reusing a binding does not share pending obligations:
/// each operation owns its own progress and evidence.
#[derive(Clone, Copy)]
pub struct BoundPolicy<R: ?Sized> {
    requirements: PolicySet,
    resolver: R,
}
impl<R: PolicyResolver> BoundPolicy<R> {
    /// Bind requirements once, before any application selection.
    pub const fn new(requirements: PolicySet, resolver: R) -> Self {
        Self {
            requirements,
            resolver,
        }
    }
    /// Recover the configured requirements and callback.
    pub fn into_parts(self) -> (PolicySet, R) {
        (self.requirements, self.resolver)
    }
}
impl<R: ?Sized> BoundPolicy<R> {
    /// Requirements and built-in preferences used by this binding.
    pub const fn requirements(&self) -> PolicySet {
        self.requirements
    }
    /// Access callback state without replacing or weakening the requirements.
    pub fn resolver_mut(&mut self) -> &mut R {
        &mut self.resolver
    }
}
impl<R: ?Sized> fmt::Debug for BoundPolicy<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BoundPolicy")
            .field("requirements", &self.requirements)
            .finish_non_exhaustive()
    }
}

mod sealed {
    pub trait Sealed {}
}

/// Policy accepted by operation entry points.
///
/// Pass a [`PolicySet`] for built-in selection, a closure for application-only
/// selection, or a [`BoundPolicy`] for both. Custom callback structs implement
/// [`PolicyResolver`] and go inside a binding. This trait is sealed so callback
/// wrappers cannot accidentally inherit a default that drops requirements.
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not an operation policy",
    note = "pass a PolicySet, a closure, or BoundPolicy::new(requirements, resolver)"
)]
pub trait OperationPolicy: sealed::Sealed {
    /// Application error retained with its original location trace.
    type Error: Error + Send + Sync + 'static;
    /// Bound requirements and preferences; bare callbacks have none.
    /// `None` also retains a legacy entry point's configured preferences during
    /// migration. It never cancels that entry point's requirements.
    fn requirements(&self) -> Option<PolicySet>;
    /// Select among already admitted choices.
    fn decide(&mut self, question: Question<'_>) -> Result<Decision, At<Self::Error>>;
}
impl sealed::Sealed for PolicySet {}
impl OperationPolicy for PolicySet {
    type Error = Infallible;
    fn requirements(&self) -> Option<PolicySet> {
        Some(*self)
    }
    fn decide(&mut self, question: Question<'_>) -> Result<Decision, At<Infallible>> {
        self.resolve_builtin(question)
    }
}
impl sealed::Sealed for &mut PolicySet {}
impl OperationPolicy for &mut PolicySet {
    type Error = Infallible;
    fn requirements(&self) -> Option<PolicySet> {
        Some(**self)
    }
    fn decide(&mut self, question: Question<'_>) -> Result<Decision, At<Infallible>> {
        self.resolve_builtin(question)
    }
}
impl<R: PolicyResolver + ?Sized> sealed::Sealed for BoundPolicy<R> {}
impl<R: PolicyResolver + ?Sized> OperationPolicy for BoundPolicy<R> {
    type Error = R::Error;
    fn requirements(&self) -> Option<PolicySet> {
        Some(self.requirements)
    }
    fn decide(&mut self, question: Question<'_>) -> Result<Decision, At<R::Error>> {
        self.resolver.resolve(question)
    }
}
impl<R: PolicyResolver + ?Sized> sealed::Sealed for &mut BoundPolicy<R> {}
impl<R: PolicyResolver + ?Sized> OperationPolicy for &mut BoundPolicy<R> {
    type Error = R::Error;
    fn requirements(&self) -> Option<PolicySet> {
        Some(self.requirements)
    }
    fn decide(&mut self, question: Question<'_>) -> Result<Decision, At<R::Error>> {
        self.resolver.resolve(question)
    }
}
impl<F, E> sealed::Sealed for F
where
    F: FnMut(Question<'_>) -> Result<Decision, At<E>>,
    E: Error + Send + Sync + 'static,
{
}
impl<F, E> OperationPolicy for F
where
    F: FnMut(Question<'_>) -> Result<Decision, At<E>>,
    E: Error + Send + Sync + 'static,
{
    type Error = E;
    fn requirements(&self) -> Option<PolicySet> {
        None
    }
    fn decide(&mut self, question: Question<'_>) -> Result<Decision, At<E>> {
        self(question)
    }
}
