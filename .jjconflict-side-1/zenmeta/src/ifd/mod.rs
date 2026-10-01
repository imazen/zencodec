//! Shared TIFF/BigTIFF inventory traversal. Each codec supplies its decoder's
//! consumption rules through [`walk::Rules`]; no pixels are decoded here.
//! This module is available only with the `inventory` feature.

pub mod tags;
pub mod walk;
