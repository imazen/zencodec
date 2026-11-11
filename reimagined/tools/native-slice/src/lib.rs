//! Bounded RGB8 native trials; not complete codec integrations.
#![forbid(unsafe_code)]

use next_codec::decode::{DecodeCodec, DecodeSpec, Sink};
use next_codec::simple::{WholeImage, WholeImageDecode};
use next_codec::{FormatInfo, ImageFormat, ImageInfo, Layout, Result};
use std::{borrow::Cow, hint::black_box, sync::Arc};
use zenpixels::{PixelBuffer, PixelDescriptor};

pub const ARMS: [&str; 8] = [
    "native",
    "native_probed",
    "borrowed_typed",
    "borrowed_dyn",
    "older_erased",
    "next_slice",
    "next_read_at",
    "next_inline",
];

#[derive(Clone, Copy, Debug)]
pub enum Kind {
    Qoi,
    Bmp,
    Jpeg,
}
impl Kind {
    pub fn parse(name: &str) -> Self {
        match name {
            "qoi" => Self::Qoi,
            "bmp" => Self::Bmp,
            "jpeg" => Self::Jpeg,
            _ => panic!("unknown codec"),
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Qoi => "qoi",
            Self::Bmp => "bmp",
            Self::Jpeg => "jpeg",
        }
    }
    fn format(self) -> ImageFormat {
        static QOI: FormatInfo =
            FormatInfo::new("qoi", "image/qoi", "qoi", 4, |b| b.starts_with(b"qoif"));
        static BMP: FormatInfo =
            FormatInfo::new("bmp", "image/bmp", "bmp", 2, |b| b.starts_with(b"BM"));
        static JPEG: FormatInfo = FormatInfo::new("jpeg", "image/jpeg", "jpg", 2, |b| {
            b.starts_with(&[255, 216])
        });
        ImageFormat::new(match self {
            Self::Qoi => &QOI,
            Self::Bmp => &BMP,
            Self::Jpeg => &JPEG,
        })
    }
    fn dims(self, bytes: &[u8]) -> (u32, u32) {
        match self {
            Self::Qoi => {
                let (w, h, alpha) = zenbitmaps::experiment_probe_qoi(bytes).unwrap();
                assert!(!alpha);
                (w, h)
            }
            Self::Bmp => {
                let h = zenbitmaps::probe_bmp(bytes).unwrap();
                (h.width, h.height)
            }
            Self::Jpeg => {
                let h = zenjpeg::decoder::DecodeConfig::new()
                    .read_info(bytes)
                    .unwrap();
                (h.dimensions.width, h.dimensions.height)
            }
        }
    }
    fn raw(
        self,
        bytes: &[u8],
        pulse: Option<&dyn next_codec::Pulse>,
    ) -> NativeResult<(Vec<u8>, u32, u32, usize)> {
        let stop: &dyn enough::Stop = match pulse {
            Some(p) => p,
            None => &enough::Unstoppable,
        };
        match self {
            Self::Qoi | Self::Bmp => {
                let decoded = match self {
                    Self::Qoi => zenbitmaps::decode_qoi(bytes, stop),
                    _ => zenbitmaps::decode_bmp(bytes, stop),
                }?;
                assert!(!decoded.is_borrowed(), "owned-output witness");
                assert_eq!(decoded.layout, zenbitmaps::PixelLayout::Rgb8);
                let ptr = decoded.pixels().as_ptr() as usize;
                let (w, h) = (decoded.width, decoded.height);
                Ok((decoded.experiment_into_pixels().into_owned(), w, h, ptr))
            }
            Self::Jpeg => {
                let config = zenjpeg::decoder::DecodeConfig::new();
                let decoded = match pulse {
                    Some(p) => config.decode_with_pulse(bytes, Some(p)),
                    None => config.decode(bytes, enough::Unstoppable),
                }?;
                let ptr = decoded.pixels_u8().unwrap().as_ptr() as usize;
                let (w, h) = (decoded.width(), decoded.height());
                Ok((decoded.into_pixels_u8().unwrap(), w, h, ptr))
            }
        }
    }
    pub fn native(self, bytes: &[u8]) -> PixelBuffer {
        let (pixels, w, h, _) = self.raw(bytes, None).unwrap();
        PixelBuffer::from_vec(pixels, w, h, PixelDescriptor::RGB8_SRGB).unwrap()
    }
    pub fn older(self, bytes: &[u8]) -> PixelBuffer {
        let decode = |config: &dyn old_codec::decode::DynDecoderConfig| {
            black_box(config)
                .dyn_job()
                .into_decoder(Cow::Borrowed(bytes), &[])
                .unwrap()
                .decode()
                .unwrap()
        };
        let decoded = match self {
            Self::Qoi => decode(&zenbitmaps::QoiDecoderConfig::new()),
            Self::Bmp => decode(&zenbitmaps::BmpDecoderConfig::new()),
            Self::Jpeg => decode(&zenjpeg::JpegDecoderConfig::new()),
        };
        let (w, h) = (decoded.pixels().width(), decoded.pixels().rows());
        let pixels = decoded
            .into_buffer()
            .try_into_tight_u8_vec()
            .expect("RGB byte storage");
        PixelBuffer::from_vec(pixels, w, h, PixelDescriptor::RGB8_SRGB).unwrap()
    }
}

