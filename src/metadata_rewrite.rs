//! Shared contracts for metadata-only rewrites of encoded images.
//!
//! Codec crates own container validation and output planning. These types do not
//! require zenmeta, zencodecs, zenpipe, XML, compression or pixel decoding.
use crate::{LimitKind, MetadataPolicy};
use alloc::{borrow::Cow, vec::Vec};
use core::ops::Range;

/// A failure produces no output plan and leaves the input untouched.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// Invalid container framing or metadata.
    Malformed(&'static str),
    /// Preserving the rendering contract requires an unsupported operation.
    Unsupported(&'static str),
    /// A configured resource budget was exceeded.
    Limit(LimitKind),
    /// An allocation failed.
    Allocation,
    /// An explicitly injected service refused its input.
    Service(&'static str),
    /// Cooperative cancellation was requested.
    Cancelled,
    /// A deadline or another cooperative pulse reason.
    Stopped(enough::StopReason),
}
impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "metadata rewrite: {self:?}")
    }
}
impl core::error::Error for Error {}

/// Compression of metadata, never image samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Compression {
    /// PNG iCCP / compressed text.
    Zlib,
    /// JPEG XL brob metadata boxes.
    Brotli,
}

/// Optional runtime services, supplied at the application boundary.
///
/// No XML, Brotli, deflate, or CMS implementation becomes a dependency of this
/// module. Implementations must enforce `limit` *during* parsing/decompression,
/// honor cancellation, and bound intermediate allocations. Returned byte lengths
/// are checked again by the caller, but that cannot bound a service's own memory.
/// Services never receive pixel buffers. Unknown ICC profiles must return `None`.
pub trait Services {
    /// Merge already-filtered descriptive XMP with required output signaling.
    /// Preserve the required packet's properties exactly. The codec validates
    /// gain-map parameters and discovery after this callback; retaining arbitrary
    /// private properties remains the application's explicit policy decision.
    fn merge_xmp(
        &self,
        _descriptive: &[u8],
        _required: &[u8],
        _limit: usize,
        _pulse: Option<&dyn crate::Pulse>,
    ) -> Result<Vec<u8>, Error> {
        Err(Error::Unsupported("XMP merge service not supplied"))
    }
    /// Rewrite selected XMP properties and return serialized XML, or remove it.
    /// The application owns retention semantics; preserve unknown properties only
    /// when explicitly intended. Default refuses, rather than preserving a packet.
    fn rewrite_xmp(
        &self,
        _xml: &[u8],
        _limit: usize,
        _pulse: Option<&dyn crate::Pulse>,
    ) -> Result<Option<Vec<u8>>, Error> {
        Err(Error::Unsupported("XMP editing service not supplied"))
    }
    /// Replace only a recognized, rendering-equivalent ICC profile.
    /// `None` means unrecognized/unchanged, not removal. No arbitrary sanitization.
    fn normalize_icc(
        &self,
        _icc: &[u8],
        _limit: usize,
        _pulse: Option<&dyn crate::Pulse>,
    ) -> Result<Option<Vec<u8>>, Error> {
        Ok(None)
    }
    /// Decompress a metadata packet, bounding expansion while producing output.
    fn decompress(
        &self,
        _kind: Compression,
        _data: &[u8],
        _limit: usize,
        _pulse: Option<&dyn crate::Pulse>,
    ) -> Result<Vec<u8>, Error> {
        Err(Error::Unsupported(
            "metadata decompression service not supplied",
        ))
    }
    /// Compress metadata after an explicit edit/normalization.
    fn compress(
        &self,
        _kind: Compression,
        _data: &[u8],
        _limit: usize,
        _pulse: Option<&dyn crate::Pulse>,
    ) -> Result<Vec<u8>, Error> {
        Err(Error::Unsupported(
            "metadata compression service not supplied",
        ))
    }
}

