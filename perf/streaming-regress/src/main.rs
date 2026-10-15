//! Main-vs-PR regression benchmarks for the streaming-output rollout.
//!
//! Every group runs the same workload through a codec's main branch
//! (`main`) and its PR head (`pr`), interleaved by zenbench, both on one
//! zencodec. Groups whose path is new in the PR add `pr+begin`: the same
//! encode with `begin` declared and output drained after every push, which
//! is what a streaming driver does. That row is informational, not a
//! regression check.
//!
//! The source is the photo named by `STREAMING_REGRESS_PHOTO` (a JPEG),
//! cropped from its top-left corner to each size.

use std::borrow::Cow;
use std::sync::{Arc, OnceLock};

use zenbench::prelude::*;
use zencodec::decode::{Decode, DecodeJob, DecoderConfig};
use zencodec::encode::{DynAnimationFrameEncoder, DynEncoder, EncodeJob, EncoderConfig};
use zenpixels::{PixelDescriptor, PixelSlice};

const RGB8: PixelDescriptor = PixelDescriptor::RGB8_SRGB;
const RGBA8: PixelDescriptor = PixelDescriptor::RGBA8_SRGB;
/// Rows per `push_rows` call: one JPEG MCU row at 4:2:0.
const STRIP: u32 = 16;
const IMIJ_WORKING: usize = 64 << 20;
/// Still-image sizes: a tiny one for fixed per-call cost, a moderate photo.
const SIZES: [(u32, u32); 2] = [(64, 64), (1024, 768)];
/// Animation: frames of this size, each a shifted crop of the photo.
const ANIM: (u32, u32, u32) = (320, 240, 8);

/// Builds an encoder for an image of the given width.
type MkEnc = fn(u32) -> Box<dyn DynEncoder>;
type MkAnim = fn() -> Box<dyn DynAnimationFrameEncoder>;
type Dec = fn(&[u8]) -> usize;

/// The codec-side constructors, instantiated once per side so `main` and
/// `pr` run the same builder calls.
macro_rules! side {
    ($m:ident, $jpeg:ident, $png:ident, $bmp:ident, $gif:ident, $imij:ident) => {
        pub mod $m {
            use super::*;

            fn jpeg() -> $jpeg::JpegEncoderConfig {
                $jpeg::JpegEncoderConfig::ycbcr(85.0, $jpeg::encoder::ChromaSubsampling::Quarter)
            }
            pub fn jpeg_default(_width: u32) -> Box<dyn DynEncoder> {
                jpeg().job().dyn_encoder().unwrap()
            }
            /// Effort 0, sequential: fixed Huffman tables, the streaming case.
            pub fn jpeg_fixed(_width: u32) -> Box<dyn DynEncoder> {
                jpeg()
                    .with_generic_effort(0)
                    .with_progressive(false)
                    .job()
                    .dyn_encoder()
                    .unwrap()
            }
            fn png(effort: u32) -> $png::PngEncoderConfig {
                $png::PngEncoderConfig::new()
                    .with_compression($png::Compression::Effort(effort))
                    .with_downcast($png::DowncastFlags::none())
                    .with_threading(zencodec::ThreadingPolicy::Sequential)
            }
            pub fn png_e0(_width: u32) -> Box<dyn DynEncoder> {
                png(0).job().dyn_encoder().unwrap()
            }
            pub fn png_e1(_width: u32) -> Box<dyn DynEncoder> {
                png(1).job().dyn_encoder().unwrap()
            }
            pub fn png_e5(_width: u32) -> Box<dyn DynEncoder> {
                png(5).job().dyn_encoder().unwrap()
            }
            pub fn apng_e0() -> Box<dyn DynAnimationFrameEncoder> {
                png(0).job().dyn_animation_frame_encoder().unwrap()
            }
            pub fn apng_e2() -> Box<dyn DynAnimationFrameEncoder> {
                png(2).job().dyn_animation_frame_encoder().unwrap()
            }
            pub fn apng_e7() -> Box<dyn DynAnimationFrameEncoder> {
                png(7).job().dyn_animation_frame_encoder().unwrap()
            }
            pub fn qoi(_width: u32) -> Box<dyn DynEncoder> {
                $bmp::QoiEncoderConfig::new().job().dyn_encoder().unwrap()
            }
            pub fn pnm(_width: u32) -> Box<dyn DynEncoder> {
                $bmp::PnmEncoderConfig::new().job().dyn_encoder().unwrap()
            }
            pub fn gif() -> Box<dyn DynAnimationFrameEncoder> {
                $gif::GifEncoderConfig::new()
                    .job()
                    .dyn_animation_frame_encoder()
                    .unwrap()
            }
            pub fn imij_quick(width: u32) -> Box<dyn DynEncoder> {
                $imij::codec::EncodeConfig::quick_rows(width, IMIJ_WORKING)
                    .job()
                    .dyn_encoder()
                    .unwrap()
            }
            pub fn imij_group(width: u32) -> Box<dyn DynEncoder> {
                $imij::codec::EncodeConfig::group_rows(width, IMIJ_WORKING)
                    .job()
                    .dyn_encoder()
                    .unwrap()
            }
            pub fn jpeg_decode(b: &[u8]) -> usize {
                decode($jpeg::JpegDecoderConfig::new(), b)
            }
            pub fn png_decode(b: &[u8]) -> usize {
                decode($png::PngDecoderConfig::new(), b)
            }
            pub fn qoi_decode(b: &[u8]) -> usize {
                decode($bmp::QoiDecoderConfig::new(), b)
            }
            pub fn imij_decode(b: &[u8]) -> usize {
                decode($imij::codec::DecodeConfig::default(), b)
            }
        }
    };
}

