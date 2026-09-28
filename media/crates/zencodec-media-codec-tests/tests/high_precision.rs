use std::borrow::Cow;
use zencodec::{
    animation::FrameDuration,
    decode::{DecodeJob, DecoderConfig, DynAnimationFrameDecoder},
    encode::{DynAnimationFrameEncoder, EncodeJob, EncoderConfig},
};
use zencodec_media::animation::{AnimationLimits, TimingPolicy, transcode_dyn};
use zencodec_media_codec_tests::Format;
use zenpixels::{Cicp, ColorPrimaries, PixelBuffer, PixelDescriptor, TransferFunction};

type Error = Box<dyn std::error::Error + Send + Sync>;
const FORMATS: [Format; 3] = [Format::Apng, Format::Jxl, Format::Avif];

fn encoder(
    format: Format,
    bits: u8,
    plays: Option<u32>,
) -> Result<Box<dyn DynAnimationFrameEncoder>, Error> {
    if format != Format::Avif {
        return format.encoder(plays);
    }
    let mut config = zenavif::AvifEncoderConfig::new().with_lossless(true);
    *config.inner_mut() = config
        .inner()
        .clone()
        .speed(10)
        .threads(Some(1))
        .bit_depth(if bits == 10 {
            zenavif::EncodeBitDepth::Ten
        } else {
            zenavif::EncodeBitDepth::Twelve
        })
        .color_model(zenavif::EncodeColorModel::Rgb)
        .chroma_subsampling(zenavif::EncodeChromaSubsampling::Yuv444)
        .pixel_range(zenavif::EncodePixelRange::Full);
    config
        .job()
        .with_loop_count(plays)
        .dyn_animation_frame_encoder()
}
fn decoder(
    format: Format,
    bytes: Vec<u8>,
    desc: PixelDescriptor,
) -> Result<Box<dyn DynAnimationFrameDecoder>, Error> {
    match format {
        Format::Apng => zenpng::PngDecoderConfig::new()
            .job()
            .dyn_animation_frame_decoder(Cow::Owned(bytes), &[desc]),
        Format::Jxl => zenjxl::JxlDecoderConfig::new()
            .job()
            .dyn_animation_frame_decoder(Cow::Owned(bytes), &[desc]),
        Format::Avif => zenavif::AvifDecoderConfig::new()
            .job()
            .dyn_animation_frame_decoder(Cow::Owned(bytes), &[desc]),
        _ => unreachable!(),
    }
}

#[test]
fn every_high_precision_pair_preserves_native_codes_alpha_color_and_fractional_time() {
    for bits in [10, 12] {
        for (cp, tf) in [
            (ColorPrimaries::Bt709, TransferFunction::Srgb),
            (ColorPrimaries::Bt709, TransferFunction::Linear),
            (ColorPrimaries::Bt2020, TransferFunction::Pq),
            (ColorPrimaries::Bt2020, TransferFunction::Hlg),
        ] {
            let desc = PixelDescriptor::RGBA16_SRGB
                .with_primaries(cp)
                .with_transfer(tf);
            let cicp = Cicp::new(cp.to_cicp().unwrap(), tf.to_cicp().unwrap(), 0, true);
            // Full-range U16 codes expanded by bit replication, the native AVIF
            // decoder's documented normalization. All original low bits matter.
            let frames: Vec<_> = [0, 1, 1]
                .into_iter()
                .map(|phase| {
                    let bytes = (0..17 * 13 * 4)
                        .flat_map(|i| {
                            let code = ((i * 73 + phase * 31) & ((1 << bits) - 1)) as u16;
                            ((code << (16 - bits)) | (code >> (2 * bits - 16))).to_ne_bytes()
                        })
                        .collect();
                    PixelBuffer::from_vec(bytes, 17, 13, desc)
                        .unwrap()
                        .with_cicp(cicp)
                })
                .collect();
            let times = [1001, 2002, 1001].map(|n| FrameDuration::new(n, 30000).unwrap());
            for source in FORMATS {
                let mut enc = encoder(source, bits, Some(2)).unwrap();
                for (frame, duration) in frames.iter().zip(times) {
                    enc.push_frame_timed(frame.as_slice(), duration, None)
                        .unwrap();
                }
                let bytes = enc.finish(None).unwrap().into_vec();
                for target in FORMATS {
                    let label = format!("{source:?} -> {target:?} bits={bits} {tf:?}");
                    let mut dec = decoder(source, bytes.clone(), desc).unwrap();
                    let (output, report) = transcode_dyn(
                        dec.as_mut(),
                        |_, plays| encoder(target, bits, plays),
                        TimingPolicy::Exact,
                        AnimationLimits::new(3, 100_000).unwrap(),
                        None,
                    )
                    .unwrap_or_else(|e| panic!("{label}: {e}"));
                    assert_eq!(report.changed_durations(), 0, "{label}");
                    let mut decoded = decoder(target, output.into_vec(), desc).unwrap();
                    assert_eq!(decoded.loop_count(), Some(2), "{label}");
                    for (expected, duration) in frames.iter().zip(times) {
                        let frame = decoded.render_next_frame_owned(None).unwrap().unwrap();
                        assert_eq!(frame.duration(), duration, "{label}");
                        let actual = frame.pixels();
                        assert_eq!(actual.descriptor(), desc, "{label}");
                        let context = actual.color_context().unwrap();
                        let current = context.cicp.or_else(|| {
                            context.icc.as_deref().and_then(|icc| {
                                zenpixels::icc::extract_cicp(icc).or_else(|| {
                                    zenpixels::icc::identify_common(icc).and_then(|id| id.to_cicp())
                                })
                            })
                        });
                        assert_eq!(
                            current,
                            Some(cicp),
                            "{label}: ICC length {:?}",
                            context.icc.as_ref().map(|i| i.len())
                        );
                        for y in 0..13 {
                            assert_eq!(
                                actual.row(y),
                                expected.as_slice().row(y),
                                "{label} row {y}"
                            );
                        }
                    }
                    assert!(decoded.render_next_frame_owned(None).unwrap().is_none());
                }
            }
        }
    }
}
