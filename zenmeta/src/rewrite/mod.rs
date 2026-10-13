//! Common bounded packet processing for codec-owned metadata rewrite planners.
//! Public codec signatures use zencodec's request and plan types; these helpers
//! remain implementation details that can evolve with zenmeta independently.
use alloc::{borrow::Cow, vec::Vec};
use core::ops::Range;
use zencodec::{
    Orientation, enough,
    metadata_rewrite::{Action, Compression, Entry, Error, Plan, Request, Services},
};
mod exif;

/// Shared request validation and metadata packet processing.
#[derive(Clone)]
pub struct Context<'a> {
    request: Request<'a>,
}
impl<'a> core::ops::Deref for Context<'a> {
    type Target = Request<'a>;
    fn deref(&self) -> &Self::Target {
        &self.request
    }
}
impl<'a> Context<'a> {
    /// Validate a metadata-only request before invoking a container walker.
    pub fn new(request: &Request<'a>) -> Result<Self, Error> {
        let context = Self {
            request: request.clone(),
        };
        context.check()?;
        if (request.edit_xmp() || request.normalize_icc()) && request.services().is_none() {
            return Err(Error::Unsupported("metadata service not supplied"));
        }
        Ok(context)
    }
    /// Poll cancellation while preserving the caller's stop reason.
    pub fn check(&self) -> Result<(), Error> {
        self.stop().check().map_err(|reason| match reason {
            enough::StopReason::Cancelled => Error::Cancelled,
            other => Error::Stopped(other),
        })?;
        if self.input().len() > self.max_input() {
            return Err(Error::Limit(zencodec::LimitKind::InputSize));
        }
        Ok(())
    }
    /// Enforce the parsed/expanded packet or regenerated metadata budget.
    pub fn packet(&self, size: usize) -> Result<(), Error> {
        self.check()?;
        if size > self.max_metadata() {
            Err(Error::Limit(zencodec::LimitKind::MetadataSize))
        } else {
            Ok(())
        }
    }
    /// Reuse the parent policy and limits for a contained image.
    pub fn with_input(mut self, input: &'a [u8]) -> Self {
        self.request = self.request.with_input(input);
        self
    }
    /// Override descriptive XMP handling for a contained image.
    pub fn with_xmp_editing(mut self, enabled: bool) -> Self {
        self.request = self.request.with_xmp_editing(enabled);
        self
    }
    /// Require an application service provider.
    pub fn services(&self) -> Result<&dyn Services, Error> {
        self.request
            .services()
            .ok_or(Error::Unsupported("metadata service not supplied"))
    }
    /// Apply explicitly enabled descriptive XMP editing within packet limits.
    pub fn xmp(&self, xml: &[u8]) -> Result<Option<Vec<u8>>, Error> {
        self.packet(xml.len())?;
        if !self.edit_xmp() {
            self.check()?;
            return if self.policy().fields().xmp.keeps() {
                let mut kept = Vec::new();
                kept.try_reserve_exact(xml.len())
                    .map_err(|_| Error::Allocation)?;
                kept.extend_from_slice(xml);
                Ok(Some(kept))
            } else {
                Ok(None)
            };
        }
        let out = self
            .services()?
            .rewrite_xmp(xml, self.max_metadata(), self.stop())?;
        if let Some(ref bytes) = out {
            self.packet(bytes.len())?;
        }
        self.check()?;
        Ok(out)
    }
    /// Normalize a known ICC profile while retaining rendering dependencies.
    pub fn icc(&self, bytes: &[u8]) -> Result<Option<Vec<u8>>, Error> {
        self.packet(bytes.len())?;
        if matches!(self.policy().fields().icc, zencodec::IccRetention::Drop) {
            return Err(Error::Unsupported(
                "removing an ICC profile requires explicit color conversion",
            ));
        }
        if !self.normalize_icc() {
            return Ok(None);
        }
        let out = self
            .services()?
            .normalize_icc(bytes, self.max_metadata(), self.stop())?;
        if let Some(ref bytes) = out {
            self.packet(bytes.len())?;
            if bytes.len() < 128 || bytes.get(36..40) != Some(b"acsp") {
                return Err(Error::Service("normalizer returned an invalid ICC header"));
            }
        }
        self.check()?;
        Ok(out)
    }
    /// Expand a metadata packet through a bounded caller service.
    pub fn decompress(&self, kind: Compression, bytes: &[u8]) -> Result<Vec<u8>, Error> {
        self.packet(bytes.len())?;
        let out = self
            .services()?
            .decompress(kind, bytes, self.max_metadata(), self.stop())?;
        self.packet(out.len())?;
        self.check()?;
        Ok(out)
    }
    /// Validate and filter EXIF while preserving orientation.
    pub fn exif(&self, bytes: &[u8]) -> Result<Vec<u8>, Error> {
        self.packet(bytes.len())?;
        exif::validate(self, bytes)?;
        let exif = zencodec::exif::Exif::parse(bytes).ok_or(Error::Malformed("EXIF"))?;
        let orientation = exif.orientation().unwrap_or(Orientation::Identity);
        let filtered = crate::exif::filter(&exif, &self.policy().fields().exif);
        if filtered.orientation().unwrap_or(Orientation::Identity) != orientation {
            return Err(Error::Unsupported(
                "removing orientation requires explicit pixel transformation",
            ));
        }
        let output = crate::exif::serialize(&filtered);
        self.packet(output.len())?;
        Ok(output)
    }
}

