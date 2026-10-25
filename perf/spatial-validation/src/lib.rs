#![forbid(unsafe_code)]
//! Executing native spatial candidates, not a promoted public API.
#![cfg(test)]

mod io_bridge;
#[cfg(test)]
mod strip_tests;
mod tests;
mod tiff_region;
