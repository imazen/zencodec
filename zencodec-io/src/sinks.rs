//! Sinks shared by drivers and tests.

use alloc::format;
use alloc::vec::Vec;

use zencodec::decode::{
    BatchMeta, DecodeRowSink, RowMetadata, RowStrip, SinkError, StripDimensions,
};
use zenpixels::{
    ByteStorage, ByteStorageMut, PixelBuf, PixelBufParts, PixelFormat, PixelView, PixelViewMut,
};

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
    data: Option<PixelBufParts>,
    width: u32,
    height: u32,
    rows_written: u64,
    covered_rows: Vec<u64>,
    unique_rows: u32,
    finished: bool,
    metadata: Option<RowMetadata>,
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
    pub fn descriptor(&self) -> Option<PixelFormat> {
        self.data.as_ref().map(|parts| parts.format.clone())
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
            .map_or(&[], |buffer| buffer.storage.as_bytes())
    }

    /// A zero-copy view of the collected image.
    pub fn as_pixel_slice(&self) -> Option<PixelView<'_>> {
        let parts = self.data.as_ref()?;
        let view = PixelView::new(
            &parts.storage.as_bytes()[parts.byte_offset..],
            parts.width,
            parts.height,
            parts.stride_bytes,
            &parts.format,
        )
        .ok()?;
        Some(match (&self.metadata, self.finished) {
            (Some(metadata), true) => metadata.attach(view),
            _ => view,
        })
    }

    /// Take the finished image without copying its pixels or allocating.
    ///
    /// Returns `None` before [`DecodeRowSink::finish`] succeeds. Call this only
    /// after the driver returns success: a sink's finish is not proof that the
    /// decoder's later validation succeeded. Pixel evidence and provenance are
    /// retained. Container metadata remains in the decoder's completion details.
    pub fn into_buffer(self) -> Option<PixelBuf> {
        let parts = self.finished.then_some(self.data).flatten()?;
        let buffer = PixelBuf::from_parts(parts).ok()?;
        Some(self.metadata.unwrap_or_default().attach_owned(buffer))
    }

    /// Copy the collected image into an owned [`PixelBuf`].
    pub fn to_buffer(&self) -> Option<PixelBuf> {
        self.as_pixel_slice()?.to_tight_buf().ok()
    }
}

impl core::fmt::Debug for CollectSink {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CollectSink")
            .field("width", &self.width)
            .field("height", &self.height)
            .field(
                "layout",
                &self.data.as_ref().map(|parts| parts.format.layout),
            )
            .field("rows_written", &self.rows_written)
            .field("finished", &self.finished)
            .finish_non_exhaustive()
    }
}

impl DecodeRowSink for CollectSink {
    fn begin(&mut self, width: u32, height: u32, descriptor: PixelFormat) -> Result<(), SinkError> {
        if self.data.is_some() {
            return Err("CollectSink: begin called twice".into());
        }
        // Leaves state unchanged on failure. Allocates the format's own sample type, so
        // the rows are aligned whatever the allocator returns for bytes, and the
        // decoded buffer can move to a typed owner without a copy.
        let buffer = PixelBuf::new(width, height, descriptor)?;
        let words = usize::try_from(u64::from(height).div_ceil(64))
            .map_err(|_| SinkError::from("CollectSink: coverage too large"))?;
        let mut coverage = Vec::new();
        coverage.try_reserve_exact(words)?;
        coverage.resize(words, 0);
        self.covered_rows = coverage;
        self.data = Some(buffer.into_parts());
        self.width = width;
        self.height = height;
        Ok(())
    }

