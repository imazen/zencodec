//! Native TIFF qualification of the isolated document contract.
//!
//! This test-only package reuses the frozen image-tiff parser and decoder. Its
//! worker bridge makes synchronous native reads visible as the same byte requests
//! used by discovery and decoding. A worker can finish its current native call
//! before drop joins it; no hard deadline or efficiency claim is made.
#![forbid(unsafe_code)]
#[cfg(test)]
mod bridge;
#[cfg(test)]
mod native;
#[cfg(test)]
mod tests;
