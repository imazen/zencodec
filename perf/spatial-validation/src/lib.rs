#![forbid(unsafe_code)]
//! Executing native spatial candidates, not a promoted public API.
#![cfg(test)]

#[path = "../../../docs/examples/checked_source.rs"]
mod checked_source;
mod collection;
mod image_items;
mod jxl_items;
mod jxl_tests;
#[cfg(test)]
mod strip_tests;
mod tests;
mod tiff_items;
mod tiff_pixels;
mod tiff_region;
