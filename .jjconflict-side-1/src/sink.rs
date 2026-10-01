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
//! job.push_decoder(data, &mut sink, &[])?;
//! ```
//!
//! # Lifecycle
//!
//! 1. [`begin()`](DecodeRowSink::begin) — called once with total dimensions
//!    and pixel format. The sink can pre-allocate and validate.
//! 2. [`provide_next_buffer()`](DecodeRowSink::provide_next_buffer) — called
//!    once per strip, in top-to-bottom order (`y` increases monotonically).
//!    The codec writes into the returned [`PixelSliceMut`] via `row_mut()`.
//! 3. [`finish()`](DecodeRowSink::finish) — called once after the last strip
//!    has been fully written. The sink can flush or finalize.
//!
//! `begin()` and `finish()` have default no-op implementations. Minimal
//! sinks only need to implement `provide_next_buffer()`.

use alloc::boxed::Box;

use zenpixels::{PixelDescriptor, PixelSliceMut};

/// Boxed error type for sink failures.
pub type SinkError = Box<dyn core::error::Error + Send + Sync>;

/// Receives decoded rows during streaming decode.
///
/// The codec calls [`begin`](DecodeRowSink::begin) once, then
/// [`provide_next_buffer`](DecodeRowSink::provide_next_buffer) for each strip
/// of rows. The codec writes decoded pixels directly into the returned
/// [`PixelSliceMut`], then requests the next strip. After the last strip
/// is written, the codec calls [`finish`](DecodeRowSink::finish).
///
/// The sink controls the stride — it can return tightly-packed buffers
/// (stride = width × bpp) or SIMD-aligned buffers (stride padded to 64 bytes).
/// The codec respects whatever stride the `PixelSliceMut` carries.
///
/// # Failure
///
/// Any method can return `Err(...)` to signal the decoder should stop (e.g.,
/// the sink has been cancelled, the format is incompatible, or the sink
/// can't accommodate this strip). The decoder propagates the error.
///
/// # Object safety
///
/// This trait is object-safe. Use `&mut dyn DecodeRowSink` in generic code.
///
/// # Example implementation
///
/// ```
/// use zencodec::decode::DecodeRowSink;
/// use zenpixels::{PixelSliceMut, PixelDescriptor};
///
/// struct CollectSink {
///     buf: Vec<u8>,
/// }
///
/// impl DecodeRowSink for CollectSink {
///     fn provide_next_buffer(&mut self, _y: u32, height: u32, width: u32, descriptor: PixelDescriptor) -> Result<PixelSliceMut<'_>, Box<dyn core::error::Error + Send + Sync>> {
///         let bpp = descriptor.bytes_per_pixel();
///         let stride = width as usize * bpp;
///         let needed = height as usize * stride;
///         self.buf.resize(needed, 0);
///         Ok(PixelSliceMut::new(&mut self.buf, width, height, stride, descriptor)
///             .expect("buffer sized correctly"))
///     }
/// }
/// ```
pub trait DecodeRowSink {
    /// Called once before the first strip with total output dimensions
    /// and pixel format.
    ///
    /// The sink can use this to:
    /// - Pre-allocate buffers with known total dimensions
    /// - Reject incompatible formats early, before decode work is done
    /// - Initialize processing state
    ///
    /// Return `Err(...)` to abort the decode. Default: no-op.
    fn begin(
        &mut self,
        _width: u32,
        _height: u32,
        _descriptor: PixelDescriptor,
    ) -> Result<(), SinkError> {
        Ok(())
    }

    /// Provide a mutable pixel buffer for decoded rows `y .. y + height`.
    ///
    /// The codec passes the strip `width` (pixels), `height` (rows in this
    /// strip), and `descriptor` (pixel format). The sink returns a
    /// [`PixelSliceMut`] with its chosen stride.
    ///
    /// The codec writes into the buffer via `row_mut()` for each row.
    ///
    /// When this method is called, any buffer returned by a previous call
    /// has been fully written with decoded pixel data.
    ///
    /// Return `Err(...)` to abort the decode. The error is propagated to
    /// the caller.
    fn provide_next_buffer(
        &mut self,
        y: u32,
        height: u32,
        width: u32,
        descriptor: PixelDescriptor,
    ) -> Result<PixelSliceMut<'_>, SinkError>;

    /// Called once after the last strip has been fully written.
    ///
    /// The last buffer from `provide_next_buffer()` has been completely
    /// written when this is called. The sink can flush output, finalize
    /// processing, or release resources.
    ///
    /// Return `Err(...)` to signal finalization failure. Default: no-op.
    fn finish(&mut self) -> Result<(), SinkError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;
    use alloc::string::ToString;
    use alloc::vec::Vec;

