//! A retained source/document binding, with bounded lazy discovery.
//!
//! Queries use the existing input-window planner. A selected image uses the
//! existing decoder and row sink. Errors during execution poison the session;
//! unsupported/foreign requests rejected before execution leave it usable.

//!
//! ```no_run
//! use core::num::NonZeroU16;
//! use zencodec::{Result, decode::{DecodeSpec, Sink}, document::*};
//! use zencodec_io::{ReadAt, DriveOptions, document::DocumentReader};
//!
//! fn browse_then_decode(codec: &dyn DocumentCodec, source: impl ReadAt,
//!                       sink: &mut dyn Sink) -> Result<()> {
//!     let mut document = DocumentReader::open(
//!         codec, source, OpenSpec::default(), DriveOptions::default())?;
//!     let page = document.query(Query::list(Listing::Images, NonZeroU16::new(16).unwrap()))?;
//!     if let Answer::Page { entries, .. } = page {
//!         for entry in entries {
//!             if let Entry::Image(image) = entry {
//!                 // Query levels/channels here when native selection needs them.
//!                 let selection = Selection::new(Target::new(image));
//!                 let receipt = document.decode(&selection, &DecodeSpec::new(), sink)?;
//!                 assert_eq!(receipt.region, receipt.full_bounds);
//!             }
//!         }
//!     }
//!     Ok(())
//! }
//! ```

use crate::{
    DriveOptions, ReadAt,
    drive::{Next, Window},
    read::read_next,
};
use zencodec::{
    ErrorKind, InputRequest, Result, SharedPulse, check,
    decode::{DecodeEvent, DecodeSpec, Sink},
    document::{Answer, Document, DocumentCodec, OpenSpec, Query, QueryEvent, Selected, Selection},
    error_msg,
};

/// The source remains bound to its document across browsing and ROI requests.
/// Source adapters must enforce revision consistency on every actual read.
pub struct DocumentReader<S> {
    document: Option<Box<dyn Document>>,
    spec: OpenSpec,
    origin: zencodec::document::Origin,
    source: S,
    options: DriveOptions,
}

impl<S: ReadAt> DocumentReader<S> {
    /// Open before any source read. An unknown native workspace refuses a cap.
    pub fn open(
        codec: &dyn DocumentCodec,
        source: S,
        spec: OpenSpec,
        options: DriveOptions,
    ) -> Result<Self> {
        check(&spec.pulse)?;
        spec.limits.check_memory(zencodec::Bound::Unknown)?;
        let document = codec.document(&spec)?;
        check(&spec.pulse)?;
        let options = bound_options(options, spec.limits.max_input_bytes);
        let origin = document.origin().clone();
        Ok(Self {
            document: Some(document),
            spec,
            origin,
            source,
            options,
        })
    }

    /// Inspect transport statistics without replacing the bound source.
    pub fn source(&self) -> &S {
        &self.source
    }

    /// Whether the owner remains usable after a successful or refused request.
    pub fn is_open(&self) -> bool {
        self.document.is_some()
    }

    fn check_retained(&mut self, pulse: &SharedPulse) -> Result<()> {
        if let Err(error) = check(pulse) {
            self.document = None;
            return Err(error);
        }
        Ok(())
    }

    /// Execute one metadata request. No pixel decoder or sink is constructed.
    pub fn query(&mut self, query: Query) -> Result<Answer> {
        self.document.as_ref().ok_or_else(finished)?;
        let pulse = self.spec.pulse.clone();
        self.check_retained(&pulse)?;
        query.validate(&self.origin)?;
        let mut document = self.document.take().expect("checked above");
        if let Err(error) = document.start(query.clone()) {
            if matches!(error.error().kind(), ErrorKind::Unsupported(_)) && check(&pulse).is_ok() {
                self.document = Some(document);
            }
            return Err(error);
        }
        check(&pulse)?;
        let mut window = Window::new(&self.options);
        let mut stalled = None;
        loop {
            check(&pulse)?;
            let requests = document.requested();
            let request = requests.first().copied().unwrap_or(InputRequest::Len);
            match window.next_read(requests)? {
                Next::Ready => {}
                next => {
                    read_next(&mut window, &mut self.source, next)?;
                    check(&pulse)?;
                    continue;
                }
            }
            let input = window.input(&request);
            let view = (request, input.offset, input.bytes.len());
            if stalled == Some(view) {
                return Err(error_msg(
                    ErrorKind::Protocol,
                    "document requested the same insufficient input",
                ));
            }
            let step = document.step(input)?;
            check(&pulse)?;
            if step.consumed > view.2 {
                return Err(error_msg(
                    ErrorKind::Protocol,
                    "document consumed more input than supplied",
                ));
            }
            window.consume(&request, step.consumed);
            stalled =
                (matches!(step.event, QueryEvent::NeedInput) && step.consumed == 0).then_some(view);
            match step.event {
                QueryEvent::Complete(answer) => {
                    query.validate_answer(&self.origin, &answer)?;
                    check(&pulse)?;
                    self.document = Some(document);
                    return Ok(answer);
                }
                QueryEvent::Continue => std::thread::yield_now(),
                QueryEvent::NeedInput => {}
            }
        }
    }

