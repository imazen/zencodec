//! Color profile types — re-exported from [`zenpixels`].
//!
//! These types appear in the zencodec-types public API ([`ImageInfo`](crate::ImageInfo),
//! [`SourceColor`](crate::info::SourceColor), etc.), so they are re-exported here
//! for convenience. Users don't need to add `zenpixels` as a direct dependency
//! just for these types.

pub use zenpixels::{ColorContext, ColorProfileSource, NamedProfile};
