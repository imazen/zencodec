#![forbid(unsafe_code)]
//! Executing native spatial candidates, not a promoted public API.
#![cfg(test)]

mod checked_collection;
mod checked_collection_tests;
#[path = "../../../docs/examples/checked_source.rs"]
mod checked_source;
mod collection;
mod collection_factories;
mod control;
mod description;
mod description_checked_tests;
mod description_jxl_tests;
mod discovery;
mod generated_output_tests;
#[cfg(all(feature = "file_snapshot", not(target_os = "linux")))]
compile_error!("the file_snapshot experiment requires Linux and a reflink-capable filesystem");
#[cfg(feature = "file_snapshot")]
mod file_snapshot;
#[cfg(feature = "http")]
mod http_source;
pub mod image_items;
pub mod image_output;
mod jxl_admission_tests;
mod jxl_items;
mod jxl_layer_tests;
mod jxl_layers;
mod jxl_output;
mod jxl_tests;
mod output_tests;
mod placement;
#[cfg(test)]
mod strip_tests;
mod tests;
mod tiff_items;
mod tiff_pixels;
mod tiff_region;
