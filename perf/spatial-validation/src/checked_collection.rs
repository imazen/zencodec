//! Candidate consumer entry point: configure policy once, check every output.
//! Publication follows native reconstruction; this is not allocation admission.
use super::{
    collection::Collection,
    image_items::{ImageId, ImageRequest},
    image_output::ImageOutput,
    tiff_region::Failure,
};
use std::{fmt, sync::OnceLock};
use zencodec::{
    At, StopToken,
    enough::{Stop, StopReason},
    operation::{OperationReport, Outcome, PolicyResolver, PolicySet, resolve_image},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Stage {
    Open,
    Discovery,
    Decode,
    Policy,
    Stopped,
    Finished,
}

/// Safe default formatting; concrete causes and prior decisions remain available.
pub struct Error {
    pub stage: Stage,
    pub report: Box<OperationReport>,
    cause: Failure,
}
impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SelectedImageError")
            .field("stage", &self.stage)
            .field("report", &self.report)
            .finish_non_exhaustive()
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "selected-image request failed during {:?}", self.stage)
    }
}
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.cause.as_ref())
    }
}
fn failure(stage: Stage, report: &OperationReport, cause: impl Into<Failure>) -> Error {
    Error {
        stage,
        report: Box::new(report.clone()),
        cause: cause.into(),
    }
}

/// The first native or wrapper observation stops this opened operation, even
/// when a custom stop callback would subsequently return Ok. Policy failure and
/// request refusal do not change this token. This is private candidate machinery.
struct LatchedStop {
    source: StopToken,
    reason: OnceLock<StopReason>,
}
impl Stop for LatchedStop {
    fn check(&self) -> Result<(), StopReason> {
        if let Some(reason) = self.reason.get() {
            return Err(*reason);
        }
        if let Err(reason) = self.source.check() {
            // Preserve the first recorded reason, including future variants.
            return Err(*self.reason.get_or_init(|| reason));
        }
        // Another caller may have stopped while this callback returned Ok.
        self.reason.get().map_or(Ok(()), |reason| Err(*reason))
    }
}

pub struct Checked<C, R> {
    collection: Option<C>,
    resolver: R,
    preset: PolicySet,
    stop: StopToken,
    report: OperationReport,
}
impl<C: Collection, R: PolicyResolver> Checked<C, R> {
    /// Bind the same token to the native adapter and publication checks. The
    /// factory is adapter setup; ordinary selection methods need no callback args.
    pub fn open(
        stop: StopToken,
        resolver: R,
        open: impl FnOnce(StopToken) -> Result<C, Failure>,
    ) -> Result<Self, At<Error>> {
        let stop = StopToken::new(LatchedStop {
            source: stop,
            reason: OnceLock::new(),
        });
        let report = OperationReport::default();
        let run = || {
            check(&stop, &report)?;
            let collection = open(stop.clone()).map_err(|e| failure(Stage::Open, &report, e))?;
            check(&stop, &report)?;
            Ok(Self {
                collection: Some(collection),
                resolver,
                preset: PolicySet::new(),
                stop,
                report,
            })
        };
        run().map_err(At::wrap)
    }
    pub fn with_policy_set(mut self, preset: PolicySet) -> Self {
        self.preset = preset;
        self
    }
    pub fn report(&self) -> &OperationReport {
        &self.report
    }
    pub fn is_finished(&self) -> bool {
        self.collection.is_none()
    }

    fn run<T>(
        &mut self,
        operation: impl FnOnce(
            &mut C,
            &mut R,
            PolicySet,
            &StopToken,
            &mut OperationReport,
        ) -> Result<T, Error>,
    ) -> Result<T, At<Error>> {
        // An unwind drops native state and cannot leave a half-mutated operation
        // callable after catch_unwind. Ordinary errors remain request-local;
        // the native adapter retains responsibility for its own terminal state.
        let Some(mut collection) = self.collection.take() else {
            return Err(At::wrap(failure(
                Stage::Finished,
                &self.report,
                std::io::Error::other("selected-image operation has finished"),
            )));
        };
        check(&self.stop, &self.report).map_err(At::wrap)?;
        let result = operation(
            &mut collection,
            &mut self.resolver,
            self.preset,
            &self.stop,
            &mut self.report,
        );
        match check(&self.stop, &self.report) {
            Ok(()) => {
                self.collection = Some(collection);
                result.map_err(At::wrap)
            }
            Err(stopped) => {
                // Preserve the first native/policy cause; drop a successful
                // unpublished owner if cancellation arrived at the final seam.
                result.and_then(|_| Err(stopped)).map_err(At::wrap)
            }
        }
    }
    pub fn images(&mut self) -> Result<C::Cursor, At<Error>> {
        self.run(|c, _, _, _, report| c.images().map_err(|e| failure(Stage::Discovery, report, e)))
    }
    pub fn children(&mut self, image: &ImageId) -> Result<C::Cursor, At<Error>> {
        self.run(|c, _, _, _, report| {
            c.children(image)
                .map_err(|e| failure(Stage::Discovery, report, e))
        })
    }
    pub fn next(&mut self, cursor: &mut C::Cursor) -> Result<Option<ImageId>, At<Error>> {
        self.run(|c, _, _, _, report| {
            c.next(cursor)
                .map_err(|e| failure(Stage::Discovery, report, e))
        })
    }
    pub fn decode(
        &mut self,
        image: &ImageId,
        request: &ImageRequest,
    ) -> Result<Outcome<ImageOutput>, At<Error>> {
        self.run(|c, resolver, preset, stop, report| {
            let image = c
                .decode(image, request)
                .map_err(|e| failure(Stage::Decode, report, e))?;
            match resolve_image(image, resolver, preset, stop) {
                Ok(output) => {
                    for &record in output.report().decisions() {
                        report.record(record);
                    }
                    Ok(output)
                }
                Err(cause) => {
                    report.record(cause.error().record());
                    Err(failure(Stage::Policy, report, cause))
                }
            }
        })
    }
    pub fn first_image(
        &mut self,
        request: &ImageRequest,
    ) -> Result<Option<Outcome<ImageOutput>>, At<Error>> {
        let mut cursor = self.images()?;
        self.next(&mut cursor)?
            .map(|id| self.decode(&id, request))
            .transpose()
    }
}

fn check(stop: &StopToken, report: &OperationReport) -> Result<(), Error> {
    stop.check().map_err(|reason| {
        failure(
            Stage::Stopped,
            report,
            zencodec::CodecError::new(None, reason.into()),
        )
    })
}
