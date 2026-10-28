//! Retained-job executor. Raw-encoder entry points keep their legacy ordering.
use super::*;
use zencodec::{
    encode::{EncodeRows, OutputPhase, PullProgress, RowDemand, RowFillError, SourcePasses},
    operation::{
        EncodeEvidence, EncodeRouteOffer, InputAcquisition, ReplayEvidence, resolve_encode_route,
    },
};

pub(super) struct RawDisk {
    pub(super) file: File,
    pub(super) origin: u64,
    pub(super) max_bytes: u64,
}

/// Generic EncodeJob/DynEncodeJob expose neither format nor fidelity.
/// Successful setters and carrier facts do not certify metadata treatment.
fn unknown_job_evidence() -> EncodeEvidence {
    EncodeEvidence::unknown()
}

enum Input<'a, S: Source> {
    Raster {
        pixels: PixelSlice<'a>,
        source: &'a S,
    },
    Source(&'a mut S),
}
impl<S: Source> Input<'_, S> {
    fn source(&self) -> &S {
        match self {
            Self::Raster { source, .. } => source,
            Self::Source(source) => source,
        }
    }
    fn validate(
        &self,
        desc: &Description,
        mechanism: OutputSource,
        positional: bool,
    ) -> Result<(), Cause> {
        validate_source(self.source(), desc, mechanism)?;
        if self.source().supports_positional_replay() != positional {
            return Err(Refusal::SourceChanged.into());
        }
        if let Self::Raster { pixels, .. } = self
            && (pixels.width() != desc.width
                || pixels.rows() != desc.height
                || pixels.descriptor() != desc.descriptor
                || pixels.color_context() != desc.color_context.as_ref()
                || pixels
                    .exact_depth()
                    .is_some_and(|claim| Some(claim) != desc.exact_depth))
        {
            return Err(Refusal::SourceChanged.into());
        }
        Ok(())
    }
    fn read(
        &mut self,
        first: u32,
        mut rows: PixelSliceMut<'_>,
        pulse: &dyn zencodec::Pulse,
    ) -> Result<(), Cause> {
        match self {
            Self::Raster { pixels, .. } => {
                for y in 0..rows.rows() {
                    check(pulse)?;
                    rows.row_mut(y).copy_from_slice(pixels.row(first + y));
                }
                Ok(())
            }
            Self::Source(source) => source
                .read_rows(first, rows, pulse)
                .map_err(|e| Box::new(e) as Cause),
        }
    }
}

// Same digest as the qualified experiment. This detects changes; it is neither
// a cryptographic commitment nor evidence of sample-exact immutability.
fn digest(bytes: &[u8]) -> u64 {
    const K: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut h = (bytes.len() as u64).wrapping_mul(K);
    let (words, rest) = bytes.as_chunks::<8>();
    for word in words {
        h = (h ^ u64::from_le_bytes(*word))
            .wrapping_mul(K)
            .rotate_left(29);
    }
    let mut tail = [0; 8];
    tail[..rest.len()].copy_from_slice(rest);
    (h ^ u64::from_le_bytes(tail)).wrapping_mul(K) ^ (h >> 31)
}

