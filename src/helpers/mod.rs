//! Codec implementation helpers.
//!
//! Free functions that codec crates use internally to implement trait methods.
//! These are not part of the consumer-facing API — they exist so codecs don't
//! have to duplicate boilerplate for common patterns.
//!
//! # Submodules
//!
//! - `icc`: ICC profile identification and pixel descriptor derivation.
//! - [`WholeFileIncremental`](crate::helpers::WholeFileIncremental): the sans-I/O decode contract for a codec core
//!   that needs the complete file.

use alloc::borrow::Cow;

use zenpixels::PixelFormat;

use crate::cost::OutputInfo;
use crate::sink::SinkError;
use crate::traits::{AnimationFrameDecoder, Decode, DecodeJob};

mod cicp;
mod exif;
mod icc;
mod pixels;
mod whole_file;

pub use cicp::{
    CicpDisplay, color_primaries_name, matrix_coefficients_name, transfer_characteristics_name,
};
pub use exif::{parse_exif_orientation, set_exif_color_space, set_exif_orientation};
pub use icc::descriptor_for_decoded_pixels_v2;
#[allow(deprecated)]
pub use icc::{
    IccMatchTolerance, descriptor_for_decoded_pixels, icc_profile_is_srgb, identify_well_known_icc,
};
pub use pixels::codec_spec;
#[cfg(test)]
pub(crate) use pixels::signal_range;
pub(crate) use pixels::{has_alpha, redacted_spec};
pub use whole_file::WholeFileIncremental;

/// Copy already-decoded rows into the sink's admitted strip capacity.
///
/// Preserves source and destination strides, absolute row coordinates and
/// progressive-pass metadata. Calls neither `begin` nor `finish`; the codec
/// owns that lifecycle. A provisional batch must already be authorized by the
/// sink's `accepts_provisional` contract. Does not allocate or change the native
/// batch's lifetime. A sink failure can follow partially copied strips.
pub fn copy_rows_to_sink(
    sink: &mut dyn crate::DecodeRowSink,
    meta: crate::decode::BatchMeta,
    pixels: &zenpixels::PixelView<'_>,
) -> Result<(), SinkError> {
    meta.y
        .checked_add(pixels.rows())
        .ok_or_else(|| SinkError::from("copied rows exceed the row coordinate range"))?;
    let max_rows = sink.max_buffer_rows().get();
    let metadata = crate::sink::RowMetadata::from_view(pixels);
    let mut offset = 0;
    while offset < pixels.rows() {
        let rows = max_rows.min(pixels.rows() - offset);
        let mut part = meta;
        part.y += offset;
        let mut dst = sink.provide_buffer_with_metadata(
            part,
            rows,
            pixels.width(),
            pixels.format().clone(),
            &metadata,
        )?;
        if dst.width() != pixels.width() || dst.rows() != rows || dst.format() != pixels.format() {
            return Err("sink returned a buffer with a different geometry or descriptor".into());
        }
        for row in 0..rows {
            dst.row_mut(row).copy_from_slice(pixels.row(offset + row));
        }
        offset += rows;
    }
    Ok(())
}

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
///     preferred: &[PixelFormat],
/// ) -> Result<OutputInfo, Self::Error> {
///     zencodec::helpers::copy_decode_to_sink(self, data, sink, preferred, MyError::from_sink)
/// }
/// ```
pub fn copy_decode_to_sink<'a, J>(
    job: J,
    data: Cow<'a, [u8]>,
    sink: &mut dyn crate::DecodeRowSink,
    preferred: &[PixelFormat],
    wrap_sink_error: fn(SinkError) -> J::Error,
) -> Result<OutputInfo, J::Error>
where
    J: DecodeJob<'a>,
{
    let mut completion = job.output_info(&data)?;
    let dec = job.decoder(data, preferred)?;
    let output = dec.decode()?;
    let ps = output.pixels();
    let desc = ps.format().clone();
    let w = ps.width();
    let h = ps.rows();

    sink.begin(w, h, desc.clone()).map_err(wrap_sink_error)?;

    copy_rows_to_sink(sink, crate::decode::BatchMeta::final_rows(0), &ps)
        .map_err(wrap_sink_error)?;

    sink.finish().map_err(wrap_sink_error)?;

    completion.width = w;
    completion.height = h;
    completion.has_alpha = crate::helpers::has_alpha(&desc);
    completion.native_format = desc;
    Ok(completion)
}

/// Implement `render_next_frame_to_sink` by rendering a frame and copying rows.
///
/// Codecs that implement [`AnimationFrameDecoder`] can use this to implement
/// `render_next_frame_to_sink` without duplicating the row-copy logic:
///
/// ```rust,ignore
/// fn render_next_frame_to_sink(
///     &mut self,
///     pulse: Option<&dyn crate::Pulse>,
///     sink: &mut dyn DecodeRowSink,
/// ) -> Result<Option<OutputInfo>, Self::Error> {
///     zencodec::helpers::copy_frame_to_sink(self, pulse, sink)
/// }
/// ```
pub fn copy_frame_to_sink<D: AnimationFrameDecoder>(
    decoder: &mut D,
    pulse: Option<&dyn crate::Pulse>,
    sink: &mut dyn crate::DecodeRowSink,
) -> Result<Option<OutputInfo>, D::Error> {
    let frame = match decoder.render_next_frame(pulse)? {
        Some(f) => f,
        None => return Ok(None),
    };
    let ps = frame.pixels();
    let desc = ps.format().clone();
    let w = ps.width();
    let h = ps.rows();

    sink.begin(w, h, desc.clone()).map_err(D::wrap_sink_error)?;
    copy_rows_to_sink(sink, crate::decode::BatchMeta::final_rows(0), ps)
        .map_err(D::wrap_sink_error)?;
    sink.finish().map_err(D::wrap_sink_error)?;

    Ok(Some(OutputInfo::full_decode(w, h, desc).with_frame(
        crate::decode::FrameCompletion {
            duration: frame.duration(),
            frame_index: frame.frame_index(),
            pts: frame.pts(),
        },
    )))
}