impl WholeImageDecode for Kind {
    fn name(&self) -> &'static str {
        (*self).name()
    }
    fn formats(&self) -> &[ImageFormat] {
        &[]
    } // explicit selection only in this experiment
    fn probe(&self, bytes: &[u8]) -> Result<Option<ImageInfo>> {
        let (w, h) = self.dims(bytes);
        Ok(Some(ImageInfo::new(
            self.format(),
            w,
            h,
            Layout::Packed(PixelDescriptor::RGB8_SRGB),
        )))
    }
    fn decode(&self, bytes: &[u8], _: &ImageInfo, output: PixelDescriptor) -> Result<PixelBuffer> {
        assert_eq!(output, PixelDescriptor::RGB8_SRGB);
        Ok(self.native(bytes))
    }
}

#[derive(Default)]
struct Output {
    pixels: Option<PixelBuffer>,
    began: bool,
    ended: bool,
}
impl Sink for Output {
    fn lend(
        &mut self,
        _: u32,
        _: u32,
        _: next_codec::Pass,
    ) -> Result<zenpixels::PixelSliceMut<'_>> {
        Err(next_codec::error_msg(
            next_codec::ErrorKind::Protocol,
            "this measurement sink accepts owned frames",
        ))
    }
    fn begin(&mut self, _: &next_codec::FrameHeader) -> Result<()> {
        assert!(!self.began);
        self.began = true;
        Ok(())
    }
    fn frame(&mut self, pixels: PixelBuffer) -> Result<()> {
        assert!(self.began);
        assert!(self.pixels.replace(pixels).is_none());
        Ok(())
    }
    fn end(&mut self) -> Result<()> {
        assert!(self.pixels.is_some());
        self.ended = true;
        Ok(())
    }
}

pub fn execute(
    arm: &str,
    kind: Kind,
    file: &Arc<[u8]>,
    rt: &tokio::runtime::Runtime,
) -> PixelBuffer {
    if arm == "borrowed_typed" {
        return BorrowedSlice::decode_slice(&kind, file, &how_far::NoPulse).unwrap();
    }
    if arm == "borrowed_dyn" {
        let codec: &dyn BorrowedSlice = black_box(&kind);
        return codec.decode_slice(file, &how_far::NoPulse).unwrap();
    }
    if arm == "native" {
        return kind.native(file);
    }
    if arm == "native_probed" {
        black_box(kind.dims(file));
        return kind.native(file);
    }
    if arm == "older_erased" {
        return kind.older(file);
    }
    let native_codec = WholeImage(kind);
    let codec: &dyn DecodeCodec = black_box(&native_codec);
    let decoder = codec.decoder(&DecodeSpec::new()).unwrap();
    let mut sink = Output::default();
    match arm {
        "next_slice" => {
            next_io::decode_slice(decoder, file, &mut sink).unwrap();
        }
        "next_read_at" => {
            next_io::decode_read_at(
                decoder,
                &mut &file[..],
                &mut sink,
                &next_io::DriveOptions::new(),
            )
            .unwrap();
        }
        "next_inline" => {
            use next_io::tokio_io::{self, InMemory, Offload};
            let (_, out) = rt
                .block_on(tokio_io::decode(
                    decoder,
                    &mut InMemory(&file[..]),
                    sink,
                    Offload::Inline,
                    next_io::DriveOptions::new(),
                    next_codec::SharedPulse::default(),
                ))
                .unwrap();
            sink = out;
        }
        _ => panic!("unknown arm"),
    }
    assert!(sink.ended);
    sink.pixels.unwrap()
}

pub fn fixture(kind: Kind, size: u32) -> Arc<[u8]> {
    let mut pixels = Vec::new();
    let mut state = 0x1379_ace5u32;
    for y in 0..size {
        for x in 0..size {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            let patch = ((x / 8) ^ (y / 8)) as u8;
            pixels.extend_from_slice(&[
                state as u8 ^ patch,
                (state >> 8) as u8,
                (state >> 16) as u8,
            ]);
        }
    }
    match kind {
        Kind::Qoi => zenbitmaps::encode_qoi(
            &pixels,
            size,
            size,
            zenbitmaps::PixelLayout::Rgb8,
            enough::Unstoppable,
        )
        .unwrap(),
        Kind::Bmp => zenbitmaps::encode_bmp(
            &pixels,
            size,
            size,
            zenbitmaps::PixelLayout::Rgb8,
            enough::Unstoppable,
        )
        .unwrap(),
        Kind::Jpeg => {
            let config = zenjpeg::encoder::EncoderConfig::ycbcr(
                85.0,
                zenjpeg::encoder::ChromaSubsampling::Quarter,
            )
            .progressive(false);
            let mut encoder = config
                .encode_from_bytes(size, size, zenjpeg::encoder::PixelLayout::Rgb8Srgb)
                .unwrap();
            encoder.push_packed(&pixels, enough::Unstoppable).unwrap();
            encoder.finish().unwrap()
        }
    }
    .into()
}