side!(
    old,
    zenjpeg_old,
    zenpng_old,
    zenbitmaps_old,
    zengif_old,
    imij_old
);
side!(
    new,
    zenjpeg_new,
    zenpng_new,
    zenbitmaps_new,
    zengif_new,
    imij_new
);

fn decode<D: DecoderConfig>(cfg: D, bytes: &[u8]) -> usize
where
    D::Error: core::fmt::Debug,
{
    let out = cfg
        .job()
        .decoder(Cow::Borrowed(bytes), &[RGB8, RGBA8])
        .unwrap()
        .decode()
        .unwrap();
    let px = out.pixels();
    px.width() as usize * px.rows() as usize
}

/// The photo as RGB8, decoded once.
struct Photo {
    width: u32,
    height: u32,
    rgb: Vec<u8>,
    rgba: Vec<u8>,
}

static PHOTO: OnceLock<Photo> = OnceLock::new();
fn photo() -> &'static Photo {
    PHOTO.get_or_init(|| {
        let path = std::env::var("STREAMING_REGRESS_PHOTO")
            .expect("set STREAMING_REGRESS_PHOTO to a JPEG photo");
        let bytes = std::fs::read(&path).unwrap();
        let out = zenjpeg_new::JpegDecoderConfig::new()
            .job()
            .decoder(Cow::Borrowed(&bytes), &[RGB8])
            .unwrap()
            .decode()
            .unwrap();
        let px = out.pixels();
        let rgb = px.contiguous_bytes().into_owned();
        let rgba = rgb
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect();
        Photo {
            width: px.width(),
            height: px.rows(),
            rgb,
            rgba,
        }
    })
}

/// Rows `y..y + rows` of the `w`-wide crop at column `x` (strided).
fn rows(x: u32, w: u32, y: u32, n: u32, rgba: bool) -> PixelSlice<'static> {
    let p = photo();
    let (buf, bpp, desc) = if rgba {
        (&p.rgba, 4, RGBA8)
    } else {
        (&p.rgb, 3, RGB8)
    };
    let stride = p.width as usize * bpp;
    let start = y as usize * stride + x as usize * bpp;
    let end = start + (n as usize - 1) * stride + w as usize * bpp;
    PixelSlice::new(&buf[start..end], w, n, stride, desc).unwrap()
}

fn one_shot(mk: MkEnc, (w, h): (u32, u32)) -> Vec<u8> {
    mk(w).encode(rows(0, w, 0, h, false)).unwrap().into_vec()
}

