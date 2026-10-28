//! Same configured native JPEG path with and without canonical grid evidence.
//! JPEG remains lossy: the oracle compares the two encoded paths, not their
//! decoded samples with the original uncompressed raster.
use crate::fixtures;
use zencodec::{
    NoPulse,
    encode::{DynEncoderConfig, EncodeJob, EncoderConfig},
    operation::{OutputSchedule, PolicySet},
    storage::StorageBudget,
};
use zencodec_io::row_encode::{self, Options, Raster};
use zenpixels::{ChannelType, SignalRange, sample::ExactDepth};

#[test]
fn native_jpeg_claim_transport_preserves_configuration_bytes_and_decoded_samples() {
    let raw = fixtures::raw();
    let claim = ExactDepth::new(ChannelType::U8, SignalRange::Full, 8, 8).unwrap();
    for progressive in [false, true] {
        let config = zenjpeg::JpegEncoderConfig::new()
            .with_progressive(progressive)
            .with_prefer_streaming(true);
        for route in 0..3 {
            let mut outputs = Vec::new();
            for claimed in [false, true] {
                let mut pixels = fixtures::pixels(&raw);
                if claimed {
                    pixels = pixels.with_exact_depth(claim).unwrap();
                }
                let mut output = Vec::new();
                let source = Raster::new(pixels);
                let policy = PolicySet::new().require_output_schedule(OutputSchedule::FinishFirst);
                let options = Options::new(StorageBudget::new(1 << 20, 1024), 1 << 20);
                let result = match route {
                    0 => row_encode::encode(
                        config.clone().job().encoder().unwrap(),
                        source,
                        &mut output,
                        policy,
                        options,
                        &NoPulse,
                    ),
                    1 => row_encode::encode_job(
                        config.clone().job(),
                        source,
                        &mut output,
                        policy,
                        options,
                        &NoPulse,
                    ),
                    2 => row_encode::encode_dyn_job(
                        config.clone().dyn_job(),
                        source,
                        &mut output,
                        policy,
                        options,
                        &NoPulse,
                    ),
                    _ => unreachable!(),
                }
                .unwrap();
                assert_eq!(result.schedule, OutputSchedule::FinishFirst);
                assert_eq!(result.emitted as usize, output.len());
                outputs.push(output);
            }
            assert_eq!(
                outputs[0], outputs[1],
                "claim changed native bytes in route {route}"
            );
            assert_eq!(fixtures::oracle(&outputs[0]), fixtures::oracle(&outputs[1]));
        }
    }
}
