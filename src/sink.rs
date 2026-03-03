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
//! - The codec passes `width` (pixels) and `bpp` (bytes per pixel) so the
//!   sink knows the pixel dimensions.
//! - The sink returns `(buffer, stride)`:
//!   - `stride >= width * bpp`
//!   - `stride % bpp == 0` (every row starts pixel-aligned)
//!   - `buffer.len() >= (height - 1) * stride + width * bpp`
//! - The codec writes `width * bpp` bytes at offsets `[0, stride, 2*stride, ...]`
//!   within the returned buffer.
//! - When `demand()` is called again, the previous buffer has been fully
//!   written. When `decode_rows()` returns, the last buffer has been written.
//! - The pixel format matches what
//!   [`output_info()`](crate::DecodeJob::output_info) returned.

/// Receives decoded rows during streaming decode.
///
/// The codec calls [`demand`](DecodeRowSink::demand) for each strip of rows,
/// writes decoded pixels directly into the returned buffer at stride offsets,
/// then calls `demand` again for the next strip. After
/// `decode_rows()` returns, the last
/// demanded buffer has been fully written.
///
/// The sink controls the stride — it can return tightly-packed buffers
/// (stride = width × bpp) or SIMD-aligned buffers (stride padded to 64 bytes).
/// The codec respects whatever stride the sink provides.
///
/// # Object safety
///
/// This trait is object-safe. Use `&mut dyn DecodeRowSink` in generic code.
///
/// # Example implementation
///
/// ```
/// use zencodec_types::DecodeRowSink;
///
/// struct CollectSink {
///     buf: Vec<u8>,
/// }
///
/// impl DecodeRowSink for CollectSink {
///     fn demand(&mut self, _y: u32, height: u32, width: u32, bpp: usize) -> (&mut [u8], usize) {
///         let stride = width as usize * bpp; // tight packing
///         let needed = height as usize * stride;
///         self.buf.resize(needed, 0);
///         (&mut self.buf, stride)
///     }
/// }
/// ```
pub trait DecodeRowSink {
    /// Provide a mutable buffer for decoded rows `y .. y + height`.
    ///
    /// The codec passes the image `width` in pixels and `bpp` (bytes per pixel).
    /// The sink returns `(buffer, stride)` where:
    /// - `stride >= width * bpp`
    /// - `stride % bpp == 0`
    /// - `buffer.len() >= (height - 1) * stride + width * bpp`
    ///
    /// The codec writes `width * bpp` pixel bytes per row at byte offsets
    /// `[0, stride, 2*stride, ...]` within the buffer.
    ///
    /// When this method is called, any buffer returned by a previous call
    /// has been fully written with decoded pixel data.
    fn demand(&mut self, y: u32, height: u32, width: u32, bpp: usize) -> (&mut [u8], usize);
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    /// Verify the basic demand/fill/demand lifecycle with stride.
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
                bpp: usize,
            ) -> (&mut [u8], usize) {
                // Record the previous strip as completed (if any)
                if y > 0 {
                    self.completed.push((y - height, height));
                }
                let stride = width as usize * bpp;
                let needed = height as usize * stride;
                self.buf.resize(needed, 0);
                (&mut self.buf, stride)
            }
        }

        let mut sink = TestSink {
            buf: Vec::new(),
            completed: Vec::new(),
        };

        let width = 10u32;
        let bpp = 3usize; // RGB8

        // Simulate a codec writing 3 strips of 8 rows each
        for strip in 0..3u32 {
            let y = strip * 8;
            let h = 8;
            let (buf, stride) = sink.demand(y, h, width, bpp);
            assert_eq!(stride, 30);
            assert!(buf.len() >= h as usize * stride);
            // Simulate codec writing at stride offsets
            for row in 0..h as usize {
                let start = row * stride;
                for b in &mut buf[start..start + width as usize * bpp] {
                    *b = (strip + 1) as u8;
                }
            }
        }
        // After last strip, record it manually (decode_rows would have returned)
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
                bpp: usize,
            ) -> (&mut [u8], usize) {
                let stride = width as usize * bpp;
                let needed = height as usize * stride;
                self.buf.resize(needed, 0);
                (&mut self.buf, stride)
            }
        }

        fn use_sink(sink: &mut dyn DecodeRowSink) {
            let (buf, stride) = sink.demand(0, 8, 10, 3);
            assert_eq!(stride, 30);
            assert!(buf.len() >= 8 * 30);
            buf[0] = 42;
        }

        let mut sink = SimpleSink { buf: Vec::new() };
        use_sink(&mut sink);
        assert_eq!(sink.buf[0], 42);
    }

    /// Verify the lending pattern — each demand() call's borrow is independent.
    #[test]
    fn lending_borrow_pattern() {
        struct ReuseSink {
            buf: vec::Vec<u8>,
            call_count: u32,
        }
        impl DecodeRowSink for ReuseSink {
            fn demand(
                &mut self,
                _y: u32,
                height: u32,
                width: u32,
                bpp: usize,
            ) -> (&mut [u8], usize) {
                self.call_count += 1;
                let stride = width as usize * bpp;
                let needed = height as usize * stride;
                self.buf.resize(needed, 0);
                (&mut self.buf, stride)
            }
        }

        let mut sink = ReuseSink {
            buf: Vec::new(),
            call_count: 0,
        };

        // Multiple sequential borrows — each one ends before the next starts
        {
            let (buf, _stride) = sink.demand(0, 4, 10, 1);
            buf.fill(1);
        } // borrow ends

        {
            let (buf, _stride) = sink.demand(4, 4, 10, 1);
            buf.fill(2);
        } // borrow ends

        {
            let (buf, _stride) = sink.demand(8, 4, 10, 1);
            buf.fill(3);
        } // borrow ends

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
                bpp: usize,
            ) -> (&mut [u8], usize) {
                let row_bytes = width as usize * bpp;
                // Round stride up to next multiple of 64
                let stride = (row_bytes + 63) & !63;
                let needed = if height > 0 {
                    (height as usize - 1) * stride + row_bytes
                } else {
                    0
                };
                self.buf.resize(needed, 0);
                (&mut self.buf, stride)
            }
        }

        let mut sink = AlignedSink { buf: Vec::new() };

        // RGBA8: width=10, bpp=4 → row_bytes=40, stride=64
        let (buf, stride) = sink.demand(0, 4, 10, 4);
        assert_eq!(stride, 64);
        // Buffer fits: (4-1)*64 + 40 = 232
        assert!(buf.len() >= 232);
        // Verify stride % bpp == 0
        assert_eq!(stride % 4, 0);

        // RGB8: width=10, bpp=3 → row_bytes=30, stride=64
        // Note: stride=64 is NOT a multiple of bpp=3. A proper SIMD sink
        // for RGB8 would need stride=lcm(3,64)=192. This test shows the
        // sink has full control and responsibility.
        let (buf, stride) = sink.demand(0, 4, 10, 3);
        assert_eq!(stride, 64);
        assert!(buf.len() >= (3 * 64 + 30));
    }
}
