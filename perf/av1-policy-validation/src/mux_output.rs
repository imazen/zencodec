//! Native qualification uses the shared public driver, not a private copy.
use enough::Stop;
use std::io::{self, Seek, SeekFrom, Write};
use zencodec_media::output::*;
use zencodec_media::{
    container::{MuxHint, MuxTiming, Muxer},
    packet::Packet,
    session::{ErasedError, ProtocolViolation, SessionError, UnsupportedMedia},
    track::TrackInfo,
};
#[path = "mux_output_tests.rs"]
mod tests;