/// Disposition of an input carrier. Retaining ICC never certifies its privacy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Action {
    /// Retain the original carrier bytes.
    Keep,
    /// Omit the input carrier.
    Remove,
    /// Serialize filtered metadata or generate a replacement carrier.
    Rewrite,
    /// Retain profile contents without certifying their privacy.
    OpaqueIcc,
    /// Explicitly relinquish byte-exact JPEG reconstruction.
    RemoveJpegReconstruction,
    /// Preserve an image codestream without decoding or certifying it.
    OpaqueImageData,
}

/// One complete input carrier, including its framing bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    range: Range<usize>,
    carrier: &'static str,
    action: Action,
}
impl Entry {
    /// Describe an input carrier and its disposition in the output plan.
    pub fn new(range: Range<usize>, carrier: &'static str, action: Action) -> Self {
        Self {
            range,
            carrier,
            action,
        }
    }

    /// Byte range in the original input. Entries include image payload carriers.
    pub fn source_range(&self) -> Range<usize> {
        self.range.clone()
    }
    /// Stable human-readable carrier category.
    pub fn carrier(&self) -> &'static str {
        self.carrier
    }
    /// What the plan does to this carrier.
    pub fn action(&self) -> Action {
        self.action
    }
}

/// Codec-produced rewrite result. Chunks borrow unmodified encoded data.
/// Structural report bounds are checked here; container correctness and policy
/// enforcement belong to the planner, typically backed by zenmeta.
#[derive(Debug)]
pub struct Plan<'a> {
    parts: Vec<Cow<'a, [u8]>>,
    entries: Vec<Entry>,
    size: usize,
}
impl<'a> Plan<'a> {
    /// Assemble a codec-validated plan without copying its payload chunks.
    ///
    /// The codec must validate container framing and enforce its request budgets.
    /// This constructor checks output-length arithmetic and report ranges only.
    pub fn from_parts(
        input_len: usize,
        parts: Vec<Cow<'a, [u8]>>,
        entries: Vec<Entry>,
    ) -> Result<Self, Error> {
        if entries
            .iter()
            .any(|entry| entry.range.start > entry.range.end || entry.range.end > input_len)
        {
            return Err(Error::Malformed("invalid report range"));
        }
        let size = parts
            .iter()
            .try_fold(0usize, |size, part| size.checked_add(part.len()))
            .ok_or(Error::Limit(LimitKind::OutputSize))?;
        Ok(Self {
            parts,
            entries,
            size,
        })
    }
    /// Every input carrier, including removed data and opaque retained profiles.
    pub fn report(&self) -> &[Entry] {
        &self.entries
    }
    /// Complete output in order. Use `try_for_each(|b| writer.write_all(b))`.
    /// A sink failure may leave partial output in the caller's sink.
    pub fn chunks(&self) -> impl Iterator<Item = &[u8]> {
        self.parts.iter().map(|p| p.as_ref())
    }
    /// Exact encoded output length, without allocating or writing output.
    pub fn output_len(&self) -> usize {
        self.size
    }
    /// Explicitly allocate and collect the encoded output once.
    pub fn to_vec(&self) -> Result<Vec<u8>, Error> {
        let mut out = Vec::new();
        out.try_reserve_exact(self.size)
            .map_err(|_| Error::Allocation)?;
        for p in self.chunks() {
            out.extend_from_slice(p);
        }
        Ok(out)
    }
}

/// Metadata-only request. Defaults: remove descriptive metadata, preserve color
/// and orientation, 512 MiB input/output, 16 MiB metadata, 65,536 carriers.
/// Unsupported formats fail rather than falling back to decoding/re-encoding.
#[derive(Clone)]
pub struct RewriteRequest<'a> {
    input: &'a [u8],
    policy: MetadataPolicy,
    services: Option<&'a dyn Services>,
    edit_xmp: bool,
    normalize_icc: bool,
    remove_reconstruction: bool,
    max_input: usize,
    max_output: usize,
    max_metadata: usize,
    max_carriers: usize,
    pulse: Option<&'a dyn crate::Pulse>,
}
/// Legacy ambiguous name for a metadata rewrite request.
#[deprecated(note = "use RewriteRequest")]
pub type Request<'a> = RewriteRequest<'a>;

