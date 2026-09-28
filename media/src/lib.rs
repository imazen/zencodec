//! Experimental media contracts, validated through concrete codec adapters.
#![forbid(unsafe_code)]

#[cfg(feature = "av1-decode")]
pub mod av1;
#[cfg(feature = "av1-encode")]
pub mod av1_encode;
#[cfg(feature = "av1-decode")]
pub mod av1_index;

#[cfg(feature = "animation")]
pub mod animation;

pub mod color;
pub mod display;
pub mod ebml;
pub mod encode_color;
pub mod frame;
#[cfg(feature = "http")]
pub mod http;
pub mod ivf;
pub mod mp4;
pub mod plane;
pub mod source;
pub mod time;
pub mod track;
pub mod webm;

#[cfg(all(
    feature = "animation",
    any(feature = "av1-decode", feature = "av1-encode")
))]
pub mod video;
