//! Native factories validate the shared CodecSet contract without adding native
//! dependencies to zencodec. Codec-specific limit templates remain explicit.
use crate::{
    jxl_items::{JxlItems, Limits},
    jxl_layers::JxlLayers,
    tiff_items::TiffCollection,
    tiff_region::Allowances,
};
use std::{error::Error, fmt};
pub use zencodec::collection::{CollectionFactory as Factory, CollectionOpenError as FactoryError};
use zencodec::decode::DecodePolicy;
use zencodec::{
    ImageFormat, ResourceLimits, StopToken,
    collection::{Cause, Composition, ErasedCollection, SelectionError},
    source::{DynReadAt, ReadAt},
};
use zencodec_io::ReadAtReader;

// There is no qualified native mapping of the full legacy configurations yet.
// Refuse before source I/O, rather than imply these templates enforce them.
fn require_configuration(
    format: ImageFormat,
    supported: ImageFormat,
    limits: Option<ResourceLimits>,
    policy: Option<DecodePolicy>,
) -> Result<(), FactoryError> {
    if format != supported {
        return Err(FactoryError::NotRegistered(format));
    }
    if limits.is_some() {
        return Err(FactoryError::UnsupportedLimits);
    }
    if policy.is_some() {
        return Err(FactoryError::UnsupportedDecodePolicy);
    }
    Ok(())
}

#[derive(Clone, Copy)]
pub struct JxlFactory(pub Limits);
impl Factory for JxlFactory {
    fn formats(&self) -> &[ImageFormat] {
        &[ImageFormat::Jxl]
    }
    fn open<'a>(
        &self,
        source: &'a mut dyn DynReadAt,
        format: ImageFormat,
        view: Composition,
        limits: Option<ResourceLimits>,
        policy: Option<DecodePolicy>,
        stop: StopToken,
    ) -> Result<ErasedCollection<'a>, Cause> {
        require_configuration(format, ImageFormat::Jxl, limits, policy)?;
        let bytes = DynReadAt::as_slice(source).ok_or(FactoryError::ResidentInputRequired)?;
        match view {
            Composition::Composited => Ok(ErasedCollection::new(JxlItems::open_with_stop(
                bytes, view, self.0, stop,
            )?)),
            Composition::Uncomposited => {
                Ok(ErasedCollection::new(JxlLayers::open(bytes, self.0, stop)?))
            }
            _ => Err(Box::new(SelectionError::UnsupportedComposition)),
        }
    }
}

#[derive(Clone, Copy)]
pub struct TiffFactory(pub Allowances);
impl Factory for TiffFactory {
    fn formats(&self) -> &[ImageFormat] {
        &[ImageFormat::Tiff]
    }
    fn open<'a>(
        &self,
        source: &'a mut dyn DynReadAt,
        format: ImageFormat,
        view: Composition,
        limits: Option<ResourceLimits>,
        policy: Option<DecodePolicy>,
        stop: StopToken,
    ) -> Result<ErasedCollection<'a>, Cause> {
        require_configuration(format, ImageFormat::Tiff, limits, policy)?;
        Ok(ErasedCollection::new(TiffCollection::open_with_stop(
            ReadAtReader::new(ParserSource(source)),
            view,
            self.0,
            stop,
        )?))
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
