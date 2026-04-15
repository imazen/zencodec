//! Codec implementation helpers.
//!
//! Free functions that codec crates use internally to implement trait methods.
//! These are not part of the consumer-facing API — they exist so codecs don't
//! have to duplicate boilerplate for common patterns.
//!
//! # Submodules
//!
//! - [`icc`]: ICC profile identification and pixel descriptor derivation.

use alloc::borrow::Cow;

use enough::Stop;
use zenpixels::PixelDescriptor;

use crate::cost::OutputInfo;
use crate::sink::SinkError;
use crate::traits::{AnimationFrameDecoder, Decode, DecodeJob};

mod exif;
mod icc;
pub use exif::parse_exif_orientation;
pub use icc::{descriptor_for_decoded_pixels, descriptor_for_decoded_pixels_v2};
#[allow(deprecated)]
pub use icc::{IccMatchTolerance, icc_profile_is_srgb, identify_well_known_icc};

/// Implement `push_decoder` by doing a full decode and copying rows to the sink.
///
/// Most codecs that don't have a native streaming decode path can use this to
/// implement [`DecodeJob::push_decoder`] trivially:
///
/// ```rust,ignore
/// fn push_decoder(
///     self,
///     data: Cow<'a, [u8]>,
///     sink: &mut dyn DecodeRowSink,
///     preferred: &[PixelDescriptor],
/// ) -> Result<OutputInfo, Self::Error> {
///     zencodec::helpers::copy_decode_to_sink(self, data, sink, preferred, MyError::from_sink)
/// }
/// ```
pub fn copy_decode_to_sink<'a, J>(
    job: J,
    data: Cow<'a, [u8]>,
    sink: &mut dyn crate::DecodeRowSink,
    preferred: &[PixelDescriptor],
    wrap_sink_error: fn(SinkError) -> J::Error,
) -> Result<OutputInfo, J::Error>
where
    J: DecodeJob<'a>,
{
    let dec = job.decoder(data, preferred)?;
    let output = dec.decode()?;
    let ps = output.pixels();
    let desc = ps.descriptor();
    let w = ps.width();
    let h = ps.rows();

    sink.begin(w, h, desc).map_err(wrap_sink_error)?;

    let mut dst = sink
        .provide_next_buffer(0, h, w, desc)
        .map_err(wrap_sink_error)?;
    for row in 0..h {
        dst.row_mut(row).copy_from_slice(ps.row(row));
    }
    drop(dst);

    sink.finish().map_err(wrap_sink_error)?;

    let info = output.info();
    Ok(OutputInfo::full_decode(info.width, info.height, desc))
}

/// Implement `render_next_frame_to_sink` by rendering a frame and copying rows.
///
/// Codecs that implement [`AnimationFrameDecoder`] can use this to implement
/// `render_next_frame_to_sink` without duplicating the row-copy logic:
///
/// ```rust,ignore
/// fn render_next_frame_to_sink(
///     &mut self,
///     stop: Option<&dyn Stop>,
///     sink: &mut dyn DecodeRowSink,
/// ) -> Result<Option<OutputInfo>, Self::Error> {
///     zencodec::helpers::copy_frame_to_sink(self, stop, sink)
/// }
/// ```
pub fn copy_frame_to_sink<D: AnimationFrameDecoder>(
    decoder: &mut D,
    stop: Option<&dyn Stop>,
    sink: &mut dyn crate::DecodeRowSink,
) -> Result<Option<OutputInfo>, D::Error> {
    let frame = match decoder.render_next_frame(stop)? {
        Some(f) => f,
        None => return Ok(None),
    };
    let ps = frame.pixels();
    let desc = ps.descriptor();
    let w = ps.width();
    let h = ps.rows();

    sink.begin(w, h, desc).map_err(D::wrap_sink_error)?;
    let mut dst = sink
        .provide_next_buffer(0, h, w, desc)
        .map_err(D::wrap_sink_error)?;
    for row in 0..h {
        dst.row_mut(row).copy_from_slice(ps.row(row));
    }
    drop(dst);
    sink.finish().map_err(D::wrap_sink_error)?;

    Ok(Some(OutputInfo::full_decode(w, h, desc)))
}
