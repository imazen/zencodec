//! Native animation fixtures. All source pixels and timelines are authored here;
//! codecs produce fixture bitstreams, never expected output pixels.
use std::borrow::Cow;
use zencodec::{
    animation::FrameDuration,
    decode::{DecodeJob, DecoderConfig, DynAnimationFrameDecoder},
    encode::{DynAnimationFrameEncoder, EncodeJob, EncoderConfig},
};
use zenpixels::{PixelBuffer, PixelDescriptor};

type Error = Box<dyn std::error::Error + Send + Sync>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum Format {
    Gif,
    Apng,
    Webp,
    Jxl,
    Avif,
}
impl Format {
    pub const ALL: [Self; 5] = [Self::Gif, Self::Apng, Self::Webp, Self::Jxl, Self::Avif];
    pub fn extension(self) -> &'static str {
        match self {
            Self::Gif => "gif",
            Self::Apng => "png",
            Self::Webp => "webp",
            Self::Jxl => "jxl",
            Self::Avif => "avif",
        }
    }
    pub fn encoder(self, plays: Option<u32>) -> Result<Box<dyn DynAnimationFrameEncoder>, Error> {
        match self {
            Self::Avif => {
                let mut config = zenavif::AvifEncoderConfig::new().with_lossless(true);
                *config.inner_mut() = config
                    .inner()
                    .clone()
                    .speed(10)
                    .threads(Some(1))
                    .bit_depth(zenavif::EncodeBitDepth::Eight)
                    .color_model(zenavif::EncodeColorModel::Rgb)
                    .chroma_subsampling(zenavif::EncodeChromaSubsampling::Yuv444)
                    .pixel_range(zenavif::EncodePixelRange::Full);
                config
                    .job()
                    .with_loop_count(plays)
                    .dyn_animation_frame_encoder()
            }
            Self::Jxl => zenjxl::JxlEncoderConfig::new()
                .with_lossless(true)
                .job()
                .with_loop_count(plays)
                .dyn_animation_frame_encoder(),
            Self::Gif => zengif::GifEncoderConfig::new()
                .with_dithering(0.0)
                .job()
                .with_loop_count(plays)
                .dyn_animation_frame_encoder(),
            Self::Apng => zenpng::PngEncoderConfig::new()
                .job()
                .with_loop_count(plays)
                .dyn_animation_frame_encoder(),
            Self::Webp => zenwebp::zencodec::WebpEncoderConfig::lossless()
                .job()
                .with_loop_count(plays)
                .dyn_animation_frame_encoder(),
        }
    }
    pub fn decoder(self, bytes: Vec<u8>) -> Result<Box<dyn DynAnimationFrameDecoder>, Error> {
        // Explicit RGBA output permits direct comparisons while preserving alpha.
        self.decoder_with_preference(bytes, &[PixelDescriptor::RGBA8_SRGB])
    }
    pub fn decoder_with_preference(
        self,
        bytes: Vec<u8>,
        preferred: &[PixelDescriptor],
    ) -> Result<Box<dyn DynAnimationFrameDecoder>, Error> {
        match self {
            Self::Avif => zenavif::AvifDecoderConfig::new()
                .job()
                .dyn_animation_frame_decoder(Cow::Owned(bytes), preferred),
            Self::Jxl => zenjxl::JxlDecoderConfig::new()
                .job()
                .dyn_animation_frame_decoder(Cow::Owned(bytes), preferred),
            Self::Gif => zengif::GifDecoderConfig::new()
                .job()
                .dyn_animation_frame_decoder(Cow::Owned(bytes), preferred),
            Self::Apng => zenpng::PngDecoderConfig::new()
                .job()
                .dyn_animation_frame_decoder(Cow::Owned(bytes), preferred),
            Self::Webp => zenwebp::zencodec::WebpDecoderConfig::new()
                .job()
                .dyn_animation_frame_decoder(Cow::Owned(bytes), preferred),
        }
    }
}

pub struct Fixture {
    pub name: String,
    pub frames: Vec<PixelBuffer>,
    pub durations: Vec<FrameDuration>,
    pub plays: u32,
}
impl Fixture {
    pub fn encode(&self, format: Format) -> Result<Vec<u8>, Error> {
        let mut encoder = format.encoder(Some(self.plays))?;
        for (pixels, &duration) in self.frames.iter().zip(&self.durations) {
            encoder.push_frame_timed(pixels.as_slice(), duration, None)?;
        }
        Ok(encoder.finish(None)?.into_vec())
    }
}

pub fn fixtures() -> Vec<Fixture> {
    let palette = [
        [0, 0, 0, 255],
        [255, 255, 255, 255],
        [255, 0, 0, 255],
        [0, 255, 0, 255],
        [0, 0, 255, 255],
        [127, 127, 127, 255],
        [0, 0, 0, 0],
    ];
    let mut fixtures = Vec::new();
    for (w, h) in [(1, 1), (17, 13), (257, 259)] {
        for transparent in [false, true] {
            let mut frames = Vec::new();
            for phase in if transparent {
                [6, 1, 1, 2, 6]
            } else {
                [0, 1, 1, 2, 0]
            } {
                let mut bytes = Vec::new();
                for y in 0..h {
                    for x in 0..w {
                        let index =
                            ((x / 3 + y / 5 + phase) % if transparent { 7 } else { 6 }) as usize;
                        bytes.extend_from_slice(&palette[index]);
                    }
                }
                frames
                    .push(PixelBuffer::from_vec(bytes, w, h, PixelDescriptor::RGBA8_SRGB).unwrap());
            }
            fixtures.push(Fixture {
                name: format!(
                    "{w}x{h}-{}",
                    if transparent {
                        "binary-alpha"
                    } else {
                        "opaque"
                    }
                ),
                frames,
                durations: [1, 2, 3, 1, 7]
                    .map(|n| FrameDuration::new(n, 100).unwrap())
                    .to_vec(),
                plays: 2,
            });
        }
    }
    fixtures
}