pub fn qualify(kind: Kind, bytes: &Arc<[u8]>, rt: &tokio::runtime::Runtime) {
    let expected = kind.native(bytes);
    for arm in ARMS {
        let actual = execute(arm, kind, bytes, rt);
        assert_eq!(
            (actual.width(), actual.height(), actual.descriptor()),
            (expected.width(), expected.height(), expected.descriptor())
        );
        assert_eq!(
            actual.as_slice().contiguous_bytes(),
            expected.as_slice().contiguous_bytes(),
            "{} {arm}",
            kind.name()
        );
    }
    let (pixels, w, h, ptr) = kind.raw(bytes, None).unwrap();
    assert_eq!(
        pixels.as_ptr() as usize,
        ptr,
        "native ownership extraction copied"
    );
    let pixels = PixelBuffer::from_vec(pixels, w, h, PixelDescriptor::RGB8_SRGB).unwrap();
    assert_eq!(
        pixels.as_slice().row(0).as_ptr() as usize,
        ptr,
        "pixel adoption copied"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_frontends_keep_exact_samples_and_adopt_the_output_allocation() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        for kind in [Kind::Qoi, Kind::Bmp, Kind::Jpeg] {
            for size in [8, 63, 256] {
                qualify(kind, &fixture(kind, size), &rt);
            }
        }
    }
}

/// A local-only candidate entry point; no stored input/control, box or sharing.
/// This is measurement code, not a proposed complete public trait.
pub trait BorrowedSlice {
    fn decode_slice(
        &self,
        bytes: &[u8],
        pulse: &dyn next_codec::Pulse,
    ) -> NativeResult<PixelBuffer>;
}
pub type NativeResult<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
impl BorrowedSlice for Kind {
    fn decode_slice(
        &self,
        bytes: &[u8],
        pulse: &dyn next_codec::Pulse,
    ) -> NativeResult<PixelBuffer> {
        next_codec::check(pulse)?;
        let (pixels, w, h, _) = self.raw(bytes, Some(pulse))?;
        next_codec::check(pulse)?;
        Ok(PixelBuffer::from_vec(
            pixels,
            w,
            h,
            PixelDescriptor::RGB8_SRGB,
        )?)
    }
}

#[cfg(test)]
mod borrowed_pulse_controls {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Observer<'a> {
        checks: &'a AtomicUsize,
        reports: &'a AtomicUsize,
        trip: usize,
    }
    impl enough::Stop for Observer<'_> {
        fn check(&self) -> std::result::Result<(), enough::StopReason> {
            let n = self.checks.fetch_add(1, Ordering::SeqCst) + 1;
            if n >= self.trip {
                Err(enough::StopReason::TimedOut)
            } else {
                Ok(())
            }
        }
    }
    impl how_far::Report for Observer<'_> {
        fn advance(&self, n: u64) {
            self.reports.fetch_add(n as usize, Ordering::SeqCst);
        }
    }
    impl next_codec::Pulse for Observer<'_> {
        fn split(
            &self,
            _: how_far::Execution,
            _: &[how_far::PhaseSpec<'_>],
        ) -> std::result::Result<Vec<how_far::Child<'_>>, how_far::PlanError> {
            Err(how_far::PlanError::Unsupported)
        }
    }
    #[test]
    fn a_nonshareable_borrowed_pulse_reaches_native_decode() {
        for kind in [Kind::Qoi, Kind::Bmp, Kind::Jpeg] {
            let file = fixture(kind, 63);
            let checks = AtomicUsize::new(0);
            let reports = AtomicUsize::new(0);
            let pulse = Observer {
                checks: &checks,
                reports: &reports,
                trip: usize::MAX,
            };
            assert!(matches!(
                next_codec::Pulse::share(&pulse),
                Err(how_far::PlanError::NotShareable)
            ));
            let codec: &dyn BorrowedSlice = &kind;
            codec.decode_slice(&file, &pulse).unwrap();
            assert!(
                checks.load(Ordering::SeqCst) > 2,
                "native must check beyond the two wrapper boundaries"
            );
            if matches!(kind, Kind::Jpeg) {
                assert!(
                    reports.load(Ordering::SeqCst) > 0,
                    "native JPEG reports progress"
                );
            }
            let checks = AtomicUsize::new(0);
            let stopped = Observer {
                checks: &checks,
                reports: &reports,
                trip: 3,
            };
            let error = codec.decode_slice(&file, &stopped).unwrap_err();
            match kind {
                Kind::Qoi | Kind::Bmp => assert!(matches!(
                    error
                        .downcast_ref::<zenbitmaps::At<zenbitmaps::BitmapError>>()
                        .unwrap()
                        .error(),
                    zenbitmaps::BitmapError::Cancelled(enough::StopReason::TimedOut)
                )),
                Kind::Jpeg => assert!(matches!(
                    error
                        .downcast_ref::<zenjpeg::decoder::Error>()
                        .unwrap()
                        .kind(),
                    zenjpeg::decoder::ErrorKind::Cancelled(enough::StopReason::TimedOut)
                )),
            }
        }
    }
}
