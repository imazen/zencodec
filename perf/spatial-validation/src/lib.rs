#![forbid(unsafe_code)]
//! Executing native spatial candidates, not a promoted public API.
#![cfg(test)]

#[path = "../../../docs/examples/checked_source.rs"]
mod checked_source;
mod collection;
mod control;
mod discovery;
mod generated_output_tests;
#[cfg(feature = "http")]
mod http_source;
pub mod image_items;
pub mod image_output;
mod jxl_items;
mod jxl_output;
mod jxl_tests;
mod output_tests;
#[cfg(test)]
mod strip_tests;
mod tests;
mod tiff_items;
mod tiff_pixels;
mod tiff_region;
