//! EXIF rewrite entry points using zencodec's interchange types.
use alloc::{borrow::Cow, vec::Vec};
use zencodec::{Exif, ExifPolicy};

/// Filter a parsed EXIF tree. Opaque offset-bearing metadata is not relocated.
pub fn filter<'a>(exif: &Exif<'a>, policy: &ExifPolicy) -> Exif<'a> {
    exif.filtered(policy)
}

/// Serialize a parsed EXIF tree. This is not byte-exact preservation.
pub fn serialize(exif: &Exif<'_>) -> Vec<u8> {
    exif.to_bytes()
}

/// Retain EXIF under a policy; KEEP_ALL borrows the original bytes unchanged.
pub fn retain<'a>(bytes: &'a [u8], policy: &ExifPolicy) -> Option<Cow<'a, [u8]>> {
    zencodec::exif::retain(bytes, policy)
}
