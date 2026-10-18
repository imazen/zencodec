//! Streaming encode output through dyn dispatch. `DynEncoder` and
//! `DynAnimationFrameEncoder` forward `begin`, `pending_output`,
//! `consume_output` and `take_output`, so a codec-agnostic pipeline streams
//! exactly what the typed encoder does. A shim that dropped one of them would
//! still produce a correct file, from `finish`, and no conformance check would
//! notice: these tests require bytes before `finish` as well.

use zencodec::encode::{
    AnimationFrameEncoder, AnimationOutputTiming, DynEncoderConfig, EncodeJob, Encoder,
    EncoderConfig, OutputTiming,
};
use zencodec_testkit::reference::ReferenceEncoderConfig;
use zenpixels::{PixelDescriptor, PixelSlice};

const DESC: PixelDescriptor = PixelDescriptor::RGBA8_SRGB;

fn pixels(w: u32, h: u32, seed: u8) -> Vec<u8> {
    (0..w as usize * h as usize * 4)
        .map(|i| (i as u8).wrapping_mul(7).wrapping_add(seed))
        .collect()
}

#[test]
fn dyn_encoder_streams_what_the_typed_encoder_does() {
    let (w, h, strip) = (16u32, 32u32, 4u32);
    let stride = w as usize * 4;
    let px = pixels(w, h, 3);
    let rows = |y: u32| {
        PixelSlice::new(
            &px[y as usize * stride..][..strip as usize * stride],
            w,
            strip,
            stride,
            DESC,
        )
        .unwrap()
    };
    let mut typed = ReferenceEncoderConfig::new().job().encoder().unwrap();
    assert_eq!(typed.begin(w, h, DESC).unwrap(), OutputTiming::streaming(0));
    let mut want = Vec::new();
    for y in (0..h).step_by(strip as usize) {
        typed.take_output(&mut want);
        typed.push_rows(rows(y)).unwrap();
    }
    typed.take_output(&mut want);
    want.extend_from_slice(typed.finish().unwrap().data());

    for take in [false, true] {
        let cfg = ReferenceEncoderConfig::new();
        let dynamic: &dyn DynEncoderConfig = &cfg;
        let mut enc = dynamic.dyn_job().into_encoder().unwrap();
        assert_eq!(enc.begin(w, h, DESC).unwrap(), OutputTiming::streaming(0));
        assert!(
            !enc.pending_output().is_empty(),
            "the header is pending after begin"
        );
        let mut out = Vec::new();
        let mut drain = |enc: &mut Box<dyn zencodec::encode::DynEncoder>, out: &mut Vec<u8>| {
            if take {
                enc.take_output(out);
            } else {
                let n = enc.pending_output().len();
                out.extend_from_slice(enc.pending_output());
                enc.consume_output(n + 1); // over-consume clamps
            }
            assert!(enc.pending_output().is_empty());
        };
        let mut header = 0;
        for y in (0..h).step_by(strip as usize) {
            drain(&mut enc, &mut out);
            if y == 0 {
                header = out.len();
            }
            enc.push_rows(rows(y)).unwrap();
        }
        drain(&mut enc, &mut out);
        assert!(
            out.len() > header,
            "rows streamed before finish (take_output: {take})"
        );
        out.extend_from_slice(enc.finish().unwrap().data());
        assert_eq!(out, want, "take_output: {take}");
    }
}

#[test]
fn dyn_animation_encoder_streams_what_the_typed_encoder_does() {
    let (w, h) = (12u32, 8u32);
    let frames: Vec<Vec<u8>> = (0..3).map(|i| pixels(w, h, i * 40)).collect();
    let frame = |i: usize| PixelSlice::new(&frames[i], w, h, w as usize * 4, DESC).unwrap();
    let mut typed = ReferenceEncoderConfig::new()
        .job()
        .with_loop_count(Some(0))
        .animation_frame_encoder()
        .unwrap();
    let declared = Some(frames.len() as u32);
    assert_eq!(
        typed.begin(w, h, DESC, declared).unwrap(),
        AnimationOutputTiming::streaming(0)
    );
    let mut want = Vec::new();
    for i in 0..frames.len() {
        typed.take_output(&mut want);
        typed.push_frame(frame(i), 40, None).unwrap();
    }
    typed.take_output(&mut want);
    want.extend_from_slice(typed.finish(None).unwrap().data());

    let cfg = ReferenceEncoderConfig::new();
    let dynamic: &dyn DynEncoderConfig = &cfg;
    let mut job = dynamic.dyn_job();
    job.set_loop_count(Some(0));
    let mut a = job.into_animation_frame_encoder().unwrap();
    assert_eq!(
        a.begin(w, h, DESC, declared).unwrap(),
        AnimationOutputTiming::streaming(0)
    );
    let mut out = Vec::new();
    let mut header = 0;
    for i in 0..frames.len() {
        a.take_output(&mut out);
        if i == 0 {
            header = out.len();
        }
        assert!(a.pending_output().is_empty());
        a.push_frame(frame(i), 40, None).unwrap();
    }
    let n = a.pending_output().len();
    out.extend_from_slice(a.pending_output());
    a.consume_output(n);
    assert!(out.len() > header, "frames streamed before finish");
    out.extend_from_slice(a.finish(None).unwrap().data());
    assert_eq!(out, want);
}
