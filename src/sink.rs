//! Zero-copy row sink for streaming decode.
//!
//! [`DecodeRowSink`] lets the caller own the output buffer while the codec
//! writes decoded pixels directly into it — no intermediate allocation.
//!
//! # Usage
//!
//! ```text
//! let job = config.job();
//! let info = job.output_info(data)?;
//! let desc = info.descriptor();
//!
//! // Set up sink with known dimensions and format
//! let mut sink = MySink::new(info.width(), info.height(), desc);
//! job.decoder().decode_rows(data, &mut sink)?;
//! ```
//!
//! # Contract
//!
//! - The codec calls [`demand()`](DecodeRowSink::demand) once per strip,
//!   in top-to-bottom order (`y` increases monotonically).
//! - The codec passes `width`, `height` (of the strip), and `descriptor`
//!   so the sink can construct the [`PixelSliceMut`] with appropriate stride.
//! - The returned [`PixelSliceMut`] carries the buffer, stride, dimensions,
//!   and pixel descriptor together — the codec writes into it via
//!   [`row_mut()`](crate::PixelSliceMut::row_mut).
//! - When `demand()` is called again, the previous buffer has been fully
//!   written. When `decode_rows()` returns, the last buffer has been written.

use zenpixels::{PixelDescriptor, PixelSliceMut};

/// Receives decoded rows during streaming decode.
///
/// The codec calls [`demand`](DecodeRowSink::demand) for each strip of rows,
/// writes decoded pixels directly into the returned [`PixelSliceMut`],
/// then calls `demand` again for the next strip. After
/// `decode_rows()` returns, the last
/// demanded buffer has been fully written.
///
/// The sink controls the stride — it can return tightly-packed buffers
/// (stride = width × bpp) or SIMD-aligned buffers (stride padded to 64 bytes).
/// The codec respects whatever stride the `PixelSliceMut` carries.
///
/// # Object safety
///
/// This trait is object-safe. Use `&mut dyn DecodeRowSink` in generic code.
///
/// # Example implementation
///
/// ```
/// use zencodec_types::DecodeRowSink;
/// use zenpixels::{PixelSliceMut, PixelDescriptor};
///
/// struct CollectSink {
///     buf: Vec<u8>,
/// }
///
/// impl DecodeRowSink for CollectSink {
///     fn demand(&mut self, _y: u32, height: u32, width: u32, descriptor: PixelDescriptor) -> PixelSliceMut<'_> {
///         let bpp = descriptor.bytes_per_pixel();
///         let stride = width as usize * bpp;
///         let needed = height as usize * stride;
///         self.buf.resize(needed, 0);
///         PixelSliceMut::new(&mut self.buf, width, height, stride, descriptor)
///             .expect("buffer sized correctly")
///     }
/// }
/// ```
pub trait DecodeRowSink {
    /// Provide a mutable pixel buffer for decoded rows `y .. y + height`.
    ///
    /// The codec passes the strip `width` (pixels), `height` (rows in this
    /// strip), and `descriptor` (pixel format). The sink returns a
    /// [`PixelSliceMut`] with its chosen stride.
    ///
    /// The codec writes into the buffer via
    /// [`row_mut()`](crate::PixelSliceMut::row_mut) for each row.
    ///
    /// When this method is called, any buffer returned by a previous call
    /// has been fully written with decoded pixel data.
    fn demand(
        &mut self,
        y: u32,
        height: u32,
        width: u32,
        descriptor: PixelDescriptor,
    ) -> PixelSliceMut<'_>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    /// Verify the basic demand/fill/demand lifecycle.
    #[test]
    fn demand_lifecycle() {
        struct TestSink {
            buf: Vec<u8>,
            completed: Vec<(u32, u32)>,
        }

        impl DecodeRowSink for TestSink {
            fn demand(
                &mut self,
                y: u32,
                height: u32,
                width: u32,
                descriptor: PixelDescriptor,
            ) -> PixelSliceMut<'_> {
                if y > 0 {
                    self.completed.push((y - height, height));
                }
                let bpp = descriptor.bytes_per_pixel();
                let stride = width as usize * bpp;
                let needed = height as usize * stride;
                self.buf.resize(needed, 0);
                PixelSliceMut::new(&mut self.buf, width, height, stride, descriptor)
                    .expect("valid buffer")
            }
        }

        let mut sink = TestSink {
            buf: Vec::new(),
            completed: Vec::new(),
        };

        let width = 10u32;
        let desc = PixelDescriptor::RGB8_SRGB;
        let bpp = desc.bytes_per_pixel();