/// Fallible, bounded assembly shared by codec-specific rewrite planners.
#[derive(Debug, Default)]
pub struct PlanBuilder<'a> {
    parts: Vec<Cow<'a, [u8]>>,
    entries: Vec<Entry>,
    size: usize,
    rewritten: usize,
    input: Option<&'a [u8]>,
}
impl<'a> PlanBuilder<'a> {
    /// Start an empty plan; each carrier operation checks its request budget.
    pub fn new() -> Self {
        Self::default()
    }
    /// Record one input carrier and an optional output chunk, enforcing budgets.
    pub fn record(
        &mut self,
        req: &Context<'a>,
        range: Range<usize>,
        carrier: &'static str,
        action: Action,
        bytes: Option<Cow<'a, [u8]>>,
    ) -> Result<(), Error> {
        req.check()?;
        if range.start > range.end || range.end > req.input().len() {
            return Err(Error::Malformed("rewrite source range"));
        }
        let input = req.input();
        if self
            .input
            .is_some_and(|previous| !core::ptr::eq(previous, input))
        {
            return Err(Error::Malformed("mixed rewrite inputs"));
        }
        if self.entries.len() >= req.max_carriers() {
            return Err(Error::Limit(zencodec::LimitKind::MetadataCarriers));
        }
        let added = bytes.as_ref().map_or(0, |b| b.len());
        let size = self
            .size
            .checked_add(added)
            .filter(|n| *n <= req.max_output())
            .ok_or(Error::Limit(zencodec::LimitKind::OutputSize))?;
        let rewritten = self
            .rewritten
            .checked_add(
                bytes
                    .as_ref()
                    .filter(|b| matches!(b, Cow::Owned(_)))
                    .map_or(0, |b| b.len()),
            )
            .ok_or(Error::Limit(zencodec::LimitKind::MetadataRewriteSize))?;
        if rewritten > req.max_metadata() {
            return Err(Error::Limit(zencodec::LimitKind::MetadataRewriteSize));
        }
        self.entries.try_reserve(1).map_err(|_| Error::Allocation)?;
        if bytes.is_some() {
            self.parts.try_reserve(1).map_err(|_| Error::Allocation)?;
        }
        self.entries.push(Entry::new(range, carrier, action));
        if let Some(bytes) = bytes {
            self.parts.push(bytes);
        }
        self.size = size;
        self.rewritten = rewritten;
        self.input = Some(input);
        Ok(())
    }

