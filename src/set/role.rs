//! Typed multi-provider factory role registry for [`CodecSet`](super::CodecSet).
//!
//! `zencodec` owns the generic role storage (`FactoryRole`, `NamedFactory`,
//! `RegistrationId`, `RegistrationError`) and the still-image/encoded-transform roles,
//! while downstream crates such as `zencodec-media` define their own roles (`DemuxRole`,
//! `MuxRole`, `VideoDecodeRole`, `VideoEncodeRole`) without any dependency cycle.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::any::{Any, TypeId};
use core::fmt;

use crate::format::ImageFormat;
use crate::traits::{BoxedError, DynDecoderConfig, DynEncoderConfig};

/// Maximum length of a [`RegistrationId`] in ASCII bytes.
pub const MAX_REGISTRATION_ID_LEN: usize = 128;

/// Validated, bounded reverse-DNS-style ASCII namespace identifier (e.g.
/// `"org.imazen.jpeg.zenjpeg"`, `"org.imazen.mp4.zenmp4"`, `"org.imazen.av1.rav1d"`).
///
/// Rules:
/// - Length `1..=128` ASCII bytes.
/// - One or more dot-separated non-empty labels (single-label names and underscores
///   are permitted); no leading, trailing, or consecutive `.`.
/// - Each label starts with `[a-z0-9]` and contains only `[a-z0-9_-]`.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RegistrationId(Box<str>);

impl RegistrationId {
    /// Validate and construct a [`RegistrationId`].
    pub fn new(id: &str) -> Result<Self, RegistrationError> {
        if id.is_empty() || id.len() > MAX_REGISTRATION_ID_LEN {
            return Err(RegistrationError::InvalidId);
        }
        for label in id.split('.') {
            let bytes = label.as_bytes();
            if bytes.is_empty() {
                return Err(RegistrationError::InvalidId);
            }
            if !matches!(bytes[0], b'a'..=b'z' | b'0'..=b'9') {
                return Err(RegistrationError::InvalidId);
            }
            if !bytes
                .iter()
                .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_'))
            {
                return Err(RegistrationError::InvalidId);
            }
        }
        Ok(Self(String::from(id).into_boxed_str()))
    }

    /// Borrow the identifier string.
    #[inline]
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for RegistrationId {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Debug for RegistrationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("RegistrationId").field(&self.0).finish()
    }
}

impl fmt::Display for RegistrationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Failure when validating or inserting a named provider into [`CodecSet`](super::CodecSet).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RegistrationError {
    /// Identifier is empty, exceeds 128 bytes, or violates reverse-DNS ASCII label rules.
    InvalidId,
    /// Another factory with the same [`RegistrationId`] is already registered in this role.
    DuplicateId(RegistrationId),
    /// A role registration supplied no keys.
    EmptyKeys,
    /// A registration key is not supported by the supplied provider (e.g. registering
    /// a PNG codec under `ImageFormat::Jpeg`).
    UnsupportedKey,
}

impl fmt::Display for RegistrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidId => f.write_str(
                "invalid registration id (expected 1..=128 ASCII reverse-DNS [a-z0-9._-])",
            ),
            Self::DuplicateId(id) => write!(f, "duplicate registration id `{id}` in role"),
            Self::EmptyKeys => f.write_str("role registration must declare at least one key"),
            Self::UnsupportedKey => {
                f.write_str("role registration key is not declared supported by the provider")
            }
        }
    }
}

impl core::error::Error for RegistrationError {}

/// Marker trait defining a typed factory slot in [`CodecSet`](super::CodecSet).
pub trait FactoryRole: Send + Sync + 'static {
    /// Lookup key for this role (e.g. `ImageFormat`, `ContainerId`, `CodecId`).
    type Key: Clone + Eq + Send + Sync + 'static;
    /// Object-safe factory trait stored behind `Arc<Self::Factory>`.
    type Factory: ?Sized + Send + Sync + 'static;
}

/// One named, versioned factory registration for role `R`.
pub struct NamedFactory<R: FactoryRole> {
    /// Stable reverse-DNS provider identifier.
    pub id: RegistrationId,
    /// Provider revision string included in route cache keys and reports.
    pub revision: &'static str,
    /// Immutable shareable factory instance.
    pub factory: Arc<R::Factory>,
}

impl<R: FactoryRole> NamedFactory<R> {
    /// Construct a named factory entry.
    #[inline]
    #[must_use]
    pub fn new(id: RegistrationId, revision: &'static str, factory: Arc<R::Factory>) -> Self {
        Self {
            id,
            revision,
            factory,
        }
    }

    /// Parse `id` and construct a named factory entry.
    pub fn try_new(
        id: &str,
        revision: &'static str,
        factory: Arc<R::Factory>,
    ) -> Result<Self, RegistrationError> {
        Ok(Self::new(RegistrationId::new(id)?, revision, factory))
    }
}

impl<R: FactoryRole> Clone for NamedFactory<R> {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            revision: self.revision,
            factory: Arc::clone(&self.factory),
        }
    }
}

impl<R: FactoryRole> fmt::Debug for NamedFactory<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NamedFactory")
            .field("id", &self.id)
            .field("revision", &self.revision)
            .finish_non_exhaustive()
    }
}

