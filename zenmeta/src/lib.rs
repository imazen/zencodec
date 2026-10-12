#![no_std]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

extern crate alloc;

#[cfg(feature = "audit")]
pub mod audit;
pub mod display;
#[cfg(feature = "xmp")]
pub mod xmp;
