#![no_main]
use libfuzzer_sys::fuzz_target;
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
    prelude::{ChromaSampling, SpeedSettings},
};

fn code(data: &[u8], frame: usize, plane: usize, x: usize, y: usize, bits: u8) -> u16 {
    let index = (x + 37 * y + 71 * plane + 97 * frame) % (data.len() - 8);
    let a = u16::from(data[8 + index]);
    let b = u16::from(data[8 + (index + 1) % (data.len() - 8)]);
    ((a | (b << 8)).wrapping_add((frame * 131 + plane * 43) as u16)) & ((1_u16 << bits) - 1)
}

fuzz_target!(|data: &[u8]| {
    if data.len() < 9 {
        return;
    }
    let (w, h) = (1 + usize::from(data[0] % 32), 1 + usize::from(data[1] % 32));
    let bits = [8, 10, 12][usize::from(data[2] % 3)];
    let (sampling, native, sx, sy) = match data[3] % 4 {
        0 => (Subsampling::Monochrome, ChromaSampling::Cs400, 1, 1),
        1 => (Subsampling::Yuv420, ChromaSampling::Cs420, 2, 2),
        2 => (Subsampling::Yuv422, ChromaSampling::Cs422, 2, 1),
        _ => (Subsampling::Yuv444, ChromaSampling::Cs444, 1, 1),
    };
    let full = data[4] & 1 != 0;
    let shift = if data[4] & 2 != 0 { 16 - bits } else { 0 };
    let count = 1 + usize::from(data[5] % 8);
    let mut speed = SpeedSettings::from_preset([6, 8, 10][usize::from(data[6] % 3)]);
    speed.rdo_lookahead_frames = 1;
    let config = EncoderConfig {
        width: w,
        height: h,
        bit_depth: usize::from(bits),
        chroma_sampling: native,
        pixel_range: if full {
            zenrav1e::color::PixelRange::Full
        } else {
            zenrav1e::color::PixelRange::Limited
        },
        color_description: None,
        quantizer: 0,
        min_quantizer: 0,
        low_latency: data[7] & 1 != 0,
        min_key_frame_interval: 8,
        max_key_frame_interval: 8,
        max_pixel_count: 4096,
        speed_settings: speed,
        ..EncoderConfig::default()
    };
    let clock = TimeBase::new(1001, 30000).unwrap();
    let sizes = [
        (w, h),
        (w.div_ceil(sx), h.div_ceil(sy)),
        (w.div_ceil(sx), h.div_ceil(sy)),
    ];
    let encoding = SampleEncoding::new(ChannelType::U16, bits, shift).unwrap();
    let mut encoder = Av1IvfEncoder::new(Vec::new(), config, clock, 32, 1).unwrap();
    for n in 0..count {
        let planes: [Vec<u16>; 3] = std::array::from_fn(|p| {
            let (pw, ph) = sizes[p];
            let padding = 0xa55a & !(((1_u16 << bits) - 1) << shift);
            (0..pw * ph)
                .map(|i| (code(data, n, p, i % pw, i / pw, bits) << shift) | padding)
                .collect()
        });
        let views: [Plane<'_>; 3] = std::array::from_fn(|p| {
            Plane::new(
                Samples::U16(&planes[p]),
                sizes[p].0,
                sizes[p].1,
                sizes[p].0 * 2,
                encoding,
            )
            .unwrap()
        });
        let view = YuvView::new(
            views[0],
            (sampling != Subsampling::Monochrome).then_some([views[1], views[2]]),
            sampling,
            ChromaLocation::Unknown,
            Cicp::new(2, 2, 2, full),
        )
        .unwrap();
        encoder
            .push(view, Timestamp::new((n * n) as i64 - 3, clock))
            .unwrap();
    }
    let encoded = encoder.finish().unwrap();
    let mut settings = rav1d_safe::Settings::default();
    settings.frame_size_limit = 4096;
    let mut decoder = Av1IvfDecoder::new(encoded.as_slice(), 1 << 20, settings).unwrap();
    for n in 0..count {
        let frame = decoder.next_frame().unwrap().unwrap();
        assert_eq!((frame.width() as usize, frame.height() as usize), (w, h));
        assert_eq!(frame.bit_depth(), bits);
        assert_eq!(
            frame.timestamp(),
            Some(Timestamp::new((n * n) as i64 - 3, clock))
        );
        let mapped = frame.map();
        for p in 0..if sampling == Subsampling::Monochrome {
            1
        } else {
            3
        } {
            let plane = mapped.plane(p).unwrap().unwrap();
            assert_eq!((plane.width(), plane.height()), sizes[p]);
            for y in 0..plane.height() {
                for x in 0..plane.width() {
                    assert_eq!(plane.code(x, y).unwrap(), code(data, n, p, x, y, bits));
                }
            }
        }
    }
    assert!(decoder.next_frame().unwrap().is_none());
});