    /// Decode one exact selection through the existing byte requests and sink.
    /// The receipt preserves native coordinates after this document is dropped.
    pub fn decode(
        &mut self,
        selection: &Selection,
        spec: &DecodeSpec,
        sink: &mut dyn Sink,
    ) -> Result<Selected> {
        self.document.as_ref().ok_or_else(finished)?;
        let open_pulse = self.spec.pulse.clone();
        self.check_retained(&open_pulse)?;
        self.check_retained(&spec.pulse)?;
        selection.validate(&self.origin)?;
        let spec = self.spec.bind_decode(spec)?;
        spec.limits.check_memory(zencodec::Bound::Unknown)?;
        spec.limits.check_frames(1)?;
        let Answer::Description(description) =
            self.query(Query::Describe(selection.target.clone()))?
        else {
            return Err(error_msg(
                ErrorKind::Protocol,
                "description query returned another answer",
            ));
        };
        description
            .bounds
            .admit(selection.region.unwrap_or(description.bounds))?;
        spec.limits
            .check_dimensions(description.bounds.width, description.bounds.height)?;
        let mut document = self.document.take().expect("description completed");
        let prepared = match document.decoder(selection, &spec) {
            Ok(prepared) => prepared,
            Err(error) => {
                if matches!(error.error().kind(), ErrorKind::Unsupported(_))
                    && check(&spec.pulse).is_ok()
                {
                    self.document = Some(document);
                }
                return Err(error);
            }
        };
        check(&spec.pulse)?;
        prepared.selected.validate(selection, spec.limits)?;
        if prepared.selected.full_bounds != description.bounds
            || prepared.selected.registration != description.registration
        {
            return Err(error_msg(
                ErrorKind::Protocol,
                "prepared selection changed its description",
            ));
        }
        let mut decoder = prepared.decoder;
        let mut window = Window::new(&bound_options(self.options, spec.limits.max_input_bytes));
        let mut adopted = false;
        let mut publication = Publication::default();
        let mut stalled = None;
        loop {
            check_both(&open_pulse, &spec.pulse)?;
            let requests = decoder.requested();
            let request = requests.first().copied().unwrap_or(InputRequest::Len);
            match window.next_read(requests)? {
                Next::Ready => {}
                next => {
                    read_next(&mut window, &mut self.source, next)?;
                    check_both(&open_pulse, &spec.pulse)?;
                    continue;
                }
            }
            let input = window.input(&request);
            let view = (request, input.offset, input.bytes.len());
            if stalled == Some(view) {
                return Err(error_msg(
                    ErrorKind::Protocol,
                    "selected decoder requested the same insufficient input",
                ));
            }
            let mut guard = SelectionSink {
                inner: sink,
                selected: &prepared.selected,
                admitted: adopted,
                pulse: &spec.pulse,
                publication: &mut publication,
                failure: None,
            };
            let result = decoder.step(input, &mut guard);
            if let Some(error) = guard.failure {
                return Err(error);
            }
            let step = result?;
            check_both(&open_pulse, &spec.pulse)?;
            if step.consumed > view.2 {
                return Err(error_msg(
                    ErrorKind::Protocol,
                    "selected decoder consumed more input than supplied",
                ));
            }
            window.consume(&request, step.consumed);
            stalled = (step.event == DecodeEvent::NeedInput && step.consumed == 0).then_some(view);
            if step.event == DecodeEvent::Info {
                if adopted {
                    return Err(error_msg(
                        ErrorKind::Protocol,
                        "selected decoder repeated Info",
                    ));
                }
                let plan = decoder.plan().ok_or_else(|| {
                    error_msg(ErrorKind::Protocol, "selected decoder supplied no plan")
                })?;
                if plan.raster.width != prepared.selected.region.width
                    || plan.raster.height != prepared.selected.region.height
                {
                    return Err(error_msg(
                        ErrorKind::Protocol,
                        "selected plan changed region dimensions",
                    ));
                }
                if (plan.raster.layout.is_planar() && !spec.planar)
                    || (spec.accept.is_required()
                        && plan
                            .raster
                            .descriptor()
                            .is_none_or(|descriptor| !spec.accept.formats().contains(&descriptor)))
                {
                    return Err(error_msg(
                        ErrorKind::Protocol,
                        "selected plan violates output requirements",
                    ));
                }
                let info = decoder.info().ok_or_else(|| {
                    error_msg(ErrorKind::Protocol, "selected decoder supplied no info")
                })?;
                if info.width != prepared.selected.full_bounds.width
                    || info.height != prepared.selected.full_bounds.height
                {
                    return Err(error_msg(
                        ErrorKind::Protocol,
                        "selected source dimensions changed",
                    ));
                }
                if !info.settled.contains(spec.metadata_first) {
                    return Err(error_msg(
                        ErrorKind::Protocol,
                        "selected metadata is not settled",
                    ));
                }
                publication.prepare(plan.raster.clone())?;
                adopted = true;
            }
            if step.event == DecodeEvent::Done {
                if !adopted || !publication.ended {
                    return Err(error_msg(
                        ErrorKind::Protocol,
                        "selected decoder completed before the selected frame finished",
                    ));
                }
                self.document = Some(document);
                return Ok(prepared.selected);
            }
            std::thread::yield_now();
        }
    }
}

