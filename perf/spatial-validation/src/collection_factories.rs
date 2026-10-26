//! Executable registry candidate, before choosing a public CodecSet extension.
//! No implicit full-source buffering, transport ownership or codec dependencies
//! are added to the core traits crate by this experiment.
use crate::{
    erased_collection::ErasedCollection,
    jxl_items::{JxlItems, Limits},
    jxl_layers::JxlLayers,
    local_source::LocalSource,
    tiff_items::TiffCollection,
    tiff_region::Allowances,
};
use std::{error::Error, fmt, sync::Arc};
use zencodec::{
    At, ImageFormat, StopToken,
    collection::{Cause, Collection, Composition, Discovery, SelectionError},
    operation::{CheckedCollection, CollectionOperationError, PolicyResolver},
    source::ReadAt,
};
use zencodec_io::ReadAtReader;

pub trait Factory: Send + Sync {
    fn formats(&self) -> &[ImageFormat];
    fn open<'a>(
        &self,
        source: &'a mut dyn LocalSource,
        view: Composition,
        stop: StopToken,
    ) -> Result<ErasedCollection<'a>, Cause>;
}

#[derive(Clone, Default)]
pub struct Factories(Vec<Arc<dyn Factory>>);
impl Factories {
    pub fn with(mut self, factory: impl Factory + 'static) -> Self {
        self.0.push(Arc::new(factory));
        self
    }
    pub fn open<'a, R: PolicyResolver>(
        &self,
        format: ImageFormat,
        source: &'a mut dyn LocalSource,
        request: OpenRequest,
        resolver: R,
        stop: StopToken,
    ) -> Result<CheckedCollection<Discovery<ErasedCollection<'a>>, R>, At<CollectionOperationError>>
    {
        CheckedCollection::open(stop, resolver, |stop| {
            let factory = self
                .0
                .iter()
                .find(|f| f.formats().contains(&format))
                .ok_or(FactoryError::NotRegistered(format))?;
            let collection = factory.open(source, request.view, stop)?;
            if collection.view() != request.view {
                return Err(Box::new(SelectionError::UnsupportedComposition));
            }
            Ok(Discovery::new(collection, request.discovery_attempts))
        })
    }
}

#[derive(Clone, Copy)]
pub struct OpenRequest {
    pub view: Composition,
    pub discovery_attempts: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FactoryError {
    NotRegistered(ImageFormat),
    ResidentInputRequired,
}
impl fmt::Display for FactoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotRegistered(_) => f.write_str("no collection factory registered for format"),
            Self::ResidentInputRequired => {
                f.write_str("this native collection requires already-resident input")
            }
        }
    }
}
impl Error for FactoryError {}

#[derive(Clone, Copy)]
pub struct JxlFactory(pub Limits);
impl Factory for JxlFactory {
    fn formats(&self) -> &[ImageFormat] {
        &[ImageFormat::Jxl]
    }
    fn open<'a>(
        &self,
        source: &'a mut dyn LocalSource,
        view: Composition,
        stop: StopToken,
    ) -> Result<ErasedCollection<'a>, Cause> {
        let bytes = LocalSource::as_slice(source).ok_or(FactoryError::ResidentInputRequired)?;
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
        source: &'a mut dyn LocalSource,
        view: Composition,
        stop: StopToken,
    ) -> Result<ErasedCollection<'a>, Cause> {
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
struct ParserSource<'a>(&'a mut dyn LocalSource);
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
