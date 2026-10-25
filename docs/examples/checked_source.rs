//! Executed by the native TIFF harness; this probes geometry, not full interpretation.
#![forbid(unsafe_code)]

// EXAMPLE: positional_parser
use std::path::Path;
use tiff::decoder::{Decoder, Limits};
use zencodec_io::{FileReadAt, ReadAtReader};

pub fn stored_dimensions(
    path: &Path,
    native_limits: Limits,
) -> Result<(u32, u32), Box<dyn std::error::Error + Send + Sync>> {
    let source = FileReadAt::open(path)?;
    // ANCHOR: positional_parser
    let input = ReadAtReader::new(source);
    // END: positional_parser
    // TIFF's header-only open precedes directory parsing, so limits apply first.
    let mut decoder = Decoder::open(input)?.with_limits(native_limits);
    decoder.next_image()?;
    Ok(decoder.dimensions()?)
}
// END EXAMPLE: positional_parser