/// `push_rows` in strips, then `finish`. With `begin`, declares the image
/// first and drains after every push into `sink`, as a streaming driver does.
fn pushed(mk: MkEnc, (w, h): (u32, u32), begin: bool, sink: &mut Vec<u8>) -> usize {
    sink.clear();
    let mut enc = mk(w);
    if begin {
        enc.begin(w, h, RGB8).unwrap();
    }
    let mut y = 0;
    while y < h {
        let n = STRIP.min(h - y);
        enc.push_rows(rows(0, w, y, n, false)).unwrap();
        if begin {
            enc.take_output(sink);
        }
        y += n;
    }
    let tail = enc.finish().unwrap();
    sink.len() + tail.len()
}

fn animation(mk: MkAnim, rgba: bool, begin: bool, sink: &mut Vec<u8>) -> usize {
    let (w, h, frames) = ANIM;
    sink.clear();
    let mut enc = mk();
    if begin {
        let desc = if rgba { RGBA8 } else { RGB8 };
        enc.begin(w, h, desc, Some(frames)).unwrap();
    }
    for i in 0..frames {
        enc.push_frame(rows(i * 24, w, i * 16, h, rgba), 100, None)
            .unwrap();
        if begin {
            enc.take_output(sink);
        }
    }
    let tail = enc.finish(None).unwrap();
    sink.len() + tail.len()
}

/// Whether `push_rows` works on this side; main branches without row
/// encode get no `main` row bench rather than a panic mid-run.
fn rows_supported(mk: MkEnc) -> bool {
    let mut enc = mk(16);
    enc.push_rows(rows(0, 16, 0, 16, false)).is_ok()
}

fn size_name((w, h): (u32, u32)) -> String {
    format!("{w}x{h}")
}

fn still_encoders() -> Vec<(&'static str, MkEnc, MkEnc)> {
    vec![
        ("jpeg_default", old::jpeg_default, new::jpeg_default),
        ("jpeg_fixed", old::jpeg_fixed, new::jpeg_fixed),
        ("png_e0", old::png_e0, new::png_e0),
        ("png_e1", old::png_e1, new::png_e1),
        ("png_e5", old::png_e5, new::png_e5),
        ("qoi", old::qoi, new::qoi),
        ("pnm", old::pnm, new::pnm),
        ("imij_quick", old::imij_quick, new::imij_quick),
        ("imij_group", old::imij_group, new::imij_group),
    ]
}

fn bench_still_encode(suite: &mut Suite) {
    let p = photo();
    assert!(p.width >= 1024 + 8 * 24 && p.height >= 768 + 8 * 16);
    check_outputs();
    for size in SIZES {
        for (name, mk_old, mk_new) in still_encoders() {
            let pixels = u64::from(size.0) * u64::from(size.1);
            suite.group(format!("encode/{name}/{}", size_name(size)), |g| {
                g.throughput(Throughput::Elements(pixels));
                g.throughput_unit("px");
                g.bench("main", move |b| b.iter(move || one_shot(mk_old, size)));
                g.bench("pr", move |b| b.iter(move || one_shot(mk_new, size)));
            });
            let old_rows = rows_supported(mk_old);
            if !old_rows && !rows_supported(mk_new) {
                continue;
            }
            suite.group(format!("rows/{name}/{}", size_name(size)), |g| {
                g.throughput(Throughput::Elements(pixels));
                g.throughput_unit("px");
                if old_rows {
                    g.bench("main", move |b| {
                        let mut sink = Vec::new();
                        b.iter(move || pushed(mk_old, size, false, &mut sink))
                    });
                }
                g.bench("pr", move |b| {
                    let mut sink = Vec::new();
                    b.iter(move || pushed(mk_new, size, false, &mut sink))
                });
                g.bench("pr+begin", move |b| {
                    let mut sink = Vec::new();
                    b.iter(move || pushed(mk_new, size, true, &mut sink))
                });
            });
        }
    }
}