fn finished() -> zencodec::Error {
    error_msg(ErrorKind::Protocol, "document session is terminal")
}
fn check_both(a: &SharedPulse, b: &SharedPulse) -> Result<()> {
    check(a)?;
    check(b)
}
fn bound_options(mut options: DriveOptions, limit: Option<u64>) -> DriveOptions {
    if let Some(limit) = limit {
        options.max_input_bytes = Some(options.max_input_bytes.map_or(limit, |old| old.min(limit)));
    }
    options
}

#[derive(Default)]
struct Publication {
    raster: Option<zencodec::Raster>,
    coverage: Vec<u8>,
    covered: u32,
    begun: bool,
    ended: bool,
}

impl Publication {
    fn prepare(&mut self, raster: zencodec::Raster) -> Result<()> {
        let bytes = usize::try_from(u64::from(raster.height).div_ceil(8))
            .map_err(|_| error_msg(ErrorKind::OutOfMemory, "coverage size overflow"))?;
        self.coverage
            .try_reserve_exact(bytes)
            .map_err(|_| error_msg(ErrorKind::OutOfMemory, "coverage allocation failed"))?;
        self.coverage.resize(bytes, 0);
        self.raster = Some(raster);
        Ok(())
    }
    fn mark(&mut self, y: u32, count: u32, pass: zencodec::Pass) -> Result<()> {
        let raster = self.raster.as_ref().expect("admitted before publication");
        if !self.begun
            || self.ended
            || count == 0
            || y.checked_add(count).is_none_or(|end| end > raster.height)
        {
            return Err(error_msg(
                ErrorKind::Protocol,
                "rows outside the selected frame",
            ));
        }
        if pass == zencodec::Pass::Final {
            for row in y..y + count {
                let byte = &mut self.coverage[row as usize / 8];
                let mask = 1 << (row % 8);
                if *byte & mask == 0 {
                    *byte |= mask;
                    self.covered += 1;
                }
            }
        }
        Ok(())
    }
}

struct SelectionSink<'a> {
    inner: &'a mut dyn Sink,
    selected: &'a Selected,
    admitted: bool,
    pulse: &'a SharedPulse,
    publication: &'a mut Publication,
    failure: Option<zencodec::Error>,
}

impl SelectionSink<'_> {
    fn retain<T>(&mut self, result: Result<T>) -> Result<T> {
        match result {
            Ok(value) => Ok(value),
            Err(error) => {
                if self.failure.is_none() {
                    self.failure = Some(error);
                }
                Err(error_msg(ErrorKind::Sink, "selected sink refused"))
            }
        }
    }
    fn admit(&mut self) -> Result<()> {
        let result = if self.failure.is_some() || !self.admitted || self.publication.ended {
            Err(error_msg(
                ErrorKind::Protocol,
                "sink outside selected publication",
            ))
        } else {
            check(self.pulse)
        };
        self.retain(result)
    }
}

