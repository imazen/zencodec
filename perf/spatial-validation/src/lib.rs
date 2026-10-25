#![forbid(unsafe_code)]
//! Executing native spatial candidates, not a promoted public API.
#![cfg(test)]

#[path = "../../../docs/examples/checked_source.rs"]
mod checked_source;
#[cfg(test)]
mod strip_tests;
mod tests;
mod tiff_region;