/// APNG from RGB8 frames at three efforts; GIF from opaque RGBA8.
fn animations() -> [(&'static str, MkAnim, MkAnim, bool); 4] {
    [
        ("apng_e0", old::apng_e0, new::apng_e0, false),
        ("apng_e2", old::apng_e2, new::apng_e2, false),
        ("apng_e7", old::apng_e7, new::apng_e7, false),
        ("gif", old::gif, new::gif, true),
    ]
}

fn bench_animation(suite: &mut Suite) {
    let (w, h, frames) = ANIM;
    for (name, mk_old, mk_new, rgba) in animations() {
        suite.group(format!("animation/{name}/{w}x{h}x{frames}"), |g| {
            g.throughput(Throughput::Elements(u64::from(w * h * frames)));
            g.throughput_unit("px");
            g.bench("main", move |b| {
                let mut sink = Vec::new();
                b.iter(move || animation(mk_old, rgba, false, &mut sink))
            });
            g.bench("pr", move |b| {
                let mut sink = Vec::new();
                b.iter(move || animation(mk_new, rgba, false, &mut sink))
            });
            g.bench("pr+begin", move |b| {
                let mut sink = Vec::new();
                b.iter(move || animation(mk_new, rgba, true, &mut sink))
            });
        });
    }
}

fn bench_decode(suite: &mut Suite) {
    let decoders: [(&str, MkEnc, Dec, Dec); 4] = [
        (
            "jpeg",
            new::jpeg_default,
            old::jpeg_decode,
            new::jpeg_decode,
        ),
        ("png", new::png_e5, old::png_decode, new::png_decode),
        ("qoi", new::qoi, old::qoi_decode, new::qoi_decode),
        ("imij", new::imij_group, old::imij_decode, new::imij_decode),
    ];
    for size in SIZES {
        for (name, mk, dec_old, dec_new) in decoders {
            // One file per size, encoded by the PR side; both sides decode it.
            let file: Arc<[u8]> = one_shot(mk, size).into();
            assert_eq!(dec_old(&file), dec_new(&file));
            let pixels = u64::from(size.0) * u64::from(size.1);
            suite.group(format!("decode/{name}/{}", size_name(size)), |g| {
                g.throughput(Throughput::Elements(pixels));
                g.throughput_unit("px");
                let f = file.clone();
                g.bench("main", move |b| b.iter(|| dec_old(&f)));
                let f = file.clone();
                g.bench("pr", move |b| b.iter(|| dec_new(&f)));
            });
        }
    }
}

/// Main and PR must produce the same bytes for every like-for-like encode;
/// a speed comparison between different outputs means nothing. `begin` may
/// change framing, so its output sizes are reported next to the plain ones.
fn check_outputs() {
    let report = |what: String, main: &[u8], pr: &[u8], begun: Option<usize>| {
        let begun = begun.map_or(String::new(), |b| format!(", pr+begin {b}"));
        let tag = if main == pr { "" } else { "  OUTPUTS DIFFER" };
        let (m, p) = (main.len(), pr.len());
        eprintln!("[streaming-regress] {what}: main {m}, pr {p}{begun} bytes{tag}");
    };
    for size in SIZES {
        for (name, mk_old, mk_new) in still_encoders() {
            let a = one_shot(mk_old, size);
            let b = one_shot(mk_new, size);
            let begun = rows_supported(mk_new).then(|| {
                let mut sink = Vec::new();
                pushed(mk_new, size, true, &mut sink)
            });
            report(format!("{name} {}", size_name(size)), &a, &b, begun);
        }
    }
    for (name, mk_old, mk_new, rgba) in animations() {
        let mut sink = Vec::new();
        let a = animation_bytes(mk_old, rgba);
        let b = animation_bytes(mk_new, rgba);
        let begun = animation(mk_new, rgba, true, &mut sink);
        report(format!("{name} animation"), &a, &b, Some(begun));
    }
}

/// The whole animation file, encoded without `begin`.
fn animation_bytes(mk: MkAnim, rgba: bool) -> Vec<u8> {
    let (w, h, frames) = ANIM;
    let mut enc = mk();
    for i in 0..frames {
        enc.push_frame(rows(i * 24, w, i * 16, h, rgba), 100, None)
            .unwrap();
    }
    enc.finish(None).unwrap().into_vec()
}

zenbench::main!(bench_still_encode, bench_animation, bench_decode);
