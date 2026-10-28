//! Private bridge to existing zenavif strip kernels, exposed by `_dev` only.
//! This qualifies mapping/cropping/row partitioning, not RGB color accuracy.
//! Full-range RGB words stay separate from the original component-code facts.
use crate::fixtures::*;
use zencodec::how_far::NoPulse;
use std::sync::Arc;
use zenavif::{media::Av1VideoDecoder, yuv_convert::*};
use zencodec::frame::{
    ChromaSiting, Crop, Frame, FrameInterpretation, FrameTiming, PlaneRole, PlaneSamples,
    PlaneView, Presentation, SampleEncoding, Subsampling,
};
use zencodec_media::frame::{OwnedPlane, OwnedPlanes, PlaneData};
use zencodec_media::session::VideoDecoder;
use zenpixels::{ChannelType, Cicp};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SitingChoice {
    RequireDeclared,
    AssumeCenterIfUnknown,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Refusal {
    Interpretation,
    Geometry,
    Destination,
    Siting,
    Encoding,
    Mapping,
    Sample,
    Allocation,
}

#[derive(Debug)]
pub(super) struct ConvertedFacts {
    // Source evidence is retained independently of the decoded pixel owner.
    source: Arc<FrameInterpretation>,
    timing: FrameTiming,
    presentation: Presentation,
    pub(super) assumed_center: bool,
    output_encoding: SampleEncoding,
}

fn checked_plane<'a>(
    plane: PlaneView<'a>,
    dimensions: (usize, usize),
    encoding: SampleEncoding,
) -> Result<PlaneView<'a>, Refusal> {
    if (plane.width(), plane.height()) != dimensions || plane.encoding() != encoding {
        return Err(Refusal::Mapping);
    }
    let max = ((1u32 << encoding.code_bits()) - 1) as u16;
    // Padding is not a sample. Inspect active words before any destination write.
    // A general normalizing bridge could accept shifted/masked words later;
    // this borrowed native path must never pass them to an LSB-only kernel.
    for y in 0..plane.height() {
        let valid = match plane.row(y).ok_or(Refusal::Mapping)? {
            PlaneSamples::U8(row) => row.iter().all(|v| u16::from(*v) <= max),
            PlaneSamples::U16(row) => row.iter().all(|v| *v <= max),
        };
        if !valid {
            return Err(Refusal::Sample);
        }
    }
    Ok(plane)
}

/// LSB replication from zenavif feadc4c's private `convert::scale_to_u16`.
/// Its input is explicitly native-depth; output is normalized U16 RGB.
pub(super) fn widen(value: impl Into<u16>, bits: u8) -> u16 {
    let value = value.into();
    let shift = 16 - bits;
    (value << shift) | (value >> (bits - shift))
}

pub(super) fn conversion_facts(
    frame: &Frame,
    choice: SitingChoice,
) -> Result<ConvertedFacts, Refusal> {
    let info = frame.interpretation();
    let color = info.color.resolved.ok_or(Refusal::Interpretation)?;
    if color.color_primaries != 1
        || color.transfer_characteristics != 13
        || color.matrix_coefficients != 1
        || info.roles != [PlaneRole::Luma, PlaneRole::Cb, PlaneRole::Cr]
        || !matches!(
            info.subsampling,
            Subsampling::Yuv420 | Subsampling::Yuv422 | Subsampling::Yuv444
        )
    {
        return Err(Refusal::Interpretation);
    }
    let encoding = info.encoding;
    let bits = encoding.code_bits();
    if !matches!(bits, 8 | 10 | 12)
        || encoding.bit_shift() != 0
        || encoding.storage()
            != if bits == 8 {
                ChannelType::U8
            } else {
                ChannelType::U16
            }
    {
        return Err(Refusal::Encoding);
    }
    let assumed_center = if info.subsampling == Subsampling::Yuv444 {
        false
    } else {
        match (info.siting, choice) {
            (ChromaSiting::Center, _) => false,
            (ChromaSiting::Unknown, SitingChoice::AssumeCenterIfUnknown) => true,
            _ => return Err(Refusal::Siting),
        }
    };
    let geometry = info.geometry;
    let crop = geometry.crop;
    if crop.width == 0
        || crop.height == 0
        || crop
            .x
            .checked_add(crop.width)
            .is_none_or(|x| x > geometry.coded_width)
        || crop
            .y
            .checked_add(crop.height)
            .is_none_or(|y| y > geometry.coded_height)
    {
        return Err(Refusal::Geometry);
    }
    Ok(ConvertedFacts {
        source: Arc::clone(info),
        timing: frame.timing(),
        presentation: frame.presentation(),
        assumed_center,
        output_encoding: SampleEncoding::new(ChannelType::U16, 16, 0).unwrap(),
    })
}