impl<'a> RewriteRequest<'a> {
    /// Borrow encoded input. No parsing or allocation occurs yet.
    pub fn new(input: &'a [u8]) -> Self {
        Self {
            input,
            policy: MetadataPolicy::ColorAndRotation,
            services: None,
            edit_xmp: false,
            normalize_icc: false,
            remove_reconstruction: false,
            max_input: 512 << 20,
            max_output: 512 << 20,
            max_metadata: 16 << 20,
            max_carriers: 65_536,
            pulse: None,
        }
    }
    /// Metadata retention. Retained XMP is copied unless an editor is requested.
    /// Dropping rendering dependencies is refused; it needs a pixel operation.
    pub fn with_policy(mut self, policy: MetadataPolicy) -> Self {
        self.policy = policy;
        self
    }
    /// Inject optional packet editing/compression services at runtime.
    pub fn with_services(mut self, services: &'a dyn Services) -> Self {
        self.services = Some(services);
        self
    }
    /// Explicitly replace the ordinary XMP disposition with the editor's result.
    pub fn with_xmp_editing(mut self, enabled: bool) -> Self {
        self.edit_xmp = enabled;
        self
    }
    /// Invoke a known-profile normalizer; unknown profiles remain unchanged.
    pub fn with_icc_normalization(mut self, enabled: bool) -> Self {
        self.normalize_icc = enabled;
        self
    }
    /// Allow removal of JXL's JPEG reconstruction data. Original JPEG byte-exact
    /// reconstruction is then unavailable; the JXL image codestream is unchanged.
    pub fn with_jpeg_reconstruction_removal(mut self, enabled: bool) -> Self {
        self.remove_reconstruction = enabled;
        self
    }
    /// Bound encoded input and output independently.
    pub fn with_byte_limits(mut self, input: usize, output: usize) -> Self {
        self.max_input = input;
        self.max_output = output;
        self
    }
    /// Bound each parsed/expanded packet and total regenerated metadata.
    pub fn with_metadata_limit(mut self, bytes: usize) -> Self {
        self.max_metadata = bytes;
        self
    }
    /// Bound container carrier count (including image-data chunks).
    pub fn with_carrier_limit(mut self, count: usize) -> Self {
        self.max_carriers = count;
        self
    }
    /// Cooperative cancellation, checked between carriers and long byte walks.
    pub fn with_pulse(mut self, pulse: Option<&'a dyn crate::Pulse>) -> Self {
        self.pulse = pulse;
        self
    }
    /// Encoded source bytes.
    pub fn input(&self) -> &'a [u8] {
        self.input
    }
    /// Retention policy.
    pub fn policy(&self) -> &MetadataPolicy {
        &self.policy
    }
    /// Optional application services.
    pub fn services(&self) -> Option<&'a dyn Services> {
        self.services
    }
    /// Whether descriptive XMP editing is enabled.
    pub fn edit_xmp(&self) -> bool {
        self.edit_xmp
    }
    /// Whether known ICC normalization is enabled.
    pub fn normalize_icc(&self) -> bool {
        self.normalize_icc
    }
    /// Whether JPEG reconstruction data may be removed.
    pub fn remove_reconstruction(&self) -> bool {
        self.remove_reconstruction
    }
    /// Maximum input bytes.
    pub fn max_input(&self) -> usize {
        self.max_input
    }
    /// Maximum output bytes.
    pub fn max_output(&self) -> usize {
        self.max_output
    }
    /// Maximum packet and regenerated metadata bytes.
    pub fn max_metadata(&self) -> usize {
        self.max_metadata
    }
    /// Maximum reported carriers.
    pub fn max_carriers(&self) -> usize {
        self.max_carriers
    }
    /// Caller pulse token.
    pub fn pulse(&self) -> Option<&'a dyn crate::Pulse> {
        self.pulse
    }
    /// Reuse the same policy, limits and services for a contained image.
    pub fn with_input(mut self, input: &'a [u8]) -> Self {
        self.input = input;
        self
    }
}