    /// Verify the full begin/provide_next_buffer/finish lifecycle.
    #[test]
    fn full_lifecycle() {
        struct TestSink {
            buf: Vec<u8>,
            began: bool,
            finished: bool,
            strips: Vec<(u32, u32)>,
        }

        impl DecodeRowSink for TestSink {
            fn begin(
                &mut self,
                _width: u32,
                _height: u32,
                _descriptor: PixelDescriptor,
            ) -> Result<(), SinkError> {
                assert!(!self.began, "begin called twice");
                self.began = true;
                Ok(())
            }

            fn provide_next_buffer(
                &mut self,
                y: u32,
                height: u32,
                width: u32,
                descriptor: PixelDescriptor,
            ) -> Result<PixelSliceMut<'_>, SinkError> {
                assert!(self.began, "provide_next_buffer before begin");
                self.strips.push((y, height));
                let bpp = descriptor.bytes_per_pixel();
                let stride = width as usize * bpp;
                let needed = height as usize * stride;
                self.buf.resize(needed, 0);
                Ok(
                    PixelSliceMut::new(&mut self.buf, width, height, stride, descriptor)
                        .expect("valid buffer"),
                )
            }

            fn finish(&mut self) -> Result<(), SinkError> {
                assert!(self.began, "finish before begin");
                assert!(!self.finished, "finish called twice");
                self.finished = true;
                Ok(())
            }
        }

        let mut sink = TestSink {
            buf: Vec::new(),
            began: false,
            finished: false,
            strips: Vec::new(),
        };

        let width = 10u32;
        let desc = PixelDescriptor::RGB8_SRGB;
        let bpp = desc.bytes_per_pixel();

        sink.begin(width, 24, desc).unwrap();

        for strip in 0..3u32 {
            let y = strip * 8;
            let h = 8;
            let mut ps = sink.provide_next_buffer(y, h, width, desc).unwrap();
            assert_eq!(ps.stride(), 30);
            for row in 0..h {
                let row_data = ps.row_mut(row);
                assert_eq!(row_data.len(), width as usize * bpp);
                row_data.fill((strip + 1) as u8);
            }
        }

        sink.finish().unwrap();

