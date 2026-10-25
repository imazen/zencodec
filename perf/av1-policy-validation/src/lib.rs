//! Standalone native qualification, outside the published dependency graph.
#![forbid(unsafe_code)]

#[cfg(test)]
mod fixtures;
#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "../../../docs/examples/checked_video.rs"]
mod checked_video;
