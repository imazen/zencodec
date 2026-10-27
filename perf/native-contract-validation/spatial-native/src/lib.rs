//! Plan a stored-region read before any coding unit's compressed bytes are
//! read, over the same native TIFF parser and region candidate that
//! perf/spatial-validation qualifies (included unchanged by path).
#![forbid(unsafe_code)]

#[path = "../../../spatial-validation/src/image_items.rs"]
pub mod image_items;
// Included unchanged; this crate does not use its directory seam.
#[allow(dead_code)]
#[path = "../../../spatial-validation/src/tiff_region.rs"]
pub mod tiff_region;

pub mod plan;
pub mod transport;