enum RawStore {
    None,
    Memory(OwnedStorage<u8>),
    Disk(RawDisk),
}
struct Filler<'a, S: Source> {
    input: Input<'a, S>,
    desc: &'a Description,
    mechanism: OutputSource,
    positional: bool,
    row_bytes: usize,
    store: RawStore,
    hashes: Option<OwnedStorage<u64>>,
    identity: Option<[u64; 4]>,
    declared: SourcePasses,
    pass: Option<u32>,
    final_started: bool,
    next_row: u32,
    fills: u8,
    failure: Option<(Stage, Cause)>,
    boundary: Stage,
    pulse: &'a dyn zencodec::Pulse,
}
impl<S: Source> Filler<'_, S> {
    fn validate(&self) -> Result<(), Cause> {
        self.input
            .validate(self.desc, self.mechanism, self.positional)?;
        if let Some(identity) = self.identity
            && self.input.source().replay_identity()? != Some(identity)
        {
            return Err(Refusal::ReplayChanged {
                pass: self.pass.unwrap_or(0),
                first: self.next_row,
            }
            .into());
        }
        Ok(())
    }
    fn begin_pass(&mut self, pass: u32, final_pass: bool) -> Result<(), Cause> {
        match self.pass {
            None if pass == 0 => {}
            Some(previous)
                if previous.checked_add(1) == Some(pass) && self.next_row == self.desc.height => {}
            _ => return Err(Refusal::RequestOrder { pass, first: 0 }.into()),
        }
        let count = pass.checked_add(1).ok_or(Refusal::Overflow)?;
        if self.final_started || self.declared.max().is_some_and(|max| count > max) {
            return Err(Refusal::PassLimit { pass }.into());
        }
        if final_pass && count < self.declared.min() {
            return Err(Refusal::IncompletePass { pass }.into());
        }
        self.pass = Some(pass);
        self.final_started = final_pass;
        self.next_row = 0;
        self.validate()?;
        if final_pass {
            let _ = self.pulse.set_total(zencodec::how_far::Total::Exact(
                u64::from(count) * u64::from(self.desc.height),
            ));
            check(self.pulse)?;
            self.validate()?;
        }
        Ok(())
    }
    fn fill_inner(&mut self, demand: RowDemand<'_>) -> Result<(), Cause> {
        self.boundary = Stage::Read;
        let RowDemand {
            pass,
            final_pass,
            first,
            mut rows,
            ..
        } = demand;
        check(self.pulse)?;
        let count = rows.rows();
        if rows.width() != self.desc.width
            || rows.descriptor() != self.desc.descriptor
            || rows.color_context() != self.desc.color_context.as_ref()
            || count == 0
            || first
                .checked_add(count)
                .is_none_or(|end| end > self.desc.height)
        {
            return Err(Refusal::RequestOrder { pass, first }.into());
        }
        if self.pass != Some(pass) {
            self.begin_pass(pass, final_pass)?;
        } else if final_pass != self.final_started {
            return Err(Refusal::RequestOrder { pass, first }.into());
        }
        if first != self.next_row {
            return Err(Refusal::RequestOrder { pass, first }.into());
        }
        self.validate()?;
        let external = pass == 0 || matches!(self.store, RawStore::None);
        if external {
            let stride = rows.stride();
            let mut destination = PixelSliceMut::new(
                rows.as_strided_bytes_mut(),
                self.desc.width,
                count,
                stride,
                self.desc.descriptor,
            )?;
            if let Some(context) = &self.desc.color_context {
                destination = destination.with_color_context(context.clone());
            }
            self.input.read(first, destination, self.pulse)?;
            check(self.pulse)?;
            self.validate()?;
        }
        for y in 0..count {
            self.boundary = if self.desc.exact_depth.is_some() {
                Stage::Encode
            } else {
                Stage::Read
            };
            check(self.pulse)?;
            let at = (first + y) as usize;
            let row = rows.row_mut(y);
            let offset = at.checked_mul(self.row_bytes).ok_or(Refusal::Overflow)?;
            match &mut self.store {
                RawStore::Memory(store) if external => {
                    store.as_mut_slice()[offset..][..self.row_bytes].copy_from_slice(row)
                }
                RawStore::Memory(store) => {
                    row.copy_from_slice(&store.as_slice()[offset..][..self.row_bytes])
                }
                RawStore::Disk(store) => {
                    let offset = store
                        .origin
                        .checked_add(offset as u64)
                        .ok_or(Refusal::Overflow)?;
                    check(self.pulse)?;
                    store.file.seek(SeekFrom::Start(offset))?;
                    check(self.pulse)?;
                    if external {
                        let mut written = 0;
                        write_all(&mut store.file, row, &mut written, self.pulse)?;
                    } else {
                        let mut remaining = &mut row[..];
                        while !remaining.is_empty() {
                            check(self.pulse)?;
                            match store.file.read(remaining) {
                                Ok(0) => {
                                    return Err(
                                        io::Error::from(io::ErrorKind::UnexpectedEof).into()
                                    );
                                }
                                Ok(n) => remaining = &mut remaining[n..],
                                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                                Err(e) => return Err(e.into()),
                            }
                        }
                    }
                }
                RawStore::None => {}
            }
            if let Some(exact) = self.desc.exact_depth {
                let pixels = PixelSlice::new(
                    row,
                    self.desc.width,
                    1,
                    self.row_bytes,
                    self.desc.descriptor,
                )?;
                if !exact.holds_for(&pixels)? {
                    return Err(ExactDepthContradiction {
                        first_row: first + y,
                        claim: exact,
                    }
                    .into());
                }
            }
            if let Some(hashes) = &mut self.hashes {
                let hash = digest(row);
                if pass == 0 {
                    hashes.as_mut_slice()[at] = hash;
                } else if hashes.as_slice()[at] != hash {
                    return Err(Refusal::ReplayChanged {
                        pass,
                        first: first + y,
                    }
                    .into());
                }
            }
        }
        self.boundary = Stage::Read;
        check(self.pulse)?;
        self.validate()?;
        self.pulse.advance(u64::from(count));
        check(self.pulse)?;
        self.validate()?;
        self.next_row = first + count;
        Ok(())
    }
    fn complete(&self) -> Result<(), Cause> {
        let count = self.pass.and_then(|p| p.checked_add(1)).unwrap_or(0);
        if count < self.declared.min()
            || self.declared.max().is_some_and(|max| count > max)
            || self.next_row != self.desc.height
            || !self.final_started
        {
            return Err(Refusal::IncompletePass {
                pass: self.pass.unwrap_or(0),
            }
            .into());
        }
        self.validate()
    }
}
impl<S: Source> EncodeRows for Filler<'_, S> {
    fn fill(&mut self, demand: RowDemand<'_>) -> Result<(), RowFillError> {
        if self.failure.is_some() {
            return Err(RowFillError);
        }
        self.fills = self.fills.saturating_add(1);
        if self.fills != 1 {
            self.failure = Some((Stage::Read, Refusal::PullProtocol.into()));
            return Err(RowFillError);
        }
        match self.fill_inner(demand) {
            Ok(()) => Ok(()),
            Err(error) => {
                self.failure = Some((self.boundary, error));
                Err(RowFillError)
            }
        }
    }
}

