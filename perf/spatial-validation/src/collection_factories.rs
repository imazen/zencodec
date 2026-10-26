//! Native factories validate the shared CodecSet contract without adding native
//! dependencies to zencodec. Codec-specific limit templates remain explicit.
use crate::{tiff_items::TiffCollection, tiff_region::Allowances};
use std::{error::Error, fmt};
pub use zencodec::collection::{CollectionFactory as Factory, CollectionOpenError as FactoryError};
use zencodec::decode::DecodePolicy;
use zencodec::{
    At, ImageFormat, ResourceLimits,
    collection::{Cause, CollectionOpenContext, ErasedCollection},
    source::{DynReadAt, ReadAt},
};
use zencodec_io::ReadAtReader;
pub use zenjxl::collection::JxlCollectionConfig as JxlFactory;

// Only the explicitly qualified fields may differ from the no-op configuration.
// Comparing the remainder also refuses future fields until they are qualified.
#[track_caller]
fn require_configuration(
    format: ImageFormat,
    supported: ImageFormat,
    limits: Option<ResourceLimits>,
    policy: Option<DecodePolicy>,
) -> Result<(), Cause> {
    if format != supported {
        return Err(At::wrap(FactoryError::NotRegistered(format)).into());
    }
    if let Some(mut remaining) = limits {
        // The shared field is explicitly encode-only; opening performs no encode.
        remaining.max_output_bytes = None;
        if supported == ImageFormat::Tiff {
            remaining.max_width = None;
            remaining.max_height = None;
            remaining.max_pixels = None;
            // The pinned TIFF path is sequential, including its decompressors.
            if remaining.threading == zencodec::ThreadingPolicy::Sequential {
                remaining.threading = zencodec::ThreadingPolicy::Parallel;
            }
        }
        if remaining != ResourceLimits::none() {
            return Err(At::wrap(FactoryError::UnsupportedLimits {
                unsupported: remaining,
            })
            .into());
        }
    }
    if let Some(unsupported) = policy
        && unsupported != DecodePolicy::none()
    {
        return Err(At::wrap(FactoryError::UnsupportedDecodePolicy { unsupported }).into());
    }
    Ok(())
}

#[derive(Clone, Copy)]
pub struct TiffFactory(pub Allowances);
impl Factory for TiffFactory {
    fn formats(&self) -> &[ImageFormat] {
        &[ImageFormat::Tiff]
    }
    fn open_with<'a>(
        &self,
        source: &'a mut dyn DynReadAt,
        context: CollectionOpenContext,
    ) -> Result<ErasedCollection<'a>, Cause> {
        let CollectionOpenContext {
            format,
            view,
            limits,
            policy,
            stop,
            output_storage,
            ..
        } = context;
        if output_storage.is_some() {
            return Err(At::wrap(FactoryError::UnsupportedOutputStorage).into());
        }
        require_configuration(format, ImageFormat::Tiff, limits, policy)?;
        Ok(ErasedCollection::new(
            TiffCollection::open_with_stop(
                ReadAtReader::new(ParserSource(source)),
                view,
                self.0,
                stop,
            )?
            .with_required_dimensions(limits),
        ))
    }
}

// Bridge errors retain the original concrete type and any existing At trace.
// Default formatting deliberately does not include source payloads.
struct SourceFailure(Cause);
impl fmt::Debug for SourceFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SourceFailure { .. }")
    }
}
impl fmt::Display for SourceFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("positioned source failed")
    }
}
impl Error for SourceFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.0.as_ref())
    }
}
struct ParserSource<'a>(&'a mut dyn DynReadAt);
impl ReadAt for ParserSource<'_> {
    type Error = SourceFailure;
    fn len(&mut self) -> Result<u64, Self::Error> {
        self.0.len().map_err(SourceFailure)
    }
    fn len_hint(&self) -> Option<u64> {
        self.0.len_hint()
    }
    fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<usize, Self::Error> {
        self.0.read_at(offset, out).map_err(SourceFailure)
    }
    fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), Self::Error> {
        self.0.read_exact_at(offset, out).map_err(SourceFailure)
    }
    fn read_many_at(&mut self, requests: &mut [(u64, &mut [u8])]) -> Result<(), Self::Error> {
        self.0.read_many_at(requests).map_err(SourceFailure)
    }
    fn as_slice(&self) -> Option<&[u8]> {
        self.0.as_slice()
    }
}
