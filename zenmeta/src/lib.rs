#![no_std]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

extern crate alloc;

#[cfg(feature = "apple")]
pub mod apple;
#[cfg(feature = "audit")]
pub mod audit;
#[cfg(feature = "apple")]
pub mod bplist;
pub mod display;
#[cfg(feature = "inventory")]
pub mod ifd;
#[cfg(feature = "xmp")]
pub mod xmp;