struct Delivery<D> {
    writer: D,
    options: Options,
    schedule: OutputSchedule,
    staged: VecDeque<OwnedStorage<u8>>,
    produced: u64,
    emitted: u64,
    patched: u64,
    boundary: Stage,
}
impl<D: Destination> Delivery<D> {
    fn drain<E: Encoder>(
        &mut self,
        encoder: &mut E,
        pulse: &dyn zencodec::Pulse,
    ) -> Result<(), Cause> {
        self.boundary = Stage::Encode;
        let pending = encoder.pending_output();
        count(
            &mut self.produced,
            pending.len(),
            self.options.max_output_bytes,
        )?;
        if self.schedule == OutputSchedule::Live {
            self.boundary = Stage::Write;
            write_all(&mut self.writer, pending, &mut self.emitted, pulse)?;
        } else if let Some(disk) = &mut self.options.disk {
            disk.origin
                .checked_add(self.produced)
                .ok_or(Refusal::Overflow)?;
            disk.append(pending, pulse)?;
        } else {
            stage(pending, &self.options.storage, &mut self.staged)?;
        }
        encoder.consume_output(pending.len());
        Ok(())
    }
    fn finish<E: Encoder>(
        &mut self,
        encoder: E,
        timing: zencodec::encode::OutputTiming,
        pulse: &dyn zencodec::Pulse,
    ) -> Result<(), Cause> {
        self.boundary = Stage::Finish;
        check(pulse)?;
        let finished = encoder.finish_output()?;
        check(pulse)?;
        finished.validate(self.produced)?;
        if !finished.patches.is_empty() && (!self.writer.can_patch() || !timing.patches_at_finish())
        {
            return Err(Refusal::Patches.into());
        }
        let patches = finished.patches;
        let rest = finished.tail;
        count(
            &mut self.produced,
            rest.len(),
            self.options.max_output_bytes,
        )?;
        if self.schedule == OutputSchedule::FinishFirst {
            if let Some(disk) = &mut self.options.disk {
                disk.origin
                    .checked_add(self.produced)
                    .ok_or(Refusal::Overflow)?;
                disk.append(rest.data(), pulse)?;
                drop(rest);
                disk.replay(
                    self.produced,
                    &self.options.storage,
                    &mut self.writer,
                    &mut self.emitted,
                    &mut self.boundary,
                    pulse,
                )?;
            } else {
                stage(rest.data(), &self.options.storage, &mut self.staged)?;
                drop(rest);
                self.boundary = Stage::Write;
                while let Some(bytes) = self.staged.pop_front() {
                    write_all(&mut self.writer, bytes.as_slice(), &mut self.emitted, pulse)?;
                }
            }
        } else {
            self.boundary = Stage::Write;
            write_all(&mut self.writer, rest.data(), &mut self.emitted, pulse)?;
        }
        check(pulse)?;
        if !patches.is_empty() {
            self.boundary = Stage::Patch;
            for patch in patches {
                check(pulse)?;
                self.writer.position(patch.offset)?;
                write_all(&mut self.writer, &patch.bytes, &mut self.patched, pulse)?;
            }
            check(pulse)?;
            self.writer.position(self.produced)?;
        }
        self.boundary = Stage::Write;
        check(pulse)?;
        self.writer.flush()?;
        check(pulse)
    }
}

