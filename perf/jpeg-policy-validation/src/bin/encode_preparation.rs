//! Native allocation probe: profile with heaptrack; this program has no timers.
//! No source raster is allocated or read, and no encoded output is published.
//! Native buffer allocations are the subject of this probe.
#![forbid(unsafe_code)]

use std::{error::Error, hint::black_box, io};
use zencodec::{
    CodecSet, ImageFormat,
    encode::{EncodeJob, EncoderConfig, Fidelity},
    operation::{EncodeRequest, PolicySet},
    storage::StorageBudget,
};
use zencodec_io::row_encode::{self, Description, Options, Source, Stage};
use zenjpeg::JpegEncoderConfig;
use zenpixels::{PixelDescriptor, PixelSliceMut};

const WIDTH: u32 = 2048;
const HEIGHT: u32 = 2048;

struct Unread;
impl Source for Unread {
    type Error = io::Error;
    fn description(&self) -> Description {
        Description::new(WIDTH, HEIGHT, PixelDescriptor::RGB8_SRGB)
    }
    fn read_rows(
        &mut self,
        _: u32,
        _: PixelSliceMut<'_>,
        _: &dyn zencodec::Pulse,
    ) -> Result<(), Self::Error> {
        panic!("refused preparation must not request pixels")
    }
}
struct Unwritten;
impl io::Write for Unwritten {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        panic!("refused preparation must not publish bytes")
    }
    fn flush(&mut self) -> io::Result<()> {
        panic!("refused preparation must not touch the destination")
    }
}

fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 4 {
        return Err(
            "usage: encode_preparation prepare|construct|begin|refuse 1|4 fixed|progressive".into(),
        );
    }
    let count: usize = args[2].parse()?;
    if ![1, 4].contains(&count) {
        return Err("only measured counts 1 and 4 are permitted".into());
    }
    let config = match args[3].as_str() {
        "fixed" => JpegEncoderConfig::new()
            .with_progressive(false)
            .with_huffman(false),
        "progressive" => JpegEncoderConfig::new()
            .with_progressive(true)
            .with_huffman(true),
        _ => return Err("expected fixed or progressive".into()),
    };
    #[cfg(feature = "native-preflight")]
    if args[1].starts_with("native-") {
        use zencodec::encode::Encoder;
        let jobs: Vec<_> = (0..count).map(|_| config.clone().job()).collect();
        match args[1].as_str() {
            "native-prepare" => {
                black_box(jobs);
            }
            "native-plan" => {
                for job in &jobs {
                    let timing = job.plan_rows(
                        WIDTH,
                        HEIGHT,
                        PixelDescriptor::RGB8_SRGB,
                        None,
                        zencodec::output_patch::OutputAccess::Forward,
                    )?;
                    assert_eq!(timing.streams(), args[3] == "fixed");
                    black_box(timing);
                }
                black_box(jobs);
            }
            "native-begin" => {
                let mut encoders = Vec::new();
                for job in jobs {
                    let mut encoder = job.encoder()?;
                    let timing = encoder.begin(WIDTH, HEIGHT, PixelDescriptor::RGB8_SRGB)?;
                    assert_eq!(timing.streams(), args[3] == "fixed");
                    encoders.push(encoder);
                }
                black_box(encoders);
            }
            _ => return Err("unknown native preparation stage".into()),
        }
        return Ok(());
    }
    if args[1] == "refuse" {
        if count != 1 {
            return Err("refusal probe retains exactly one encoder".into());
        }
        let required = PolicySet::new().with_workspace_limit(0);
        let error = row_encode::encode(
            config.job().encoder()?,
            Unread,
            Unwritten,
            required,
            Options::new(StorageBudget::new(2 * 1024 * 1024, 16), 1024 * 1024)
                .with_policy_set(required),
            &zencodec::how_far::NoPulse,
        )
        .expect_err("unknown native workspace cannot establish the required bound");
        assert_eq!(error.error().stage, Stage::Policy);
        assert_eq!(error.error().emitted, 0);
        black_box(error);
        return Ok(());
    }
    let codecs = CodecSet::new().with_encoder(config);
    let request = EncodeRequest::new(ImageFormat::Jpeg).preferring_fidelity(
        (0..count)
            .map(|i| Fidelity::codec_quality(75.0 + i as f32))
            .collect::<Vec<_>>(),
    );
    let jobs = request.prepare(&codecs)?;
    assert_eq!(jobs.len(), count);
    if args[1] == "prepare" {
        black_box(jobs);
        return Ok(());
    }
    if !["construct", "begin"].contains(&args[1].as_str()) {
        return Err("expected prepare, construct, begin or refuse".into());
    }
    let mut encoders = Vec::new();
    for job in jobs {
        let mut encoder = job.into_job().into_encoder()?;
        if args[1] == "begin" {
            let timing = encoder.begin_output(
                WIDTH,
                HEIGHT,
                PixelDescriptor::RGB8_SRGB,
                zencodec::output_patch::OutputAccess::Forward,
            )?;
            assert_eq!(timing.streams(), args[3] == "fixed");
        }
        encoders.push(encoder);
    }
    // All candidates remain alive at the same time, as in eager route discovery.
    black_box(encoders);
    Ok(())
}
