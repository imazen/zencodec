//! Conformance checks for `zencodec-media` implementations, and the
//! reference implementations they are validated against.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod checks;
mod frame_interchange;
pub use frame_interchange::check_frame_interchange;
pub mod toy;

pub use checks::{
    Conformance, Failure, check_animation_over_video, check_demuxer_contract, check_demuxer_seek,
    check_muxer_output_honesty, check_video_decoder_contract, check_video_encoder_contract,
};
