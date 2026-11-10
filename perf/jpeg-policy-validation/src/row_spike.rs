//! Candidate-specific controls: actual native codecs through one shared engine.
use zencodec::{
    encode::{DynEncoderConfig, EncodeJob, Encoder, EncoderConfig},
    operation::{OutputSchedule, PolicySet},
    storage::StorageBudget,
};
use zencodec_io::row_encode::{self, Options, Raster};
use zenpixels::{PixelDescriptor, PixelSlice};

fn jpeg_oracle(bytes: &[u8]) -> Vec<u8> {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let mut child = Command::new("djpeg")
        .args(["-ppm", "-rgb"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let input = bytes.to_vec();
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let output = child.wait_with_output().unwrap();
    writer.join().unwrap().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.starts_with(b"P6\n"));
    output.stdout
}

fn compare<C>(config: C, descriptor: PixelDescriptor, data: &[u8], stride: usize)
where
    C: EncoderConfig + DynEncoderConfig + Clone,
    <C::Job as EncodeJob>::Enc: Encoder,
{
    let pixels = || PixelSlice::new(data, 17, 33, stride, descriptor).unwrap();
    let mut outputs = Vec::new();
    for route in 0..3 {
        let mut output = Vec::new();
        let options = Options::new(StorageBudget::new(1 << 20, 1024), 1 << 20);
        let policy = PolicySet::new().require_output_schedule(OutputSchedule::FinishFirst);
        let source = Raster::new(pixels());
        let completed = match route {
            0 => row_encode::encode(
                config.clone().job().encoder().unwrap(),
                source,
                &mut output,
                policy,
                options,
                None,
            ),
            1 => row_encode::encode_job(
                config.clone().job(),
                source,
                &mut output,
                policy,
                options,
                None,
            ),
            2 => row_encode::encode_dyn_job(
                config.clone().dyn_job(),
                source,
                &mut output,
                policy,
                options,
                None,
            ),
            _ => unreachable!(),
        }
        .unwrap();
        assert_eq!(completed.emitted as usize, output.len());
        assert_eq!(completed.schedule, OutputSchedule::FinishFirst);
        outputs.push(output);
    }
    assert_eq!(
        outputs[0], outputs[1],
        "raw/typed differed for {descriptor:?}"
    );
    assert_eq!(
        outputs[0], outputs[2],
        "raw/erased differed for {descriptor:?}"
    );
    // JPEG's independent decoder verifies byte-identical output from all paths.
    if <C as EncoderConfig>::format() == zencodec::ImageFormat::Jpeg {
        assert_eq!(jpeg_oracle(&outputs[0]), jpeg_oracle(&outputs[1]));
    } else {
        let mut independent = png::Decoder::new(std::io::Cursor::new(&outputs[0]));
        // Indexed PNG is lossless too: compare expanded color samples, not indices.
        // EXPAND keeps U16 precision; STRIP_16 is deliberately absent.
        independent.set_transformations(png::Transformations::EXPAND);
        let mut decoder = independent.read_info().unwrap();
        assert_eq!((decoder.info().width, decoder.info().height), (17, 33));
        let mut bytes = vec![0; decoder.output_buffer_size().unwrap()];
        let info = decoder.next_frame(&mut bytes).unwrap();
        // PNG stores U16 in network order; native input words are native-endian.
        let row_bytes = 17 * descriptor.bytes_per_pixel();
        for y in 0..33 {
            let input = &data[y * stride..][..row_bytes];
            let output = &bytes[y * row_bytes..][..row_bytes];
            if info.bit_depth == png::BitDepth::Sixteen {
                for (a, b) in input
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .zip(output.as_chunks::<2>().0)
                {
                    assert_eq!(u16::from_ne_bytes(*a), u16::from_be_bytes(*b));
                }
            } else {
                assert_eq!(input, output);
            }
        }
    }
}

#[test]
fn native_shared_engine_raw_typed_erased_preserve_padded_u8_u16_rgb_gray() {
    for descriptor in [
        PixelDescriptor::RGB8_SRGB,
        PixelDescriptor::GRAY8_SRGB,
        PixelDescriptor::RGB16_SRGB,
        PixelDescriptor::GRAY16_SRGB,
    ] {
        let stride = (17 + 3) * descriptor.bytes_per_pixel();
        let mut data = vec![0xa5; stride * 33];
        for y in 0..33 {
            for (x, b) in data[y * stride..][..17 * descriptor.bytes_per_pixel()]
                .iter_mut()
                .enumerate()
            {
                *b = ((x * 31 + y * 17) % 256) as u8;
            }
        }
        compare(zenpng::PngEncoderConfig::new(), descriptor, &data, stride);
        for progressive in [false, true] {
            compare(
                zenjpeg::JpegEncoderConfig::new()
                    .with_progressive(progressive)
                    .with_prefer_streaming(true),
                descriptor,
                &data,
                stride,
            );
        }
    }
}
