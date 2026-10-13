//! Retention implementation entry points for already extracted metadata.
//!
//! These preserve the published filtering behavior. Container-aware rewrites
//! should use `rewrite::Context`, which additionally enforces byte budgets and
//! rendering constraints. This module does not certify whole-file privacy.
use zencodec::{Metadata, MetadataPolicy};

/// Filter an extracted record and reconcile its EXIF orientation.
///
/// The legacy implementation remains in zencodec until its next breaking
/// release. Delegating keeps one implementation during that compatibility period.
/// Gain-map callers use [`crate::display::filter_for_gain_map`] to validate the
/// rendering dependency as well.
#[allow(deprecated)]
pub fn filter(metadata: &Metadata, policy: &MetadataPolicy) -> Metadata {
    metadata.filtered(policy)
}
