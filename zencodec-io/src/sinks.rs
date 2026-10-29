//! Sinks shared by drivers and tests.

use alloc::format;
use alloc::vec::Vec;

use zencodec::decode::{DecodeRowSink, SinkError};
use zenpixels::{PixelBuffer, PixelDescriptor, PixelSlice, PixelSliceMut};

/// A sink that collects every final row into one tightly packed image.
///
/// `begin` sizes the buffer; `provide_next_buffer` hands out the rows in
/// place, so the decoder writes straight into the final image.
#[derive(Default)]
pub struct CollectSink {
    data: Vec<u8>,
    width: u32,
    height: u32,
    descriptor: Option<PixelDescriptor>,
    rows_written: u64,
    finished: bool,
}

impl CollectSink {
    /// An empty sink.
    pub fn new() -> Self {
        Self::default()
    }

    /// Output width, once `begin` has been called.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Output height, once `begin` has been called.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Output descriptor, once `begin` has been called.
    pub fn descriptor(&self) -> Option<PixelDescriptor> {
        self.descriptor
    }

    /// Rows written so far (a row written twice counts twice).
    pub fn rows_written(&self) -> u64 {
        self.rows_written
    }

    /// Whether `finish` has been called.
    pub fn finished(&self) -> bool {
        self.finished
    }

    /// The collected pixels, tightly packed.
    pub fn bytes(&self) -> &[u8] {
        &self.data
    }

    /// A zero-copy view of the collected image.
    pub fn as_pixel_slice(&self) -> Option<PixelSlice<'_>> {
        let desc = self.descriptor?;
        PixelSlice::new(
            &self.data,
            self.width,
            self.height,
            self.width as usize * desc.bytes_per_pixel(),
            desc,
        )
        .ok()
    }

    /// Copy the collected image into an owned [`PixelBuffer`].
    pub fn to_buffer(&self) -> Option<PixelBuffer> {
        let src = self.as_pixel_slice()?;
        let mut out = PixelBuffer::new(self.width, self.height, src.descriptor());
        {
            let mut dst = out.as_slice_mut();
            for y in 0..self.height {
                let n = src.row(y).len().min(dst.row_mut(y).len());
                dst.row_mut(y)[..n].copy_from_slice(&src.row(y)[..n]);
            }
        }
        Some(out)
    }
}

impl core::fmt::Debug for CollectSink {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CollectSink")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("descriptor", &self.descriptor)
            .field("rows_written", &self.rows_written)
            .field("finished", &self.finished)
            .finish_non_exhaustive()
    }
}

impl DecodeRowSink for CollectSink {
    fn begin(
        &mut self,
        width: u32,
        height: u32,
        descriptor: PixelDescriptor,
    ) -> Result<(), SinkError> {
        if self.descriptor.is_some() {
            return Err("CollectSink: begin called twice".into());
        }
        let stride = (width as usize)
            .checked_mul(descriptor.bytes_per_pixel())
            .ok_or_else(|| SinkError::from("CollectSink: row too large"))?;
        let total = stride
            .checked_mul(height as usize)
            .ok_or_else(|| SinkError::from("CollectSink: image too large"))?;
        let mut data = Vec::new();
        data.try_reserve_exact(total)?;
        data.resize(total, 0);
        self.data = data;
        self.width = width;
        self.height = height;
        self.descriptor = Some(descriptor);
        Ok(())
    }

    fn provide_next_buffer(
        &mut self,
        y: u32,
        height: u32,
        width: u32,
        descriptor: PixelDescriptor,
    ) -> Result<PixelSliceMut<'_>, SinkError> {
        let Some(desc) = self.descriptor else {
            return Err("CollectSink: rows before begin".into());
        };
        if width != self.width || descriptor != desc {
            return Err(format!(
                "CollectSink: strip {width}x{height} {descriptor:?} does not match image {}x{} {desc:?}",
                self.width, self.height
            )
            .into());
        }
        let end = y
            .checked_add(height)
            .filter(|end| *end <= self.height)
            .ok_or_else(|| {
                SinkError::from(format!(
                    "CollectSink: rows {y}..{} exceed height {}",
                    y as u64 + height as u64,
                    self.height
                ))
            })?;
        self.rows_written += height as u64;
        let stride = width as usize * desc.bytes_per_pixel();
        let start = y as usize * stride;
        let len = (end - y) as usize * stride;
        PixelSliceMut::new(
            &mut self.data[start..start + len],
            width,
            height,
            stride,
            desc,
        )
        .map_err(|e| format!("CollectSink: {e}").into())
    }

    fn finish(&mut self) -> Result<(), SinkError> {
        self.finished = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_sink_refuses_unrepresentable_capacity_without_panic_or_state_change() {
        let mut sink = CollectSink::new();
        // On 64-bit this fits usize but exceeds Vec's isize capacity limit;
        // on 32-bit the multiplication itself must refuse. Neither allocates.
        assert!(
            sink.begin(u32::MAX, u32::MAX, PixelDescriptor::GRAY8_SRGB)
                .is_err()
        );
        assert_eq!(sink.descriptor(), None);
        assert_eq!((sink.width(), sink.height()), (0, 0));
        assert!(sink.bytes().is_empty());
        assert!(!sink.finished());
        sink.begin(3, 2, PixelDescriptor::GRAY8_SRGB).unwrap();
        for y in 0..2 {
            sink.provide_next_buffer(y, 1, 3, PixelDescriptor::GRAY8_SRGB)
                .unwrap()
                .row_mut(0)
                .copy_from_slice(&[1 + y as u8, 3, 5]);
        }
        sink.finish().unwrap();
        assert_eq!(sink.bytes(), &[1, 3, 5, 2, 3, 5]);
        assert_eq!(sink.rows_written(), 2);
        assert!(sink.finished());
    }

    #[test]
    fn collect_sink_refuses_geometry_overflow() {
        let mut sink = CollectSink::new();
        assert!(
            sink.begin(u32::MAX, u32::MAX, PixelDescriptor::RGBAF32_LINEAR)
                .is_err()
        );
        assert!(sink.bytes().is_empty());
        assert_eq!(sink.descriptor(), None);
    }
}