    fn provide_next_buffer(
        &mut self,
        strip: RowStrip,
        descriptor: PixelFormat,
    ) -> Result<PixelViewMut<'_>, SinkError> {
        self.provide_buffer_with_metadata(
            BatchMeta::final_rows(strip.y),
            StripDimensions::new(strip.width, strip.height),
            descriptor,
            &RowMetadata::default(),
        )
    }

    fn provide_buffer_with_metadata(
        &mut self,
        meta: BatchMeta,
        size: StripDimensions,
        descriptor: PixelFormat,
        metadata: &RowMetadata,
    ) -> Result<PixelViewMut<'_>, SinkError> {
        let StripDimensions { width, height } = size;
        if !meta.is_final {
            return Err("CollectSink: provisional rows are not admitted".into());
        }
        let y = meta.y;
        if self.finished {
            return Err("CollectSink: rows after finish".into());
        }
        let Some(parts) = self.data.as_mut() else {
            return Err("CollectSink: rows before begin".into());
        };
        if width != self.width || descriptor != parts.format {
            return Err(format!(
                "CollectSink: strip {width}x{height} {:?} does not match image {}x{} {:?}",
                descriptor.layout, self.width, self.height, parts.format.layout
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
        let start = y as usize * parts.stride_bytes;
        let rows = PixelViewMut::new(
            &mut parts.storage.as_bytes_mut()[start..],
            width,
            height,
            parts.stride_bytes,
            &parts.format,
        )?;
        if height != 0 {
            if let Some(prior) = &mut self.metadata {
                prior.merge(metadata)?;
            } else {
                self.metadata = Some(metadata.clone());
            }
        }
        for row in y..y + height {
            let word = &mut self.covered_rows[(row / 64) as usize];
            let bit = 1u64 << (row % 64);
            if *word & bit == 0 {
                self.unique_rows += 1;
                *word |= bit;
            }
        }
        self.rows_written += height as u64;
        Ok(rows.with_hdr(metadata.hdr))
    }

    fn finish(&mut self) -> Result<(), SinkError> {
        if self.finished {
            return Err("CollectSink: finish called twice".into());
        }
        if self.data.is_none() {
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
        sink.begin(1, 1, PixelFormat::GRAY8_SRGB).unwrap();
        sink.provide_next_buffer(RowStrip::new(0, 1, 1), PixelFormat::GRAY8_SRGB)
            .unwrap();
        sink.finish().unwrap();
        assert!(
            sink.provide_next_buffer(RowStrip::new(0, 1, 1), PixelFormat::GRAY8_SRGB)
                .is_err()
        );
    }

    #[test]
    fn completion_requires_unique_coverage_and_closes_writes() {
        let mut sink = CollectSink::new();
        sink.begin(1, 65, PixelFormat::GRAY8_SRGB).unwrap();
        for _ in 0..65 {
            sink.provide_next_buffer(RowStrip::new(0, 1, 1), PixelFormat::GRAY8_SRGB)
                .unwrap();
        }
        assert_eq!(sink.rows_written(), 65);
        assert!(sink.finish().is_err());
        assert!(!sink.finished());
        // Out-of-order and overlapping delivery remain valid.
        sink.provide_next_buffer(RowStrip::new(32, 1, 33), PixelFormat::GRAY8_SRGB)
            .unwrap();
        sink.provide_next_buffer(RowStrip::new(1, 1, 32), PixelFormat::GRAY8_SRGB)
            .unwrap();
        sink.finish().unwrap();
        assert!(
            sink.provide_next_buffer(RowStrip::new(0, 1, 1), PixelFormat::GRAY8_SRGB)
                .is_err()
        );
        assert!(sink.into_buffer().is_some());
    }

    #[test]
    fn finished_storage_moves_without_copy_for_integer_and_float_rows() {
        for descriptor in [
            PixelFormat::RGB8_SRGB,
            PixelFormat::RGB16_SRGB,
            PixelFormat::RGBAF32_LINEAR,
        ] {
            let mut sink = CollectSink::new();
            sink.begin(3, 2, descriptor.clone()).unwrap();
            let original = sink.bytes().as_ptr();
            for y in 0..2 {
                let mut rows = sink
                    .provide_next_buffer(RowStrip::new(y, 3, 1), descriptor.clone())
                    .unwrap();
                rows.row_mut(0).fill(37 + y as u8);
            }
            sink.finish().unwrap();
            let buffer = sink.into_buffer().unwrap();
            assert_eq!(buffer.as_view().as_strided_bytes().as_ptr(), original);
            assert_eq!(buffer.format().clone(), descriptor);
            for y in 0..2 {
                assert!(buffer.as_view().row(y).iter().all(|b| *b == 37 + y as u8));
            }
        }
    }

    #[test]
    fn unfinished_collectors_do_not_yield_owned_completion() {
        assert!(CollectSink::new().into_buffer().is_none());
        let mut sink = CollectSink::new();
        sink.begin(2, 1, PixelFormat::GRAY8_SRGB).unwrap();
        sink.provide_next_buffer(RowStrip::new(0, 2, 1), PixelFormat::GRAY8_SRGB)
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
            sink.begin(u32::MAX, u32::MAX, PixelFormat::GRAY8_SRGB)
                .is_err()
        );
        assert_eq!(sink.descriptor(), None);
        assert_eq!((sink.width(), sink.height()), (0, 0));
        assert!(sink.bytes().is_empty());
        assert!(!sink.finished());
        sink.begin(3, 2, PixelFormat::GRAY8_SRGB).unwrap();
        for y in 0..2 {
            sink.provide_next_buffer(RowStrip::new(y, 3, 1), PixelFormat::GRAY8_SRGB)
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
            sink.begin(u32::MAX, u32::MAX, PixelFormat::RGBAF32_LINEAR)
                .is_err()
        );
        assert!(sink.bytes().is_empty());
        assert_eq!(sink.descriptor(), None);
    }
}

#[cfg(test)]
mod metadata_tests {
    use super::*;
    use zencodec::sample::ExactDepth;
    use zenpixels::{ColorOrigin, HdrMetadata, SampleFacts, SampleType, SignalRange};

    #[test]
    fn collected_strip_facts_cover_both_grids_and_keep_original_provenance() {
        let spec = PixelFormat::GRAY16_SRGB;
        let origin = ColorOrigin::from_cicp(zencodec::Cicp::DISPLAY_P3);
        let hdr = HdrMetadata {
            content_light_level: Some(zencodec::ContentLightLevel::new(1111, 227)),
            mastering: None,
        };
        let mut sink = CollectSink::new();
        sink.begin(1, 2, spec.clone()).unwrap();
        // Nonoverlapping five-bit and three-bit full-range code grids first
        // share a 15-bit grid: lcm(5, 3). The loan order is deliberately reversed.
        for (row, bits, sample) in [(1, 3, 9362u16), (0, 5, 2114u16)] {
            let mut buffer = PixelBuf::from_typed(alloc::vec![sample], 1, 1, spec.clone())
                .unwrap()
                .with_origin(origin.clone())
                .with_hdr(hdr);
            buffer.assume_facts(SampleFacts::NONE.with_depth(
                ExactDepth::new(SampleType::U16, SignalRange::Full, bits, 16).unwrap(),
            ));
            zencodec::helpers::copy_rows_to_sink(
                &mut sink,
                BatchMeta::final_rows(row),
                &buffer.as_view(),
            )
            .unwrap();
        }
        assert_eq!(sink.as_pixel_slice().unwrap().facts(), SampleFacts::NONE);
        sink.finish().unwrap();
        let view = sink.as_pixel_slice().unwrap();
        let depth = view.facts().depth.unwrap();
        assert_eq!((depth.color_bits(), depth.alpha_bits()), (15, 16));
        assert!(depth.holds_for(&view).unwrap());
        assert_eq!(view.row(0), 2114u16.to_ne_bytes());
        assert_eq!(view.row(1), 9362u16.to_ne_bytes());
        assert_eq!(view.origin(), Some(&origin));
        assert_eq!(view.hdr(), hdr);
        let address = sink.bytes().as_ptr();
        let output = sink.into_buffer().unwrap();
        assert_eq!(output.as_view().row(0).as_ptr(), address);
        assert_eq!(output.facts().depth, Some(depth));
        assert_eq!(output.origin(), &origin);
        assert_eq!(output.hdr(), hdr);
    }
}