impl Sink for SelectionSink<'_> {
    fn begin(&mut self, frame: &zencodec::FrameHeader) -> Result<()> {
        self.admit()?;
        let result = if self.publication.begun
            || Some(&frame.raster) != self.publication.raster.as_ref()
            || frame.raster.width != self.selected.region.width
            || frame.raster.height != self.selected.region.height
        {
            Err(error_msg(
                ErrorKind::Protocol,
                "sink frame changed selected interpretation",
            ))
        } else {
            Ok(())
        };
        self.retain(result)?;
        self.publication.begun = true;
        let result = self.inner.begin(frame);
        self.retain(result)?;
        self.retain(check(self.pulse))
    }
    fn lend(
        &mut self,
        y: u32,
        rows: u32,
        pass: zencodec::Pass,
    ) -> Result<zenpixels::PixelSliceMut<'_>> {
        self.admit()?;
        match self.inner.lend(y, rows, pass) {
            Ok(lent) => {
                let raster = self.publication.raster.as_ref().unwrap();
                let result = if lent.width() != raster.width
                    || Some(lent.descriptor()) != raster.descriptor()
                    || lent.rows() > rows
                {
                    Err(error_msg(
                        ErrorKind::Protocol,
                        "sink loan differs from selected raster",
                    ))
                } else {
                    self.publication
                        .mark(y, lent.rows(), pass)
                        .and_then(|()| check(self.pulse))
                };
                match result {
                    Ok(()) => Ok(lent),
                    Err(error) => {
                        self.failure = Some(error);
                        Err(error_msg(ErrorKind::Sink, "selected loan refused"))
                    }
                }
            }
            Err(error) => {
                self.failure = Some(error);
                Err(error_msg(ErrorKind::Sink, "selected loan refused"))
            }
        }
    }
    fn rows(
        &mut self,
        y: u32,
        rows: zenpixels::PixelSlice<'_>,
        pass: zencodec::Pass,
    ) -> Result<()> {
        self.admit()?;
        let raster = self.publication.raster.as_ref().unwrap();
        let result =
            if rows.width() != raster.width || Some(rows.descriptor()) != raster.descriptor() {
                Err(error_msg(
                    ErrorKind::Protocol,
                    "rows differ from selected raster",
                ))
            } else {
                self.publication.mark(y, rows.rows(), pass)
            };
        self.retain(result)?;
        let result = self.inner.rows(y, rows, pass);
        self.retain(result)?;
        self.retain(check(self.pulse))
    }
    fn frame(&mut self, pixels: zenpixels::PixelBuffer) -> Result<()> {
        self.admit()?;
        let raster = self.publication.raster.as_ref().unwrap();
        let result = if pixels.width() != raster.width
            || pixels.height() != raster.height
            || Some(pixels.descriptor()) != raster.descriptor()
            || pixels.color_context().map(|context| &**context) != raster.color.as_deref()
        {
            Err(error_msg(
                ErrorKind::Protocol,
                "owned frame differs from selected interpretation",
            ))
        } else {
            self.publication
                .mark(0, pixels.height(), zencodec::Pass::Final)
        };
        self.retain(result)?;
        let result = self.inner.frame(pixels);
        self.retain(result)?;
        self.retain(check(self.pulse))
    }
    fn planes(&mut self, y: u32, strip: zenpixels::plane::PlanarSlice<'_>) -> Result<()> {
        self.admit()?;
        let raster = self.publication.raster.as_ref().unwrap();
        let matches = match &raster.layout {
            zencodec::Layout::Planar { planes, alpha } => {
                strip.width() == raster.width
                    && **planes == *strip.layout()
                    && *alpha == strip.alpha_mode()
                    && raster.color.as_deref() == Some(strip.color_context())
            }
            _ => false,
        };
        let result = if matches {
            self.publication
                .mark(y, strip.height(), zencodec::Pass::Final)
        } else {
            Err(error_msg(
                ErrorKind::Protocol,
                "planes differ from selected interpretation",
            ))
        };
        self.retain(result)?;
        let result = self.inner.planes(y, strip);
        self.retain(result)?;
        self.retain(check(self.pulse))
    }
    fn end(&mut self) -> Result<()> {
        self.admit()?;
        let result =
            if self.publication.covered == self.selected.region.height && self.publication.begun {
                Ok(())
            } else {
                Err(error_msg(
                    ErrorKind::Protocol,
                    "selected frame is missing final rows",
                ))
            };
        self.retain(result)?;
        self.publication.ended = true;
        let result = self.inner.end();
        self.retain(result)?;
        self.retain(check(self.pulse))
    }
}
