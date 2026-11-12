//! Drivers for the reimagined contract. A spike, not a release.
//!
//! The contract crate has no loops; these are the loops. Every one of them is
//! the same few lines around [`Decoder::step`](zencodec::decode::Decoder::step)
//! or [`Encoder::step`](zencodec::encode::Encoder::step), which is the point:
//!
//! | need | driver |
//! |---|---|
//! | bytes in memory → pixels | [`decode`], [`decode_slice`] |
//! | pixels in memory → bytes | [`encode`], [`encode_sequence`] |
//! | a file or any positioned reader | [`decode_read_at`] |
//! | a writer, with or without seeking | [`write_output`], [`Forward`], [`SeekBack`] |
//! | one codec's rows into another | [`EncoderSink`], [`transcode`] |
//! | an async source and sink | `tokio_io` (feature `tokio`) |

#![forbid(unsafe_code)]

mod collect;
pub mod document;
mod drive;
mod encoder_sink;
mod memory;
mod read;

pub use collect::{Collect, Frame, OwnedPlane, Pixels, PlanarPixels};
pub use drive::{DriveOptions, RangeRead};
pub use encoder_sink::EncoderSink;
pub use memory::{Decoded, decode, decode_slice, encode, encode_sequence, feed_frame, transcode};
pub use read::{
    Destination, Forward, ReadAt, SeekBack, SeekRead, Sequential, decode_read_at, write_output,
};

#[cfg(feature = "tokio")]
pub mod tokio_io;