        // Simulate a codec writing 3 strips of 8 rows each
        for strip in 0..3u32 {
            let y = strip * 8;
            let h = 8;
            let mut ps = sink.demand(y, h, width, desc);
            assert_eq!(ps.stride(), 30);
            // Simulate codec writing via row_mut
            for row in 0..h {
                let row_data = ps.row_mut(row);
                assert_eq!(row_data.len(), width as usize * bpp);
                row_data.fill((strip + 1) as u8);
            }
        }
        sink.completed.push((16, 8));

        assert_eq!(sink.completed.len(), 3);
        assert_eq!(sink.completed[0], (0, 8));
        assert_eq!(sink.completed[1], (8, 8));
        assert_eq!(sink.completed[2], (16, 8));
    }

    /// Verify object safety — can use as `&mut dyn DecodeRowSink`.
    #[test]
    fn object_safe() {
        struct SimpleSink {
            buf: Vec<u8>,
        }
        impl DecodeRowSink for SimpleSink {
            fn demand(
                &mut self,
                _y: u32,
                height: u32,
                width: u32,
                descriptor: PixelDescriptor,
            ) -> PixelSliceMut<'_> {
                let bpp = descriptor.bytes_per_pixel();
                let stride = width as usize * bpp;
                let needed = height as usize * stride;
                self.buf.resize(needed, 0);
                PixelSliceMut::new(&mut self.buf, width, height, stride, descriptor)
                    .expect("valid buffer")
            }
        }

        fn use_sink(sink: &mut dyn DecodeRowSink) {
            let ps = sink.demand(0, 8, 10, PixelDescriptor::RGB8_SRGB);
            assert_eq!(ps.stride(), 30);
            assert_eq!(ps.width(), 10);
            assert_eq!(ps.rows(), 8);
        }

        let mut sink = SimpleSink { buf: Vec::new() };
        use_sink(&mut sink);
    }

    /// Verify the lending pattern — each demand() call's borrow is independent.
    #[test]
    fn lending_borrow_pattern() {
        struct ReuseSink {
            buf: Vec<u8>,
            call_count: u32,
        }
        impl DecodeRowSink for ReuseSink {
            fn demand(
                &mut self,
                _y: u32,
                height: u32,
                width: u32,
                descriptor: PixelDescriptor,
            ) -> PixelSliceMut<'_> {
                self.call_count += 1;
                let bpp = descriptor.bytes_per_pixel();
                let stride = width as usize * bpp;
                let needed = height as usize * stride;
                self.buf.resize(needed, 0);
                PixelSliceMut::new(&mut self.buf, width, height, stride, descriptor)
                    .expect("valid buffer")
            }
        }

        let mut sink = ReuseSink {
            buf: Vec::new(),
            call_count: 0,
        };

        let desc = PixelDescriptor::GRAY8_SRGB;

        // Multiple sequential borrows — each one ends before the next starts
        {
            let mut ps = sink.demand(0, 4, 10, desc);
            for row in 0..4 {
                ps.row_mut(row).fill(1);
            }
        }
        {
            let mut ps = sink.demand(4, 4, 10, desc);
            for row in 0..4 {
                ps.row_mut(row).fill(2);
            }
        }
        {
            let mut ps = sink.demand(8, 4, 10, desc);
            for row in 0..4 {
                ps.row_mut(row).fill(3);
            }
        }

        assert_eq!(sink.call_count, 3);
        // Last write was 3
        assert_eq!(sink.buf[0], 3);
    }

    /// Verify sink can provide SIMD-aligned stride.
    #[test]
    fn simd_aligned_sink() {
        struct AlignedSink {
            buf: Vec<u8>,
        }
        impl DecodeRowSink for AlignedSink {
            fn demand(
                &mut self,
                _y: u32,
                height: u32,
                width: u32,
                descriptor: PixelDescriptor,
            ) -> PixelSliceMut<'_> {
                let bpp = descriptor.bytes_per_pixel();
                let row_bytes = width as usize * bpp;
                // Round stride up to next multiple of 64
                let stride = (row_bytes + 63) & !63;
                let needed = if height > 0 {
                    (height as usize - 1) * stride + row_bytes
                } else {
                    0
                };
                self.buf.resize(needed, 0);
                PixelSliceMut::new(&mut self.buf, width, height, stride, descriptor)
                    .expect("valid buffer")
            }
        }

        let mut sink = AlignedSink { buf: Vec::new() };

        // RGBA8: width=10, bpp=4 → row_bytes=40, stride=64
        let ps = sink.demand(0, 4, 10, PixelDescriptor::RGBA8_SRGB);
        assert_eq!(ps.stride(), 64);
        assert_eq!(ps.width(), 10);
        assert_eq!(ps.rows(), 4);
        assert_eq!(ps.descriptor(), PixelDescriptor::RGBA8_SRGB);
    }
}