        assert!(sink.began);
        assert!(sink.finished);
        assert_eq!(sink.strips.len(), 3);
        assert_eq!(sink.strips[0], (0, 8));
        assert_eq!(sink.strips[1], (8, 8));
        assert_eq!(sink.strips[2], (16, 8));
    }

    /// Verify object safety — can use as `&mut dyn DecodeRowSink`.
    #[test]
    fn object_safe() {
        struct SimpleSink {
            buf: Vec<u8>,
        }
        impl DecodeRowSink for SimpleSink {
            fn provide_next_buffer(
                &mut self,
                _y: u32,
                height: u32,
                width: u32,
                descriptor: PixelDescriptor,
            ) -> Result<PixelSliceMut<'_>, SinkError> {
                let bpp = descriptor.bytes_per_pixel();
                let stride = width as usize * bpp;
                let needed = height as usize * stride;
                self.buf.resize(needed, 0);
                Ok(
                    PixelSliceMut::new(&mut self.buf, width, height, stride, descriptor)
                        .expect("valid buffer"),
                )
            }
        }

        fn use_sink(sink: &mut dyn DecodeRowSink) {
            sink.begin(10, 8, PixelDescriptor::RGB8_SRGB).unwrap();
            let ps = sink
                .provide_next_buffer(0, 8, 10, PixelDescriptor::RGB8_SRGB)
                .unwrap();
            assert_eq!(ps.stride(), 30);
            assert_eq!(ps.width(), 10);
            assert_eq!(ps.rows(), 8);
            sink.finish().unwrap();
        }

        let mut sink = SimpleSink { buf: Vec::new() };
        use_sink(&mut sink);
    }

    /// Verify the lending pattern — each provide_next_buffer() call's borrow is independent.
    #[test]
    fn lending_borrow_pattern() {
        struct ReuseSink {
            buf: Vec<u8>,
            call_count: u32,
        }
        impl DecodeRowSink for ReuseSink {
            fn provide_next_buffer(
                &mut self,
                _y: u32,
                height: u32,
                width: u32,
                descriptor: PixelDescriptor,
            ) -> Result<PixelSliceMut<'_>, SinkError> {
                self.call_count += 1;
                let bpp = descriptor.bytes_per_pixel();
                let stride = width as usize * bpp;
                let needed = height as usize * stride;
                self.buf.resize(needed, 0);
                Ok(
                    PixelSliceMut::new(&mut self.buf, width, height, stride, descriptor)
                        .expect("valid buffer"),
                )
            }
        }

        let mut sink = ReuseSink {
            buf: Vec::new(),
            call_count: 0,
        };

        let desc = PixelDescriptor::GRAY8_SRGB;

        // Multiple sequential borrows — each one ends before the next starts
        {
            let mut ps = sink.provide_next_buffer(0, 4, 10, desc).unwrap();
            for row in 0..4 {
                ps.row_mut(row).fill(1);
            }
        }
        {
            let mut ps = sink.provide_next_buffer(4, 4, 10, desc).unwrap();
            for row in 0..4 {
                ps.row_mut(row).fill(2);
            }
        }
        {
            let mut ps = sink.provide_next_buffer(8, 4, 10, desc).unwrap();
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
            fn provide_next_buffer(
                &mut self,
                _y: u32,
                height: u32,
                width: u32,
                descriptor: PixelDescriptor,
            ) -> Result<PixelSliceMut<'_>, SinkError> {
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
                Ok(
                    PixelSliceMut::new(&mut self.buf, width, height, stride, descriptor)
                        .expect("valid buffer"),
                )
            }
        }

        let mut sink = AlignedSink { buf: Vec::new() };

        // RGBA8: width=10, bpp=4 → row_bytes=40, stride=64
        let ps = sink
            .provide_next_buffer(0, 4, 10, PixelDescriptor::RGBA8_SRGB)
            .unwrap();
        assert_eq!(ps.stride(), 64);
        assert_eq!(ps.width(), 10);
        assert_eq!(ps.rows(), 4);
        assert_eq!(ps.descriptor(), PixelDescriptor::RGBA8_SRGB);
    }

    /// Verify sink error propagation from provide_next_buffer.
    #[test]
    fn provide_next_buffer_error() {
        struct FailSink;
        impl DecodeRowSink for FailSink {
            fn provide_next_buffer(
                &mut self,
                _y: u32,
                _height: u32,
                _width: u32,
                _descriptor: PixelDescriptor,
            ) -> Result<PixelSliceMut<'_>, SinkError> {
                Err("sink cancelled".into())
            }
        }

        let mut sink = FailSink;
        let result = sink.provide_next_buffer(0, 8, 10, PixelDescriptor::RGB8_SRGB);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "sink cancelled");
    }

    /// Verify begin() can reject incompatible formats.
    #[test]
    fn begin_rejects_format() {
        struct Rgba8OnlySink {
            buf: Vec<u8>,
        }
        impl DecodeRowSink for Rgba8OnlySink {
            fn begin(
                &mut self,
                _width: u32,
                _height: u32,
                descriptor: PixelDescriptor,
            ) -> Result<(), SinkError> {
                if descriptor != PixelDescriptor::RGBA8_SRGB {
                    return Err(format!("sink requires RGBA8, got {descriptor:?}").into());
                }
                Ok(())
            }

            fn provide_next_buffer(
                &mut self,
                _y: u32,
                height: u32,
                width: u32,
                descriptor: PixelDescriptor,
            ) -> Result<PixelSliceMut<'_>, SinkError> {
                let bpp = descriptor.bytes_per_pixel();
                let stride = width as usize * bpp;
                let needed = height as usize * stride;
                self.buf.resize(needed, 0);
                Ok(
                    PixelSliceMut::new(&mut self.buf, width, height, stride, descriptor)
                        .expect("valid buffer"),
                )
            }
        }

        let mut sink = Rgba8OnlySink { buf: Vec::new() };

        // RGB8 → rejected at begin()
        let result = sink.begin(10, 8, PixelDescriptor::RGB8_SRGB);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("RGBA8"));

        // RGBA8 → accepted
        let mut sink2 = Rgba8OnlySink { buf: Vec::new() };
        sink2.begin(10, 8, PixelDescriptor::RGBA8_SRGB).unwrap();
    }

    /// Verify finish() is called and can report errors.
    #[test]
    fn finish_error() {
        struct FinishFailSink;
        impl DecodeRowSink for FinishFailSink {
            fn provide_next_buffer(
                &mut self,
                _y: u32,
                _height: u32,
                _width: u32,
                _descriptor: PixelDescriptor,
            ) -> Result<PixelSliceMut<'_>, SinkError> {
                unreachable!("not called in this test")
            }

            fn finish(&mut self) -> Result<(), SinkError> {
                Err("flush failed".into())
            }
        }

        let mut sink = FinishFailSink;
        let result = sink.finish();
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "flush failed");
    }
}
