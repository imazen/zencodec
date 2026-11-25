//! Output prediction for decode operations.
//!
//! [`OutputInfo`] describes what a decode will produce given current hints.

use crate::Orientation;
use zenpixels::ImageSpec;

/// Exact timing of a successfully delivered animation frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameCompletion {
    /// Duration as stored by the source.
    pub duration: crate::animation::FrameDuration,
    /// Zero-based display index.
    pub frame_index: u32,
    /// Source presentation timestamp, when available.
    pub pts: Option<crate::time::Timestamp>,
}

/// Predicted output from a decode operation.
///
/// Returned by [`DecodeJob::output_info()`](crate::decode::DecodeJob::output_info).
/// Describes what `decode()` or `decode_into()` will produce given the
/// current decode hints (crop, scale, orientation).
///
/// Use this to allocate destination buffers — the `width` and `height`
/// are what the decoder will actually write.
///
/// # Natural info vs output info
///
/// [`ImageInfo`](crate::ImageInfo) from `probe_header()` describes the file as stored:
/// original dimensions, original orientation, embedded metadata.
///
/// `OutputInfo` describes the decoder's output: post-crop, post-scale,
/// post-orientation dimensions and native pixel format. Format preferences
/// supplied when creating an executor can select another descriptor;
/// `native_format` does not reject or admit that negotiated choice.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct OutputInfo {
    /// Width of the decoded output in pixels.
    pub width: u32,
    /// Height of the decoded output in pixels.
    pub height: u32,
    /// Pixel format the decoder will produce natively (for `decode()`).
    ///
    /// For `decode_into()`, use any format from
    /// [`supported_descriptors()`](crate::decode::DecoderConfig::supported_descriptors) —
    /// this field tells you what the codec would pick if you let it choose.
    pub native_format: ImageSpec,
    /// Whether the output has an alpha channel.
    pub has_alpha: bool,
    /// Orientation the decoder will apply internally.
    ///
    /// [`Identity`](Orientation::Identity) means the decoder will NOT handle
    /// orientation — the caller must apply it. Any other value means the
    /// decoder will rotate/flip the pixels, and the output `width`/`height`
    /// already reflect the rotated dimensions.
    ///
    /// Remaining orientation for the caller:
    /// `natural.orientation - orientation_applied` (via D4 group composition).
    pub orientation_applied: Orientation,
    /// Crop the decoder will actually apply (`[x, y, width, height]` in
    /// source coordinates).
    ///
    /// May differ from the crop hint due to block alignment (JPEG MCU
    /// boundaries, AV1 superblock alignment, etc.). `None` if no crop.
    pub crop_applied: Option<[u32; 4]>,
    /// Timing of an animation frame delivered to a sink.
    pub frame: Option<FrameCompletion>,
}

impl OutputInfo {
    /// Create an `OutputInfo` for a simple full-frame decode (no hints applied).
    pub fn full_decode(width: u32, height: u32, native_format: ImageSpec) -> Self {
        Self {
            width,
            height,
            has_alpha: crate::helpers::has_alpha(&native_format),
            native_format,
            orientation_applied: Orientation::Identity,
            crop_applied: None,
            frame: None,
        }
    }

    /// Set whether the output has alpha.
    pub fn with_alpha(mut self, has_alpha: bool) -> Self {
        self.has_alpha = has_alpha;
        self
    }

    /// Set the orientation the decoder will apply.
    pub fn with_orientation_applied(mut self, o: Orientation) -> Self {
        self.orientation_applied = o;
        self
    }

    /// Set the crop the decoder will apply.
    pub fn with_crop_applied(mut self, rect: [u32; 4]) -> Self {
        self.crop_applied = Some(rect);
        self
    }

    /// Attach exact timing for a completed animation frame.
    pub fn with_frame(mut self, frame: FrameCompletion) -> Self {
        self.frame = Some(frame);
        self
    }

    /// Minimum buffer size in bytes for the native format (no padding).
    ///
    /// This is `width * height * bytes_per_pixel`. For aligned/strided
    /// buffers, use [`zenpixels::SampleLayout::aligned_stride()`] instead.
    pub fn buffer_size(&self) -> u64 {
        self.width as u64 * self.height as u64 * self.native_format.layout.bytes_per_pixel() as u64
    }

    /// Pixel count (`width * height`).
    pub fn pixel_count(&self) -> u64 {
        self.width as u64 * self.height as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zenpixels::ImageSpec;

    #[test]
    fn output_info_full_decode() {
        let info = OutputInfo::full_decode(10, 5, ImageSpec::RGBA8_SRGB);
        assert_eq!(info.buffer_size(), 200); // 10*5*4
        assert_eq!(info.pixel_count(), 50); // 10*5
        assert!(info.has_alpha); // RGBA8 has alpha
    }
}
