//! Sinks shared by drivers and tests.

use alloc::format;
use alloc::vec::Vec;

use zencodec::decode::{DecodeRowSink, SinkError};
use zenpixels::{PixelBuffer, PixelDescriptor, PixelSlice, PixelSliceMut};

/// A sink that collects every final row into one tightly packed image.
///
/// `begin` sizes the buffer; `provide_next_buffer` hands out the rows in
/// place, so the decoder writes straight into the final image. Final-row
/// coverage is tracked separately: repeated or overlapping strips cannot hide
/// missing rows at finish. The coverage bitmap uses eight bytes per 64 rows
/// (rounded up), independent of image width. Supplying a buffer does not prove
/// that the codec actually wrote every byte; native correctness remains required.
#[derive(Default)]
pub struct CollectSink {
    data: Option<PixelBuffer>,
    width: u32,
    height: u32,
    descriptor: Option<PixelDescriptor>,
    rows_written: u64,
    covered_rows: Vec<u64>,
    unique_rows: u32,
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
        self.data
            .as_ref()
            .map_or(&[], |buffer| buffer.as_slice().as_strided_bytes())
    }

    /// A zero-copy view of the collected image.
    pub fn as_pixel_slice(&self) -> Option<PixelSlice<'_>> {
        self.data.as_ref().map(PixelBuffer::as_slice)
    }

    /// Take the finished image without copying its pixels or allocating.
    ///
    /// Returns `None` before [`DecodeRowSink::finish`] succeeds. Call this only
    /// after the driver returns success: a sink's finish is not proof that the
    /// decoder's later validation succeeded. This does not include metadata;
    /// retain the decoder's completion details separately.
    pub fn into_buffer(self) -> Option<PixelBuffer> {
        self.finished.then_some(self.data).flatten()
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
        // Retain the original reservation cause and leave state unchanged on
        // failure. Prefix slack lets PixelBuffer align U16/F32 storage without
        // copying; every row is written through that same aligned owner.
        let capacity = total
            .checked_add(descriptor.min_alignment() - 1)
            .ok_or_else(|| SinkError::from("CollectSink: aligned image too large"))?;
        let mut data = Vec::new();
        data.try_reserve_exact(capacity)?;
        data.resize(capacity, 0);
        let buffer = PixelBuffer::from_vec(data, width, height, descriptor)?;
        let words = usize::try_from(u64::from(height).div_ceil(64))
            .map_err(|_| SinkError::from("CollectSink: coverage too large"))?;
        let mut coverage = Vec::new();
        coverage.try_reserve_exact(words)?;
        coverage.resize(words, 0);
        self.covered_rows = coverage;
        self.data = Some(buffer);
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
        if self.finished {
            return Err("CollectSink: rows after finish".into());
        }
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
        y.checked_add(height)
            .filter(|end| *end <= self.height)
            .ok_or_else(|| {
                SinkError::from(format!(
                    "CollectSink: rows {y}..{} exceed height {}",
                    y as u64 + height as u64,
                    self.height
                ))
            })?;
        for row in y..y + height {
            let word = &mut self.covered_rows[(row / 64) as usize];
            let bit = 1u64 << (row % 64);
            if *word & bit == 0 {
                self.unique_rows += 1;
                *word |= bit;
            }
        }
        self.rows_written += height as u64;
        Ok(self
            .data
            .as_mut()
            .expect("begin installed pixel storage")
            .rows_mut(y, height))
    }

    fn finish(&mut self) -> Result<(), SinkError> {
        if self.descriptor.is_none() {
            return Err("CollectSink: finish before begin".into());
        }
        if self.unique_rows != self.height {
            return Err("CollectSink: finish before every row was supplied".into());
        }
        self.finished = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finish_without_begin_is_not_completion() {
        let mut sink = CollectSink::new();
        assert!(sink.finish().is_err());
        assert!(!sink.finished());
        assert!(sink.into_buffer().is_none());
    }

    #[test]
    fn completed_pixels_cannot_be_reopened_for_writing() {
        let mut sink = CollectSink::new();
        sink.begin(1, 1, PixelDescriptor::GRAY8_SRGB).unwrap();
        sink.provide_next_buffer(0, 1, 1, PixelDescriptor::GRAY8_SRGB)
            .unwrap();
        sink.finish().unwrap();
        assert!(
            sink.provide_next_buffer(0, 1, 1, PixelDescriptor::GRAY8_SRGB)
                .is_err()
        );
    }

    #[test]
    fn completion_requires_unique_coverage_and_closes_writes() {
        let mut sink = CollectSink::new();
        sink.begin(1, 65, PixelDescriptor::GRAY8_SRGB).unwrap();
        for _ in 0..65 {
            sink.provide_next_buffer(0, 1, 1, PixelDescriptor::GRAY8_SRGB)
                .unwrap();
        }
        assert_eq!(sink.rows_written(), 65);
        assert!(sink.finish().is_err());
        assert!(!sink.finished());
        // Out-of-order and overlapping delivery remain valid.
        sink.provide_next_buffer(32, 33, 1, PixelDescriptor::GRAY8_SRGB)
            .unwrap();
        sink.provide_next_buffer(1, 32, 1, PixelDescriptor::GRAY8_SRGB)
            .unwrap();
        sink.finish().unwrap();
        assert!(
            sink.provide_next_buffer(0, 1, 1, PixelDescriptor::GRAY8_SRGB)
                .is_err()
        );
        assert!(sink.into_buffer().is_some());
    }

    #[test]
    fn finished_storage_moves_without_copy_for_integer_and_float_rows() {
        for descriptor in [
            PixelDescriptor::RGB8_SRGB,
            PixelDescriptor::RGB16_SRGB,
            PixelDescriptor::RGBAF32_LINEAR,
        ] {
            let mut sink = CollectSink::new();
            sink.begin(3, 2, descriptor).unwrap();
            let original = sink.bytes().as_ptr();
            for y in 0..2 {
                let mut rows = sink.provide_next_buffer(y, 1, 3, descriptor).unwrap();
                rows.row_mut(0).fill(37 + y as u8);
            }
            sink.finish().unwrap();
            let buffer = sink.into_buffer().unwrap();
            assert_eq!(buffer.as_slice().as_strided_bytes().as_ptr(), original);
            assert_eq!(buffer.descriptor(), descriptor);
            for y in 0..2 {
                assert!(buffer.as_slice().row(y).iter().all(|b| *b == 37 + y as u8));
            }
        }
    }

    #[test]
    fn unfinished_collectors_do_not_yield_owned_completion() {
        assert!(CollectSink::new().into_buffer().is_none());
        let mut sink = CollectSink::new();
        sink.begin(2, 1, PixelDescriptor::GRAY8_SRGB).unwrap();
        sink.provide_next_buffer(0, 1, 2, PixelDescriptor::GRAY8_SRGB)
            .unwrap()
            .row_mut(0)
            .copy_from_slice(&[17, 23]);
        assert!(sink.into_buffer().is_none());
    }

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
