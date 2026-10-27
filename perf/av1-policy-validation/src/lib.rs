//! Standalone native qualification, outside the published dependency graph.
#![forbid(unsafe_code)]

#[cfg(test)]
mod fixtures;
#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "../../../docs/examples/checked_video.rs"]
mod checked_video;

// Compile the same private output candidate against the native video harness.
// These modules are test-only; no published crate gains a dependency or API.
#[cfg(test)]
mod epoch_route;
#[cfg(test)]
#[path = "../../spatial-validation/src/image_items.rs"]
pub mod image_items;
#[cfg(test)]
#[path = "../../spatial-validation/src/image_output.rs"]
pub mod image_output;
#[cfg(test)]
mod image_output_tests;
#[cfg(test)]
mod mux_output;
#[cfg(test)]
mod planar_conversion_tests;

#[cfg(test)]
mod prepared_conversion;

#[cfg(test)]
mod planar_float;

#[cfg(test)]
mod float_processing;
