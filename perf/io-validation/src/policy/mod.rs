//! Unified policy resolver experiment. No published API. Offered guarantees are
//! adapter claims until qualified by native output/allocation tests.
#![allow(dead_code)]
extern crate alloc;

mod engine;
pub(crate) mod plan;
mod resolver;
mod types;

pub(crate) use engine::*;
pub(crate) use resolver::*;
pub(crate) use types::*;