    /// Retain a validated range from the encoded input.
    pub fn keep(
        &mut self,
        req: &Context<'a>,
        range: Range<usize>,
        name: &'static str,
    ) -> Result<(), Error> {
        let bytes = req
            .input()
            .get(range.clone())
            .ok_or(Error::Malformed("rewrite source range"))?;
        self.record(req, range, name, Action::Keep, Some(Cow::Borrowed(bytes)))
    }
    /// Record a removed input carrier.
    pub fn remove(
        &mut self,
        req: &Context<'a>,
        range: Range<usize>,
        name: &'static str,
    ) -> Result<(), Error> {
        self.record(req, range, name, Action::Remove, None)
    }
    /// Current encoded output size.
    pub fn output_len(&self) -> usize {
        self.size
    }
    /// Insert a generated carrier immediately after the first output chunk.
    pub fn inject_after_first(
        &mut self,
        req: &Context<'a>,
        bytes: Vec<u8>,
        name: &'static str,
    ) -> Result<(), Error> {
        if self.parts.is_empty() {
            return Err(Error::Malformed("missing leading carrier"));
        }
        self.record(req, 0..0, name, Action::Rewrite, Some(Cow::Owned(bytes)))?;
        let part = self
            .parts
            .pop()
            .ok_or(Error::Malformed("missing generated carrier"))?;
        self.parts.insert(1, part);
        Ok(())
    }
    /// Remove a previously retained carrier before inserting its replacement.
    pub fn remove_kept_range(
        &mut self,
        req: &Context<'a>,
        range: &Range<usize>,
    ) -> Result<(), Error> {
        req.check()?;
        let input = req.input();
        if self
            .input
            .is_some_and(|previous| !core::ptr::eq(previous, input))
        {
            return Err(Error::Malformed("mixed rewrite inputs"));
        }
        let source = req
            .input()
            .get(range.clone())
            .ok_or(Error::Malformed("rewrite source range"))?;
        let index = self.parts.iter().position(|p| matches!(p, Cow::Borrowed(b) if b.as_ptr() == source.as_ptr() && b.len() == source.len()))
            .ok_or(Error::Malformed("missing retained carrier"))?;
        self.size -= self.parts.remove(index).len();
        for entry in &mut self.entries {
            if entry.source_range() == *range {
                *entry = Entry::new(range.clone(), entry.carrier(), Action::Remove);
            }
        }
        Ok(())
    }
    /// Append a contained-image plan, translating its report to parent offsets.
    pub fn append(&mut self, req: &Context<'a>, other: Self, offset: usize) -> Result<(), Error> {
        req.check()?;
        let input = req.input();
        if self
            .input
            .is_some_and(|previous| !core::ptr::eq(previous, input))
        {
            return Err(Error::Malformed("mixed rewrite inputs"));
        }
        if offset
            .checked_add(other.input.map_or(0, <[u8]>::len))
            .is_none_or(|end| end > req.input().len())
        {
            return Err(Error::Malformed("contained input range"));
        }
        let size = self
            .size
            .checked_add(other.size)
            .filter(|n| *n <= req.max_output())
            .ok_or(Error::Limit(zencodec::LimitKind::OutputSize))?;
        let rewritten = self
            .rewritten
            .checked_add(other.rewritten)
            .ok_or(Error::Limit(zencodec::LimitKind::MetadataRewriteSize))?;
        if rewritten > req.max_metadata() {
            return Err(Error::Limit(zencodec::LimitKind::MetadataRewriteSize));
        }
        if self
            .entries
            .len()
            .checked_add(other.entries.len())
            .is_none_or(|n| n > req.max_carriers())
        {
            return Err(Error::Limit(zencodec::LimitKind::MetadataCarriers));
        }
        for entry in &other.entries {
            let range = entry.source_range();
            if range.start.checked_add(offset).is_none()
                || range
                    .end
                    .checked_add(offset)
                    .is_none_or(|end| end > req.input().len())
            {
                return Err(Error::Malformed("report offset"));
            }
        }
        self.parts
            .try_reserve(other.parts.len())
            .map_err(|_| Error::Allocation)?;
        self.entries
            .try_reserve(other.entries.len())
            .map_err(|_| Error::Allocation)?;
        for entry in other.entries {
            let range = entry.source_range();
            self.entries.push(Entry::new(
                range.start + offset..range.end + offset,
                entry.carrier(),
                entry.action(),
            ));
        }
        self.input = Some(input);
        self.parts.extend(other.parts);
        self.size = size;
        self.rewritten = rewritten;
        Ok(())
    }
    /// Finish with the stable zencodec-owned plan type.
    pub fn finish(self) -> Result<Plan<'a>, Error> {
        let plan = Plan::from_parts(self.input.map_or(0, <[u8]>::len), self.parts, self.entries)?;
        if plan.output_len() != self.size {
            return Err(Error::Malformed("rewrite length accounting"));
        }
        Ok(plan)
    }
}
