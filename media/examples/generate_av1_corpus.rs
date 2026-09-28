//! One original lossless AV1 case, encoded and decoded by Imazen code.
//! The Python driver independently regenerates source bytes and hashes artifacts.
use std::{
    error::Error,
    fs::File,
    io::{BufWriter, Write},
};
use zencodec_media::{
    av1::Av1IvfDecoder,
    av1_encode::Av1IvfEncoder,
    color::{ChromaLocation, Subsampling, YuvView},
    plane::{Plane, Samples},
    time::{TimeBase, Timestamp},
};
use zenpixels::{ChannelType, Cicp, sample::SampleEncoding};
use zenrav1e::{
    EncoderConfig,
    color::*,
    prelude::{Rational, SpeedSettings},
};

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 8 {
        return Err("usage: generate_av1_corpus BITS SAMPLING full|narrow WIDTH HEIGHT FRAMES OUTPUT.ivf DECODED.yuv".into());
    }
    let bits: u8 = args[0].parse()?;
    let (sampling, native, sx, sy) = match args[1].as_str() {
        "420" => (Subsampling::Yuv420, ChromaSampling::Cs420, 2, 2),
        "422" => (Subsampling::Yuv422, ChromaSampling::Cs422, 2, 1),
        "444" => (Subsampling::Yuv444, ChromaSampling::Cs444, 1, 1),
        "mono" => (Subsampling::Monochrome, ChromaSampling::Cs400, 1, 1),
        _ => return Err("invalid sampling".into()),
    };
    let full = match args[2].as_str() {
        "full" => true,
        "narrow" => false,
        _ => return Err("invalid range".into()),
    };
    let (w, h, count): (usize, usize, usize) =
        (args[3].parse()?, args[4].parse()?, args[5].parse()?);
    let clock = TimeBase::new(1001, 30000)?;
    let mut speed = SpeedSettings::from_preset(10);
    speed.rdo_lookahead_frames = 1;
    let cfg = EncoderConfig {
        width: w,
        height: h,
        bit_depth: bits as usize,
        chroma_sampling: native,
        pixel_range: if full {
            PixelRange::Full
        } else {
            PixelRange::Limited
        },
        color_description: Some(ColorDescription {
            color_primaries: ColorPrimaries::BT709,
            transfer_characteristics: TransferCharacteristics::BT709,
            matrix_coefficients: MatrixCoefficients::BT709,
        }),
        quantizer: 0,
        min_quantizer: 0,
        low_latency: true,
        min_key_frame_interval: 2,
        max_key_frame_interval: 2,
        time_base: Rational::new(1001, 30000),
        speed_settings: speed,
        ..EncoderConfig::default()
    };
    let mut encoder =
        Av1IvfEncoder::new(BufWriter::new(File::create(&args[6])?), cfg, clock, 32, 1)?;
    let sizes = [
        (w, h),
        (w.div_ceil(sx), h.div_ceil(sy)),
        (w.div_ceil(sx), h.div_ceil(sy)),
    ];
    for n in 0..count {
        let components: [Vec<u16>; 3] = std::array::from_fn(|p| {
            let (pw, ph) = sizes[p];
            (0..pw * ph)
                .map(|i| zencodec_media_testkit::synthetic::code(n, p, i % pw, i / pw, bits, full))
                .collect()
        });
        let encoding = SampleEncoding::new(ChannelType::U16, bits, 0)?;
        let mut planes = Vec::new();
        for p in 0..3 {
            let (pw, ph) = sizes[p];
            planes.push(Plane::new(
                Samples::U16(&components[p]),
                pw,
                ph,
                pw * 2,
                encoding,
            )?);
        }
        let view = YuvView::new(
            planes[0],
            (sampling != Subsampling::Monochrome).then_some([planes[1], planes[2]]),
            sampling,
            ChromaLocation::Unknown,
            Cicp::new(1, 1, 1, full),
        )?;
        encoder.push(view, Timestamp::new(n as i64, clock))?;
    }
    encoder.finish()?;
    let mut decoder = Av1IvfDecoder::new(
        File::open(&args[6])?,
        1 << 20,
        rav1d_safe::Settings::default(),
    )?;
    let mut output = BufWriter::new(File::create(&args[7])?);
    for n in 0..count {
        let frame = decoder.next_frame()?.ok_or("missing presentation")?;
        assert_eq!(frame.timestamp(), Some(Timestamp::new(n as i64, clock)));
        assert_eq!(frame.color(), Cicp::new(1, 1, 1, full));
        let mapped = frame.map();
        for p in 0..if sampling == Subsampling::Monochrome {
            1
        } else {
            3
        } {
            let plane = mapped.plane(p)?.ok_or("missing component")?;
            for y in 0..plane.height() {
                for x in 0..plane.width() {
                    let value = plane.code(x, y)?;
                    assert_eq!(
                        value,
                        zencodec_media_testkit::synthetic::code(n, p, x, y, bits, full)
                    );
                    if bits == 8 {
                        output.write_all(&[value as u8])?;
                    } else {
                        output.write_all(&value.to_le_bytes())?;
                    }
                }
            }
        }
    }
    assert!(decoder.next_frame()?.is_none(), "extra presentation");
    output.flush()?;
    Ok(())
}