/// Typed role for named still/animated image decoders keyed by [`ImageFormat`].
pub enum ImageDecodeRole {}
impl FactoryRole for ImageDecodeRole {
    type Key = ImageFormat;
    type Factory = dyn DynDecoderConfig;
}

/// Object-safe factory for a registered still/animated image encoder that preserves
/// both its [`DynEncoderConfig`] metadata/local-job entry point and whether it can
/// construct a [`Send`] encode job for this specific provider.
pub trait ImageEncodeFactory: Send + Sync {
    /// Borrow the underlying object-safe encoder config template.
    fn config(&self) -> &dyn DynEncoderConfig;

    /// Whether [`send_job`](Self::send_job) is supported for this registered provider.
    fn supports_send_job(&self) -> bool {
        false
    }

    /// Construct a `Send` encode job (with optional checked [`Fidelity`](crate::encode::Fidelity))
    /// for this specific registered provider, or return
    /// [`CodecSetError::EncoderJobNotSend`](super::CodecSetError::EncoderJobNotSend)
    /// when registered without `Send`-job support.
    fn send_job(
        &self,
        fidelity: Option<crate::encode::Fidelity>,
    ) -> Result<Box<dyn crate::traits::DynEncodeJob + Send>, super::CodecSetError> {
        let _ = fidelity;
        Err(super::CodecSetError::EncoderJobNotSend(
            self.config().format(),
        ))
    }
}

/// Typed role for named still/animated image encoders keyed by [`ImageFormat`].
pub enum ImageEncodeRole {}
impl FactoryRole for ImageEncodeRole {
    type Key = ImageFormat;
    type Factory = dyn ImageEncodeFactory;
}

/// Provisional provider contract for lossless/encoded-domain transforms (e.g.,
/// lossless JPEG orientation/crop or container packet remuxing) between
/// `(source_format, target_format)`.
///
/// This trait is provisional and will be extended to carry a prepared effect offer
/// and the run's [`JobResources`](crate::JobResources) controls before stabilization.
pub trait EncodedTransformFactory: Send + Sync {
    /// Human-readable transform capability summary.
    fn describe(&self) -> &'static str;

    /// Apply a lossless encoded-domain byte transform when supported.
    fn transform_bytes(
        &self,
        input: &[u8],
        pulse: Option<&dyn crate::Pulse>,
    ) -> Result<Vec<u8>, BoxedError>;
}

/// Typed role for encoded-domain transforms keyed by `(source_format, target_format)`.
pub enum EncodedTransformRole {}
impl FactoryRole for EncodedTransformRole {
    type Key = (ImageFormat, ImageFormat);
    type Factory = dyn EncodedTransformFactory;
}

/// Internal copy-on-write per-role table stored type-erased inside `CodecSet`.
pub(super) struct RoleTable<R: FactoryRole> {
    pub(super) entries: Vec<(R::Key, NamedFactory<R>)>,
    pub(super) ids: Vec<RegistrationId>,
}

impl<R: FactoryRole> Clone for RoleTable<R> {
    fn clone(&self) -> Self {
        Self {
            entries: self.entries.clone(),
            ids: self.ids.clone(),
        }
    }
}

impl<R: FactoryRole> Default for RoleTable<R> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            ids: Vec::new(),
        }
    }
}

#[derive(Clone, Default)]
pub(super) struct RoleStore {
    tables: Vec<(TypeId, Arc<dyn Any + Send + Sync>)>,
}

impl RoleStore {
    pub(super) fn insert<R: FactoryRole>(
        &mut self,
        keys: impl IntoIterator<Item = R::Key>,
        entry: NamedFactory<R>,
    ) -> Result<(), RegistrationError> {
        let mut collected: Vec<R::Key> = Vec::new();
        for key in keys {
            if !collected.contains(&key) {
                collected.push(key);
            }
        }
        if collected.is_empty() {
            return Err(RegistrationError::EmptyKeys);
        }
        let tid = TypeId::of::<R>();
        if let Some((_, slot)) = self.tables.iter_mut().find(|(t, _)| *t == tid) {
            let existing = slot
                .downcast_ref::<RoleTable<R>>()
                .expect("RoleStore TypeId invariant");
            if existing.ids.contains(&entry.id) {
                return Err(RegistrationError::DuplicateId(entry.id));
            }
            let mut cloned = existing.clone();
            cloned.ids.push(entry.id.clone());
            for key in collected {
                cloned.entries.push((key, entry.clone()));
            }
            *slot = Arc::new(cloned);
        } else {
            let mut table = RoleTable::<R>::default();
            table.ids.push(entry.id.clone());
            for key in collected {
                table.entries.push((key, entry.clone()));
            }
            self.tables.push((tid, Arc::new(table)));
        }
        Ok(())
    }

    pub(super) fn table<R: FactoryRole>(&self) -> Option<&RoleTable<R>> {
        let tid = TypeId::of::<R>();
        self.tables
            .iter()
            .find(|(t, _)| *t == tid)
            .and_then(|(_, slot)| slot.downcast_ref::<RoleTable<R>>())
    }
}