pub(super) fn run<P: RowStartup, S: Source, D: Destination, R: OperationPolicy>(
    mut startup: P,
    mut source: S,
    writer: D,
    mut resolver: R,
    options: Options,
    pulse: &dyn zencodec::Pulse,
) -> Result<Completed, At<Failure>> {
    let mut out = Delivery {
        writer,
        options,
        schedule: OutputSchedule::FinishFirst,
        staged: VecDeque::new(),
        produced: 0,
        emitted: 0,
        patched: 0,
        boundary: Stage::Prepare,
    };
    let mut report = OperationReport::default();
    let bound_requirements = resolver.requirements();
    let entry_requirements = out.options.preset;
    let run = || -> Result<(), Cause> {
        check(pulse)?;
        out.boundary = Stage::Policy;
        entry_requirements.check_workspace(None)?;
        if let Some(required) = bound_requirements {
            required.check_workspace(None)?;
        }
        out.boundary = Stage::Prepare;
        let desc = source.description();
        let mechanism = source.mechanism();
        let positional = source.supports_positional_replay();
        if desc.width == 0 || desc.height == 0 {
            return Err(Refusal::Empty.into());
        }
        let row_bytes = (desc.width as usize)
            .checked_mul(desc.descriptor.bytes_per_pixel())
            .ok_or(Refusal::Overflow)?;
        let raw_bytes = row_bytes
            .checked_mul(desc.height as usize)
            .ok_or(Refusal::Overflow)?;
        let access = if out.writer.can_patch() {
            OutputAccess::SeekBack
        } else {
            OutputAccess::Forward
        };
        let mut declaration =
            EncodeDeclaration::new(desc.width, desc.height, desc.descriptor).with_access(access);
        if let Some(color) = &desc.color_context {
            declaration = declaration.with_color(color);
        }
        if let Some(exact) = desc.exact_depth {
            declaration = declaration.with_exact_depth(exact);
        }
        declaration.validate_exact_depth()?;
        let plan = startup.plan(&declaration)?;
        let declared = match plan.input {
            InputRequirement::Rows => SourcePasses::ONCE,
            InputRequirement::Replay(passes) => passes,
            _ => return Err(EncodePlanError::InputProtocol.into()),
        };
        let single = declared.max() == Some(1);
        // The temporary probe only chooses a borrow-checker branch. The second
        // returned view is the actual storage retained and used for all reads.
        let input = if source.borrowed_pixels().is_some() {
            let pixels = source.borrowed_pixels().ok_or(Refusal::SourceChanged)?;
            Input::Raster {
                pixels,
                source: &source,
            }
        } else {
            Input::Source(&mut source)
        };
        input.validate(&desc, mechanism, positional)?;
        let borrowed = matches!(input, Input::Raster { .. });
        let identity = if !single && !borrowed && positional {
            input.source().replay_identity()?
        } else {
            None
        };
        check(pulse)?;
        input.validate(&desc, mechanism, positional)?;
        let batch_rows = out.options.batch_rows.get().min(desc.height);
        let mut reservation_error: Option<Cause> = None;
        // Probe mutually exclusive reservations independently. A callback may
        // change the budget; re-admit and allocate only the selected route
        // afterwards, still before source reads or native construction.
        let disk_eligible = !single
            && !borrowed
            && out.options.raw_disk.as_ref().is_some_and(|disk| {
                raw_bytes as u64 <= disk.max_bytes
                    && disk.origin.checked_add(raw_bytes as u64).is_some()
            });
        let hash_available = if !single && !borrowed && (positional || disk_eligible) {
            match out
                .options
                .storage
                .prepare_filled(desc.height as usize, 0u64)
            {
                Ok(_) => true,
                Err(error) => {
                    reservation_error = Some(error.into());
                    false
                }
            }
        } else {
            false
        };
        let memory_available = if !single && !borrowed {
            match out.options.storage.prepare_filled(raw_bytes, 0u8) {
                Ok(_) => true,
                Err(error) => {
                    if reservation_error.is_none() {
                        reservation_error = Some(error.into());
                    }
                    false
                }
            }
        } else {
            false
        };
        let mut finish = OutputScheduleOffer::new(OutputSchedule::FinishFirst);
        finish.staging = Some(StagingFacts::new(
            if out.options.disk.is_some() {
                StagingKind::Disk
            } else {
                StagingKind::Memory
            },
            0,
            if out.options.disk.is_some() {
                out.options.max_output_bytes
            } else {
                out.options.storage.max_bytes() as u64
            },
        ));
        let schedules = [finish, OutputScheduleOffer::new(OutputSchedule::Live)];
        let mut offer_slots = [EncodeRouteOffer::new(InputAcquisition::Unknown, finish); 6];
        let mut offer_count = 0;
        let mut add = |acquisition, replay| {
            for output in &schedules[..if plan.timing.streams() { 2 } else { 1 }] {
                let mut offer = EncodeRouteOffer::new(acquisition, *output);
                offer.replay = replay;
                offer_slots[offer_count] = offer;
                offer_count += 1;
            }
        };
        if borrowed {
            add(
                InputAcquisition::BorrowedRaster,
                ReplayEvidence::ImmutableBorrow,
            );
        } else if single || (positional && hash_available) {
            add(
                InputAcquisition::DirectRows,
                if single {
                    ReplayEvidence::Unknown
                } else {
                    ReplayEvidence::VerifiedHash
                },
            );
        }
        if disk_eligible && hash_available {
            add(InputAcquisition::DiskReplay, ReplayEvidence::VerifiedHash);
        }
        if memory_available {
            add(
                InputAcquisition::MemorySnapshot,
                ReplayEvidence::PrivateSnapshot,
            );
        }
        let offers = &offer_slots[..offer_count];
        if offers.is_empty() {
            return Err(reservation_error.unwrap_or_else(|| Refusal::NoInputRoute.into()));
        }
        out.boundary = Stage::Policy;
        let (selected, record) = match resolve_encode_route(
            &declaration,
            &plan,
            unknown_job_evidence(),
            offers,
            &mut resolver,
            entry_requirements,
            pulse,
        ) {
            Ok(selected) => selected,
            Err(error) => {
                report.record(error.error().record());
                return Err(error.into());
            }
        };
        report.record(record);
        let selected = offers[selected];
        check(pulse)?;
        out.boundary = Stage::Read;
        input.validate(&desc, mechanism, positional)?;
        if (if out.writer.can_patch() {
            OutputAccess::SeekBack
        } else {
            OutputAccess::Forward
        }) != access
        {
            return Err(Refusal::DestinationChanged.into());
        }
        out.boundary = Stage::Prepare;
        let store = match selected.acquisition {
            InputAcquisition::MemorySnapshot => {
                RawStore::Memory(out.options.storage.bytes(raw_bytes)?)
            }
            InputAcquisition::DiskReplay => {
                RawStore::Disk(out.options.raw_disk.take().ok_or(Refusal::NoInputRoute)?)
            }
            _ => RawStore::None,
        };
        drop(out.options.raw_disk.take());
        let digest_storage = if matches!(
            selected.replay,
            ReplayEvidence::VerifiedHash | ReplayEvidence::VerifiedIdentity
        ) {
            Some(
                out.options
                    .storage
                    .prepare_filled(desc.height as usize, 0u64)?
                    .allocate()?,
            )
        } else {
            None
        };
        let mut row_buffer = if plan.input == InputRequirement::Rows {
            Some(
                out.options.storage.bytes(
                    row_bytes
                        .checked_mul(batch_rows as usize)
                        .ok_or(Refusal::Overflow)?,
                )?,
            )
        } else {
            None
        };
        let identity = if selected.acquisition == InputAcquisition::DirectRows {
            identity
        } else {
            None
        };
        let mut filler = Filler {
            input,
            desc: &desc,
            mechanism,
            positional,
            row_bytes,
            store,
            hashes: digest_storage,
            identity,
            declared,
            pass: None,
            final_started: false,
            next_row: 0,
            fills: 0,
            failure: None,
            boundary: Stage::Read,
            pulse,
        };
        filler.validate()?;
        let total = match declared.max() {
            Some(max) if max == declared.min() => {
                zencodec::how_far::Total::Exact(u64::from(max) * u64::from(desc.height))
            }
            _ => zencodec::how_far::Total::Unknown,
        };
        let _ = pulse.set_total(total);
        check(pulse)?;
        filler.validate()?;
        out.boundary = Stage::Start;
        let mut encoder = startup.start(&declaration, &plan)?;
        out.schedule = selected.output.schedule;
        let timing = plan.timing;
        let input_contract = plan.input;
        let phase = plan.output_phase;
        drop(plan);
        loop {
            check(pulse)?;
            out.boundary = Stage::Read;
            filler.validate()?;
            if input_contract == InputRequirement::Rows && filler.next_row == desc.height {
                break;
            }
            if matches!(input_contract, InputRequirement::Replay(_))
                && phase == OutputPhase::FinalPass
                && !filler.final_started
                && !encoder.pending_output().is_empty()
            {
                return Err(Refusal::EarlyOutput { pass: filler.pass }.into());
            }
            out.drain(&mut encoder, pulse)?;
            out.boundary = Stage::Encode;
            filler.fills = 0;
            if input_contract == InputRequirement::Rows {
                let first = filler.next_row;
                let count = batch_rows.min(desc.height - first);
                let size = row_bytes
                    .checked_mul(count as usize)
                    .ok_or(Refusal::Overflow)?;
                let buffer = row_buffer.as_mut().ok_or(Refusal::PullProtocol)?;
                let mut rows = PixelSliceMut::new(
                    &mut buffer.as_mut_slice()[..size],
                    desc.width,
                    count,
                    row_bytes,
                    desc.descriptor,
                )?;
                if let Some(context) = &desc.color_context {
                    rows = rows.with_color_context(context.clone());
                }
                let _ = filler.fill(RowDemand::new(0, first, rows).with_final_pass(true));
                if let Some((stage, cause)) = filler.failure.take() {
                    out.boundary = stage;
                    return Err(cause);
                }
                let mut pixels = PixelSlice::new(
                    &buffer.as_slice()[..size],
                    desc.width,
                    count,
                    row_bytes,
                    desc.descriptor,
                )?;
                if let Some(exact) = desc.exact_depth {
                    pixels = pixels.with_exact_depth(exact)?;
                }
                if let Some(context) = &desc.color_context {
                    pixels = pixels.with_color_context(context.clone());
                }
                encoder.push_rows(pixels)?;
            } else {
                let progress = encoder.pull_rows(&mut filler, pulse);
                // A source/control/verification cause wins even if native code
                // reports Filled/Done or a different error after failed fill.
                if let Some((stage, cause)) = filler.failure.take() {
                    out.boundary = stage;
                    return Err(cause);
                }
                match progress? {
                    PullProgress::Filled if filler.fills == 1 => {}
                    PullProgress::Done if filler.fills == 0 => {
                        filler.complete()?;
                        break;
                    }
                    _ => return Err(Refusal::PullProtocol.into()),
                }
            }
        }
        out.boundary = Stage::Read;
        filler.complete()?;
        drop(filler);
        drop(source);
        drop(row_buffer);
        drop(desc);
        out.finish(encoder, timing, pulse)
    };
    match run() {
        Ok(()) => Ok(Completed {
            emitted: out.emitted,
            patched_bytes: out.patched,
            schedule: out.schedule,
            report,
        }),
        Err(cause) => Err(At::wrap(Failure {
            stage: out.boundary,
            emitted: out.emitted,
            patched_bytes: out.patched,
            report: Box::new(report),
            cause,
        })),
    }
}