/// Convert the visible crop into strided RGB16 destination rows. One coded
/// RGB row is scratch; all source planes remain borrowed. Using coded
/// coordinates until after upsampling preserves odd-origin chroma phase.
///
/// Deliberately private: only BT.709 matrix/sRGB transfer and primaries,
/// centered subsampling and 8/10/12-bit LSB words are qualified here. The
/// existing kernels clip into the output gamut; this is not a lossless offer.
fn convert(
    frame: &Frame,
    choice: SitingChoice,
    destination: &mut [[u16; 3]],
    stride: usize,
    rows_per_strip: usize,
) -> Result<ConvertedFacts, Refusal> {
    let facts = conversion_facts(frame, choice)?;
    let info = frame.interpretation();
    let color = info.color.resolved.unwrap();
    let encoding = info.encoding;
    let bits = encoding.code_bits();
    let geometry = info.geometry;
    let crop = geometry.crop;
    let (width, height) = (
        geometry.coded_width as usize,
        geometry.coded_height as usize,
    );
    let needed = (crop.height as usize - 1)
        .checked_mul(stride)
        .and_then(|n| n.checked_add(crop.width as usize))
        .ok_or(Refusal::Destination)?;
    if stride < crop.width as usize || destination.len() < needed || rows_per_strip == 0 {
        return Err(Refusal::Destination);
    }
    let mapped = frame.map().map_err(|_| Refusal::Mapping)?;
    if mapped.plane_count() != 3 {
        return Err(Refusal::Mapping);
    }
    let chroma = info
        .subsampling
        .chroma_size(geometry.coded_width, geometry.coded_height)
        .ok_or(Refusal::Interpretation)?;
    let mut planes = Vec::new();
    for dimensions in [
        (width, height),
        (chroma.0 as usize, chroma.1 as usize),
        (chroma.0 as usize, chroma.1 as usize),
    ] {
        let plane = mapped.plane(planes.len()).ok_or(Refusal::Mapping)?;
        planes.push(checked_plane(plane, dimensions, encoding)?);
    }
    let range = if color.full_range {
        YuvRange::Full
    } else {
        YuvRange::Limited
    };
    let count = rows_per_strip.min(crop.height as usize);
    let scratch_len = width.checked_mul(count).ok_or(Refusal::Allocation)?;
    // An array keeps the three-plane kernel contract explicit.
    let planes: [PlaneView<'_>; 3] = planes.try_into().map_err(|_| Refusal::Mapping)?;
    macro_rules! convert_words {
        ($kind:ident, $f420:ident, $f422:ident, $f444:ident $(, $depth:expr)?) => {{
            let [y, u, v] = planes.each_ref().map(|plane| plane.as_strided_samples());
            let (PlaneSamples::$kind(y), PlaneSamples::$kind(u), PlaneSamples::$kind(v)) = (y, u, v) else {
                return Err(Refusal::Mapping);
            };
            let [ys, us, vs] = planes.each_ref().map(|plane| plane.stride_bytes() / encoding.bytes_per_sample());
            let mut scratch = Vec::new();
            scratch.try_reserve_exact(scratch_len).map_err(|_| Refusal::Allocation)?;
            scratch.resize(scratch_len, Default::default());
            for start in (0..crop.height as usize).step_by(count) {
                let rows = count.min(crop.height as usize - start);
                let y_start = crop.y as usize + start;
                match info.subsampling {
                    Subsampling::Yuv420 => $f420(y, ys, u, us, v, vs, width, height, y_start, rows, range, YuvMatrix::Bt709, $($depth,)? &mut scratch),
                    Subsampling::Yuv422 => $f422(y, ys, u, us, v, vs, width, y_start, rows, range, YuvMatrix::Bt709, $($depth,)? &mut scratch),
                    Subsampling::Yuv444 => $f444(y, ys, u, us, v, vs, width, y_start, rows, range, YuvMatrix::Bt709, $($depth,)? &mut scratch),
                    _ => unreachable!(),
                }
                for row in 0..rows {
                    let src = &scratch[row * width + crop.x as usize..][..crop.width as usize];
                    let dst = &mut destination[(start + row) * stride..][..crop.width as usize];
                    for (out, pixel) in dst.iter_mut().zip(src) {
                        *out = [widen(pixel.r, bits), widen(pixel.g, bits), widen(pixel.b, bits)];
                    }
                }
            }
        }};
    }
    if bits == 8 {
        convert_words!(
            U8,
            yuv420_to_rgb8_strip,
            yuv422_to_rgb8_strip,
            yuv444_to_rgb8_strip
        );
    } else {
        convert_words!(
            U16,
            yuv420_to_rgb16_strip,
            yuv422_to_rgb16_strip,
            yuv444_to_rgb16_strip,
            bits
        );
    }
    Ok(facts)
}

fn reinterpret(frame: &Frame, edit: impl FnOnce(&mut FrameInterpretation)) -> Frame {
    let mut info = (**frame.interpretation()).clone();
    edit(&mut info);
    Frame::new(
        Arc::clone(frame.picture()),
        Arc::new(info),
        frame.timing(),
        frame.presentation(),
    )
}

#[test]
fn planar_native_borrowed_rows_and_odd_crops_match_full_conversion() {
    let artifacts =
        std::env::var_os("AV1_POLICY_ARTIFACTS").expect("persistent artifact path required");
    let root = std::path::Path::new(&artifacts);
    std::fs::create_dir_all(root).unwrap();
    let mut rows = String::from(
        "bits,subsampling,full_range,frame,crop_x,crop_y,width,height,assumed_center\n",
    );
    for bits in [8, 10, 12] {
        for sub in [
            Subsampling::Yuv420,
            Subsampling::Yuv422,
            Subsampling::Yuv444,
        ] {
            for full in [false, true] {
                let s = settings(bits, sub, false).with_color(Cicp::new(1, 13, 1, full));
                let inputs: Vec<_> = (0..3).map(|i| frame(&s, i)).collect();
                let (config, packets) = encode(s.clone(), &inputs);
                let mut settings = rav1d_safe::Settings::default();
                settings.threads = 1;
                let mut decoder = Av1VideoDecoder::with_settings(settings).unwrap();
                let frames = decode(&mut decoder, &config, &packets, &NoPulse).unwrap();
                assert_eq!(frames.len(), 3);
                oracle(
                    root,
                    &format!("planar-{bits}-{sub:?}-{full}"),
                    &s,
                    &packets,
                    &frames,
                );
                let comparison = root.join(format!("planar-{bits}-{sub:?}-{full}"));
                std::fs::write(comparison.join("rgb-layout.json"), format!(
                    "{{\"width\":{},\"height\":{},\"frames\":{},\"bits\":{bits},\"subsampling\":\"{sub:?}\",\"full_range\":{full}}}\n",
                    s.width, s.height, frames.len())).unwrap();
                decoder.reset().unwrap();
                drop(decoder);
                for (index, (native, input)) in frames.into_iter().zip(inputs).enumerate() {
                    assert_eq!(samples(&native), samples(&input));
                    let (w, h) = (s.width as usize, s.height as usize);
                    let mut full_rgb = vec![[0; 3]; w * h];
                    let facts = convert(
                        &native,
                        SitingChoice::AssumeCenterIfUnknown,
                        &mut full_rgb,
                        w,
                        h,
                    )
                    .unwrap();
                    // Preserve our actual RGB result for the separate zimg
                    // inspection. The independent native-sample gate above
                    // remains exact and is not weakened by that comparison.
                    std::fs::write(
                        comparison.join(format!("zen-rgb16-{index}.raw")),
                        full_rgb
                            .iter()
                            .flatten()
                            .flat_map(|v| v.to_le_bytes())
                            .collect::<Vec<_>>(),
                    )
                    .unwrap();
                    let (full_float, processing) =
                        crate::prepared_conversion::float_pixels_with_processing(&native);
                    std::fs::write(comparison.join(format!("processing-{index}.csv")), format!("max_iec_error,max_roundtrip_error,max_continuous_error,min_linear,max_linear,clipped_samples,max_gamma_error,max_gamma_roundtrip_error,gamma_extended_samples\n{},{},{},{},{},{},{},{},{}\n", processing.max_linear_error, processing.max_roundtrip_error, processing.max_model_error, processing.min_linear, processing.max_linear, processing.clipped_samples, processing.max_gamma_error, processing.max_gamma_roundtrip_error, processing.gamma_extended_samples)).unwrap();
                    for (stage, samples) in [
                        ("linear", &processing.gamma_input),
                        ("encoded", &processing.gamma_encoded),
                        ("restored", &processing.gamma_restored),
                    ] {
                        std::fs::write(
                            comparison.join(format!("gamma-{stage}-{index}.raw")),
                            samples
                                .iter()
                                .flatten()
                                .flat_map(|v| v.to_le_bytes())
                                .collect::<Vec<_>>(),
                        )
                        .unwrap();
                    }
                    assert!(full_float.iter().flatten().any(|v| *v < 0.0));
                    assert!(full_float.iter().flatten().any(|v| *v > 1.0));
                    std::fs::write(
                        comparison.join(format!("zen-rgbf32-{index}.raw")),
                        full_float
                            .iter()
                            .flatten()
                            .flat_map(|v| v.to_le_bytes())
                            .collect::<Vec<_>>(),
                    )
                    .unwrap();
                    assert!(Arc::ptr_eq(&facts.source, native.interpretation()));
                    assert_eq!(facts.timing, native.timing());
                    assert_eq!(facts.presentation, native.presentation());
                    assert_eq!(facts.output_encoding.code_bits(), 16);
                    assert_eq!(facts.assumed_center, sub != Subsampling::Yuv444);
                    // Conversion facts never relabel the source's component encoding.
                    assert_eq!(facts.source.encoding.code_bits(), bits);
                    let mut padded_source_rgb = vec![[0; 3]; w * h];
                    convert(
                        &input,
                        SitingChoice::AssumeCenterIfUnknown,
                        &mut padded_source_rgb,
                        w,
                        1,
                    )
                    .unwrap();
                    assert_eq!(padded_source_rgb, full_rgb);
                    for crop in [
                        native.interpretation().geometry.crop,
                        Crop {
                            x: 1,
                            y: 1,
                            width: s.width - 2,
                            height: s.height - 2,
                        },
                        Crop {
                            x: s.width - 1,
                            y: s.height - 1,
                            width: 1,
                            height: 1,
                        },
                    ] {
                        let cropped = reinterpret(&native, |i| i.geometry.crop = crop);
                        let stride = crop.width as usize + 5;
                        for strip_rows in [1, 2, 4, crop.height as usize] {
                            let sentinel = [0xdead, 0xbeef, 0xa55a];
                            let mut destination = vec![sentinel; stride * crop.height as usize + 7];
                            let facts = convert(
                                &cropped,
                                SitingChoice::AssumeCenterIfUnknown,
                                &mut destination,
                                stride,
                                strip_rows,
                            )
                            .unwrap();
                            for y in 0..crop.height as usize {
                                let start = (crop.y as usize + y) * w + crop.x as usize;
                                assert_eq!(
                                    &destination[y * stride..][..crop.width as usize],
                                    &full_rgb[start..][..crop.width as usize]
                                );
                                assert!(
                                    destination[y * stride + crop.width as usize..(y + 1) * stride]
                                        .iter()
                                        .all(|p| *p == sentinel)
                                );
                            }
                            assert!(
                                destination[stride * crop.height as usize..]
                                    .iter()
                                    .all(|p| *p == sentinel)
                            );
                            assert_eq!(facts.source.geometry.crop, crop);
                        }
                        qualify_packed_publication(&cropped, &full_rgb);
                        crate::prepared_conversion::qualify(&cropped, &full_rgb);
                        crate::prepared_conversion::qualify_float(&cropped, &full_float);
                        rows.push_str(&format!(
                            "{bits},{sub:?},{full},{index},{},{},{},{},{}\n",
                            crop.x, crop.y, crop.width, crop.height, facts.assumed_center
                        ));
                    }
                    let mut untouched = vec![[0xffff; 3]; w * h];
                    if sub != Subsampling::Yuv444 {
                        assert_eq!(
                            convert(&native, SitingChoice::RequireDeclared, &mut untouched, w, 1)
                                .unwrap_err(),
                            Refusal::Siting
                        );
                        assert!(untouched.iter().all(|v| *v == [0xffff; 3]));
                    }
                    let retained_picture = Arc::downgrade(native.picture());
                    drop(native);
                    assert!(
                        retained_picture.upgrade().is_none(),
                        "converted facts must not retain raw picture storage"
                    );
                    assert_eq!(facts.source.encoding.code_bits(), bits);
                }
            }
        }
    }
    std::fs::write(root.join("planar-conversion.csv"), rows).unwrap();
}

#[test]
fn planar_unqualified_interpretations_refuse_before_writing() {
    let base = frame(&settings(10, Subsampling::Yuv420, false), 0);
    let mut destination = vec![[0xaaaa; 3]; 17 * 13];
    for siting in [ChromaSiting::Left, ChromaSiting::TopLeft] {
        let frame = reinterpret(&base, |i| i.siting = siting);
        assert_eq!(
            convert(
                &frame,
                SitingChoice::AssumeCenterIfUnknown,
                &mut destination,
                17,
                1
            )
            .unwrap_err(),
            Refusal::Siting
        );
    }
    let centered = reinterpret(&base, |i| i.siting = ChromaSiting::Center);
    for frame in [
        reinterpret(&centered, |i| {
            i.encoding = SampleEncoding::new(ChannelType::U16, 10, 6).unwrap()
        }),
        reinterpret(&centered, |i| i.encoding = SampleEncoding::U8),
    ] {
        let expected = if frame.interpretation().encoding.bit_shift() == 6 {
            Refusal::Encoding
        } else {
            Refusal::Mapping
        };
        assert_eq!(
            convert(
                &frame,
                SitingChoice::RequireDeclared,
                &mut destination,
                17,
                1
            )
            .unwrap_err(),
            expected
        );
    }
    for frame in [
        reinterpret(&centered, |i| i.color.resolved = None),
        reinterpret(&centered, |i| i.roles.swap(1, 2)),
        reinterpret(&centered, |i| {
            i.color.resolved = Some(Cicp::new(9, 16, 9, true))
        }),
    ] {
        assert_eq!(
            convert(
                &frame,
                SitingChoice::RequireDeclared,
                &mut destination,
                17,
                1
            )
            .unwrap_err(),
            Refusal::Interpretation
        );
    }
    for crop in [
        Crop {
            x: u32::MAX,
            y: 0,
            width: 2,
            height: 1,
        },
        Crop {
            x: 16,
            y: 0,
            width: 2,
            height: 1,
        },
        Crop {
            x: 0,
            y: 0,
            width: 0,
            height: 13,
        },
    ] {
        let frame = reinterpret(&centered, |i| i.geometry.crop = crop);
        assert_eq!(
            convert(
                &frame,
                SitingChoice::RequireDeclared,
                &mut destination,
                17,
                1
            )
            .unwrap_err(),
            Refusal::Geometry
        );
    }
    assert_eq!(
        convert(
            &centered,
            SitingChoice::RequireDeclared,
            &mut destination[..1],
            17,
            1
        )
        .unwrap_err(),
        Refusal::Destination
    );
    assert_eq!(
        convert(
            &centered,
            SitingChoice::RequireDeclared,
            &mut destination,
            16,
            1
        )
        .unwrap_err(),
        Refusal::Destination
    );
    assert_eq!(
        convert(
            &centered,
            SitingChoice::RequireDeclared,
            &mut destination,
            17,
            0
        )
        .unwrap_err(),
        Refusal::Destination
    );
    assert!(destination.iter().all(|v| *v == [0xaaaa; 3]));
    // Explicit centered evidence works without a caller assumption.
    assert!(
        !convert(
            &centered,
            SitingChoice::RequireDeclared,
            &mut destination,
            17,
            1
        )
        .unwrap()
        .assumed_center
    );
}

#[test]
fn planar_rejects_noncanonical_active_words_but_not_padding() {
    let source = frame(&settings(10, Subsampling::Yuv420, false), 0);
    let source = reinterpret(&source, |i| i.siting = ChromaSiting::Center);
    let mapped = source.map().unwrap();
    let mut planes = Vec::new();
    for p in 0..3 {
        let plane = mapped.plane(p).unwrap();
        let stride = plane.width() + 3;
        let mut words = vec![0xffff; stride * plane.height()];
        for y in 0..plane.height() {
            let PlaneSamples::U16(row) = plane.row(y).unwrap() else {
                unreachable!()
            };
            words[y * stride..][..row.len()].copy_from_slice(row);
        }
        if p == 2 {
            words[(plane.height() - 1) * stride + plane.width() - 1] = 1024;
        }
        planes.push(OwnedPlane {
            data: PlaneData::U16(words),
            width: plane.width(),
            height: plane.height(),
            stride_bytes: stride * 2,
        });
    }
    let encoding = source.interpretation().encoding;
    let bad = Frame::new(
        Arc::new(OwnedPlanes::new(planes, encoding).unwrap()),
        Arc::clone(source.interpretation()),
        source.timing(),
        source.presentation(),
    );
    let mut destination = vec![[0xabcd; 3]; 17 * 13];
    assert_eq!(
        convert(&bad, SitingChoice::RequireDeclared, &mut destination, 17, 1).unwrap_err(),
        Refusal::Sample
    );
    assert!(destination.iter().all(|p| *p == [0xabcd; 3]));
    for bits in [8, 10, 12] {
        let max = (1u16 << bits) - 1;
        assert_eq!(widen(0u16, bits), 0);
        assert_eq!(widen(max, bits), u16::MAX);
        for value in 0..=max {
            assert_eq!(widen(value, bits) >> (16 - bits), value);
        }
    }
}

#[test]
fn planar_native_signalled_siting_cannot_be_overridden_by_unknown_fallback() {
    for siting in [ChromaSiting::Left, ChromaSiting::TopLeft] {
        let s = settings(8, Subsampling::Yuv420, false).with_siting(siting);
        let input = frame(&s, 0);
        let (config, packets) = encode(s, std::slice::from_ref(&input));
        let mut settings = rav1d_safe::Settings::default();
        settings.threads = 1;
        let mut decoder = Av1VideoDecoder::with_settings(settings).unwrap();
        let frames = decode(&mut decoder, &config, &packets, &NoPulse).unwrap();
        assert_eq!(frames.len(), 1);
        let native = &frames[0];
        assert_eq!(native.interpretation().siting, siting);
        crate::prepared_conversion::reject_declared_siting(native);
        assert_eq!(samples(native), samples(&input));
        let mut destination = vec![[0xaaaa; 3]; 17 * 13];
        assert_eq!(
            convert(
                native,
                SitingChoice::AssumeCenterIfUnknown,
                &mut destination,
                17,
                1
            )
            .unwrap_err(),
            Refusal::Siting
        );
        assert!(destination.iter().all(|v| *v == [0xaaaa; 3]));
    }
}

/// Carry the real strip-kernel result through shared packed-image publication.
/// RGB accuracy is still separate from this exact ownership/interpretation check.
fn qualify_packed_publication(native: &Frame, expected: &[[u16; 3]]) {
    use zencodec::{
        At,
        image::{ImageOutput, OutputError, PackedImageOwner},
        operation::{Decision, PolicySet, Question, resolve_image},
        storage::{SharedStorage, StorageBudget},
    };
    use zenpixels::{ColorContext, Orientation, PixelDescriptor, PixelSlice};
    struct Owner {
        data: SharedStorage<[u16; 3]>,
        width: u32,
        height: u32,
        stride: usize,
        color: Arc<ColorContext>,
    }
    impl PackedImageOwner for Owner {
        fn pixels(&self) -> Result<PixelSlice<'_>, At<OutputError>> {
            PixelSlice::new(
                bytemuck::cast_slice(self.data.as_slice()),
                self.width,
                self.height,
                self.stride * 6,
                PixelDescriptor::RGB16_SRGB,
            )
            .map(|p| p.with_color_context(self.color.clone()))
            .map_err(|e| {
                At::wrap(OutputError::Mapping(zencodec::frame::MapError::Backend(
                    Box::new(e),
                )))
            })
        }
    }
    let crop = native.interpretation().geometry.crop;
    let stride = crop.width as usize + 5;
    let pool = StorageBudget::new(stride * crop.height as usize * 6, 1);
    let mut data = pool
        .filled(stride * crop.height as usize, [0xdead; 3])
        .unwrap();
    let facts = convert(
        native,
        SitingChoice::AssumeCenterIfUnknown,
        data.as_mut_slice(),
        stride,
        1,
    )
    .unwrap();
    let address = data.as_slice().as_ptr() as usize;
    let owner = Owner {
        data: data.freeze(),
        width: crop.width,
        height: crop.height,
        stride,
        color: Arc::new(ColorContext::from_cicp(Cicp::SRGB)),
    };
    let image =
        ImageOutput::from_frame_pixels(native.clone(), owner, Orientation::Identity).unwrap();
    let calls = std::cell::Cell::new(0);
    let image = resolve_image(
        image,
        &mut |q: Question<'_>| -> Result<Decision, At<std::convert::Infallible>> {
            let Question::Image {
                image,
                facts: current,
                ..
            } = q
            else {
                panic!()
            };
            assert!(current.frame.is_none());
            assert_eq!(
                current.source_frame.unwrap().encoding,
                facts.source.encoding
            );
            assert_eq!(current.timing, Some(facts.timing));
            assert_eq!(current.presentation, Some(facts.presentation));
            assert_eq!(current.packed, Some(PixelDescriptor::RGB16_SRGB));
            assert!(Arc::ptr_eq(image.source_frame().unwrap(), &facts.source));
            image
                .with_pixels(|p| {
                    assert_eq!(p.row(0).as_ptr() as usize, address);
                    assert_eq!(p.color_context().unwrap().cicp, Some(Cicp::SRGB));
                    for y in 0..crop.height as usize {
                        assert_eq!(
                            p.row(y as u32),
                            bytemuck::cast_slice(
                                &expected[(y + crop.y as usize)
                                    * native.interpretation().geometry.coded_width as usize
                                    + crop.x as usize..]
                                    [..crop.width as usize]
                            )
                        );
                    }
                })
                .unwrap();
            calls.set(calls.get() + 1);
            Ok(Decision::UsePreset)
        },
        PolicySet::new(),
        &NoPulse,
    )
    .unwrap()
    .into_output();
    assert_eq!(calls.get(), 1);
    let retained = image.clone();
    drop(image);
    assert_eq!(pool.reserved_objects(), 1);
    drop(retained);
    assert_eq!(pool.reserved_bytes(), 0);
    assert_eq!(pool.reserved_objects(), 0);
}
