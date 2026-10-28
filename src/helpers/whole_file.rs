//! [`WholeFileIncremental`]: the sans-I/O contract for a codec core that
//! needs the complete file.

use alloc::borrow::Cow;
use alloc::vec::Vec;

use zenpixels::PixelDescriptor;

use crate::traits::{
    BatchMeta, DecodeJob, Event, IncrementalDecode, Input, InputRequest, MetadataUpdate, Progress,
    StepError,
};
use crate::{DecodeRowSink, ImageInfo, OutputInfo};

/// First prefix asked for when probing the header.
const FIRST_PROBE: usize = 4096;

/// An [`IncrementalDecode`] for a codec whose decoder needs the whole file.
///
/// It gives such a codec the full driver ecosystem (files, streams, async
/// sources, FFI hosts) with no I/O in the codec and pixels identical to its
/// own push path:
///
/// - While bytes arrive it re-runs the job's [`probe`](DecodeJob::probe) on
///   a doubling prefix, and reports [`MetadataUpdate::HeaderReady`] (with
///   [`info`](IncrementalDecode::info)) as soon as the probe succeeds.
/// - Once the input reaches the end of the source it runs the job's
///   [`push_decoder`](DecodeJob::push_decoder) on the complete bytes, which
///   writes every row into the caller's sink, and reports them as one
///   [`Event::Rows`].
///
/// It holds no input of its own: it asks for a growing prefix from offset 0
/// (or for the whole source once its length is known), so the driver holds
/// the file exactly as a one-shot decode would. A codec that can decode as
/// data arrives should implement [`IncrementalDecode`] natively instead.
///
/// ```rust,ignore
/// fn dyn_incremental_decoder(self, preferred: &[PixelDescriptor])
///     -> Result<Box<DynIncrementalDecoder>, BoxedError>
/// {
///     Ok(zencodec::decode::erase_incremental(
///         zencodec::helpers::WholeFileIncremental::new(self, preferred),
///     ))
/// }
/// ```
///
/// [`erase_incremental`](crate::decode::erase_incremental) adds the
/// termination rules and a pulse check before every step. The job's own pulse
/// token (if any) cancels inside the final decode.
pub struct WholeFileIncremental<J> {
    job: Option<J>,
    preferred: Vec<PixelDescriptor>,
    info: Option<ImageInfo>,
    /// The header probe has succeeded or given up for want of more input.
    probed: bool,
    want: usize,
    need: [InputRequest; 1],
    /// Set once the decode has run: the next step reports `Done`.
    decoded: Option<OutputInfo>,
}

impl<J> WholeFileIncremental<J>
where
    J: for<'a> DecodeJob<'a> + Send + 'static,
{
    /// Wrap `job`, decoding to the first descriptor in `preferred` the codec
    /// can produce (as [`push_decoder`](DecodeJob::push_decoder) does).
    pub fn new(job: J, preferred: &[PixelDescriptor]) -> Self {
        Self {
            job: Some(job),
            preferred: preferred.to_vec(),
            info: None,
            probed: false,
            want: FIRST_PROBE,
            need: [InputRequest::at_least(0, FIRST_PROBE)],
            decoded: None,
        }
    }

    /// Ask for more of the file from offset 0: all of it when the length is
    /// known, otherwise twice what was offered.
    fn need_more(&mut self, input: &Input<'_>) -> Progress {
        let held = if input.offset == 0 {
            input.bytes.len()
        } else {
            0
        };
        let want = match input.total_len {
            Some(total) => total as usize,
            None => self.want.max(held.saturating_mul(2)).max(held + 1),
        };
        self.want = want.saturating_mul(2);
        self.need = [InputRequest::at_least(0, want)];
        Progress::need(0)
    }
}

impl<J, E> IncrementalDecode for WholeFileIncremental<J>
where
    J: for<'a> DecodeJob<'a, Error = E> + Send + 'static,
    E: core::error::Error + Send + Sync + 'static,
{
    type Error = E;

    fn step(
        &mut self,
        input: Input<'_>,
        sink: &mut dyn DecodeRowSink,
        pulse: &dyn crate::Pulse,
    ) -> Result<Progress, StepError<Self::Error>> {
        pulse.check()?;
        if let Some(out) = &self.decoded {
            return Ok(Progress::new(0, Event::Done(out.clone())));
        }
        let complete = input.offset == 0 && input.reaches_end();
        if !self.probed {
            let job = self.job.as_ref().ok_or(StepError::Finished)?;
            let prefix = if input.offset == 0 { input.bytes } else { &[] };
            match job.probe(prefix) {
                Ok(info) => {
                    self.info = Some(info);
                    self.probed = true;
                    return Ok(Progress::new(
                        0,
                        Event::Metadata(MetadataUpdate::HeaderReady),
                    ));
                }
                // A prefix that does not probe yet: read on. The complete
                // file that does not probe is left to the decoder, which
                // reports its own error.
                Err(_) if !complete => return Ok(self.need_more(&input)),
                Err(_) => self.probed = true,
            }
        }
        if !complete {
            return Ok(self.need_more(&input));
        }
        let job = self.job.take().ok_or(StepError::Finished)?;
        let out = job
            .push_decoder(Cow::Borrowed(input.bytes), sink, &self.preferred)
            .map_err(StepError::Codec)?;
        let rows = out.height;
        self.decoded = Some(out);
        Ok(Progress::new(
            input.bytes.len(),
            Event::Rows {
                meta: BatchMeta::final_rows(0),
                rows,
            },
        ))
    }

    fn requested(&self) -> &[InputRequest] {
        &self.need
    }

    fn info(&self) -> Option<&ImageInfo> {
        self.info.as_ref()
    }

    fn output_descriptor(&self) -> Option<PixelDescriptor> {
        self.decoded.as_ref().map(|o| o.native_format)
    }
}
