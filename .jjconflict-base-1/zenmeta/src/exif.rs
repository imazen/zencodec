//! EXIF rewrite entry points using zencodec's published interchange types.
//!
//! Reading remains available through `zencodec::Exif` without zenmeta. These
//! functions delegate to the retained compatibility implementation, avoiding a
//! duplicate parser/writer until the next breaking zencodec release.
use alloc::{borrow::Cow, vec::Vec};
use zencodec::{Exif, ExifPolicy};

/// Filter a parsed EXIF tree. Opaque offset-bearing metadata is not relocated.
#[allow(deprecated)]
pub fn filter<'a>(exif: &Exif<'a>, policy: &ExifPolicy) -> Exif<'a> {
    exif.filtered(policy)
}

/// Serialize a parsed EXIF tree. This is not byte-exact preservation.
#[allow(deprecated)]
pub fn serialize(exif: &Exif<'_>) -> Vec<u8> {
    exif.to_bytes()
}

/// Retain EXIF under a policy; KEEP_ALL borrows the original bytes unchanged.
#[allow(deprecated)]
pub fn retain<'a>(bytes: &'a [u8], policy: &ExifPolicy) -> Option<Cow<'a, [u8]>> {
    zencodec::exif::retain(bytes, policy)
}
