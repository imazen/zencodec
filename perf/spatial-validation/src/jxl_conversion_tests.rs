//! Native CMYK carriage versus output conversion: qualify the public entry point.
use crate::{jxl_packaged_storage, jxl_tests::fixture};
use std::sync::Arc;
use zencodec::{
    collection::{Composition, ImageRequest},
    image::ImageOutput,
    source::{ReadAtShim, SliceReadAt},
    storage::StorageBudget,
};
use zenpixels::{ColorOrigin, PixelBuffer, PixelBufferParts, PixelFormat, PixelSlice};
use zenpixels_convert::{
    Cicp, ConvertError,
    cms_moxcms::MoxCms,
    output::{EncodeReady, OutputProfile},
};

fn native_cmyk(view: Composition) -> (ImageOutput, StorageBudget) {
    let bytes = fixture("conformance_test_images/cmyk_layers.jxl");
    let mut source = ReadAtShim(SliceReadAt::new(&bytes));
    let (codecs, storage) = jxl_packaged_storage::factories(64, 64);
    let mut images =
        jxl_packaged_storage::open_admitted(&codecs, &mut source, view, 64, &storage).unwrap();
    let mut cursor = images.images().unwrap();
    let id = images.next(&mut cursor).unwrap().unwrap();
    let image = images.decode(&id, &ImageRequest::new()).unwrap();
    (image.into_output(), storage)
}

/// The public finalizer currently requires an owned PixelBuffer. Deliberately
/// retain prefix, row padding and a suffix so a contiguous-only copy is caught.
fn stage_for_finalizer(pixels: PixelSlice<'_>) -> PixelBuffer {
    let bpp = pixels.descriptor().bytes_per_pixel();
    let stride = (pixels.width() as usize + 2) * bpp;
    let mut parts = PixelBufferParts::new(
        zenpixels::PixelStorage::from(vec![0xa5u8; bpp + stride * pixels.rows() as usize + bpp]),
        pixels.width(),
        pixels.rows(),
        stride,
        pixels.descriptor(),
    );
    parts.offset = bpp;
    parts.color_context = pixels.color_context().cloned();
    let mut buffer = PixelBuffer::try_from_parts(parts).unwrap();
    for y in 0..pixels.rows() {
        buffer
            .as_slice_mut()
            .row_mut(y)
            .copy_from_slice(pixels.row(y));
    }
    buffer
}

fn icc(buffer: &PixelBuffer) -> Arc<[u8]> {
    buffer
        .color_context()
        .unwrap()
        .icc
        .as_ref()
        .unwrap()
        .clone()
}

#[test]
fn native_cmyk_finalization_identity_preserves_samples_and_current_profile() {
    let (image, storage) = native_cmyk(Composition::Uncomposited);
    let buffer = image.with_pixels(stage_for_finalizer).unwrap();
    assert_eq!(
        buffer.descriptor().pixel_format(),
        PixelFormat::CmykF32Inverted
    );
    let profile = icc(&buffer);
    assert_eq!(&profile[16..20], b"CMYK");
    let origin = ColorOrigin::from_icc(profile.clone());
    let original = buffer.as_slice().as_strided_bytes().to_vec();
    for target in [
        OutputProfile::SameAsOrigin,
        OutputProfile::Icc(profile.clone()),
    ] {
        let ready =
            EncodeReady::finalize(&buffer, &origin, target, PixelFormat::CmykF32Inverted, None)
                .unwrap();
        let converted = ready.pixels();
        assert_eq!(converted.descriptor(), buffer.descriptor());
        assert_eq!(
            converted.color_context().unwrap().icc,
            Some(profile.clone())
        );
        assert_eq!(ready.metadata().icc, Some(profile.clone()));
        assert_eq!(ready.metadata().cicp, None);
        for y in 0..converted.rows() {
            assert_eq!(converted.row(y), buffer.as_slice().row(y));
        }
        assert_eq!(buffer.as_slice().as_strided_bytes(), original);
        jxl_packaged_storage::assert_charges(&storage, &[&image]);
    }
}

#[test]
fn native_cmyk_finalization_requires_a_cms_for_rgb() {
    let (image, storage) = native_cmyk(Composition::Uncomposited);
    let buffer = image.with_pixels(stage_for_finalizer).unwrap();
    let origin = ColorOrigin::from_icc(icc(&buffer));
    let original = buffer.as_slice().as_strided_bytes().to_vec();
    let context = buffer.color_context().cloned();
    {
        let error = EncodeReady::finalize(
            &buffer,
            &origin,
            OutputProfile::Named(Cicp::SRGB),
            PixelFormat::RgbF32,
            None,
        )
        .err()
        .expect("CMYK to RGB requires a CMS");
        assert!(
            matches!(error.error(), ConvertError::NeedsCms { .. }),
            "{error:?}"
        );
        assert_eq!(buffer.as_slice().as_strided_bytes(), original);
        assert_eq!(buffer.color_context(), context.as_ref());
        jxl_packaged_storage::assert_charges(&storage, &[&image]);
    }
}

#[test]
fn native_cmyk_finalization_copies_are_outside_the_decode_output_pool() {
    let (image, storage) = native_cmyk(Composition::Uncomposited);
    let original_address = image
        .with_pixels(|p| p.as_strided_bytes().as_ptr() as usize)
        .unwrap();
    let buffer = image.with_pixels(stage_for_finalizer).unwrap();
    let ready = EncodeReady::finalize(
        &buffer,
        &ColorOrigin::from_icc(icc(&buffer)),
        OutputProfile::SameAsOrigin,
        PixelFormat::CmykF32Inverted,
        None,
    )
    .unwrap();
    let staged_address = buffer.as_slice().as_strided_bytes().as_ptr() as usize;
    let finalized_address = ready.pixels().as_strided_bytes().as_ptr() as usize;
    assert_ne!(original_address, staged_address);
    assert_ne!(original_address, finalized_address);
    assert_ne!(staged_address, finalized_address);
    jxl_packaged_storage::assert_charges(&storage, &[&image]);
    drop(image);
    assert_eq!(
        (storage.reserved_bytes(), storage.reserved_objects()),
        (0, 0)
    );
    for y in 0..buffer.height() {
        assert_eq!(buffer.as_slice().row(y), ready.pixels().row(y));
    }
}

#[test]
fn native_cmyk_finalization_origin_cannot_replace_missing_current_profile() {
    let (image, storage) = native_cmyk(Composition::Uncomposited);
    let buffer = image.with_pixels(stage_for_finalizer).unwrap();
    let origin = ColorOrigin::from_icc(icc(&buffer));
    let mut parts = buffer.into_parts();
    parts.color_context = None;
    let buffer = PixelBuffer::try_from_parts(parts).unwrap();
    let original = buffer.as_slice().as_strided_bytes().to_vec();
    let error = EncodeReady::finalize(
        &buffer,
        &origin,
        OutputProfile::SameAsOrigin,
        PixelFormat::CmykF32Inverted,
        Some(&MoxCms),
    )
    .err()
    .expect("provenance is not the current sample interpretation");
    assert!(
        matches!(error.error(), ConvertError::NeedsCms { .. }),
        "{error:?}"
    );
    assert_eq!(buffer.as_slice().as_strided_bytes(), original);
    jxl_packaged_storage::assert_charges(&storage, &[&image]);
}

/// The full native selection, not only previously sampled CMYK channels.
/// Keep the frozen decoder's existing strict 0.0013 CMYK bound; RGB matrix
/// oracle tests retain their separate 0.0001 bound.
#[cfg(feature = "cms_oracle")]
#[test]
fn native_cmyk_mox_adapter_matches_backend_and_reports_known_accuracy_failure() {
    cmyk_reference_control(
        &MoxCms,
        "native-cmyk-finalization",
        AccuracyExpectation::KnownMoxDisagreement,
    );
}

#[cfg(all(feature = "cms_oracle", lcms_native))]
#[test]
fn native_cmyk_lcms_finalization_matches_independent_reference() {
    cmyk_reference_control(
        &zenpixels_convert::Lcms,
        "native-cmyk-lcms",
        AccuracyExpectation::Required,
    );
}

#[cfg(feature = "cms_oracle")]
#[derive(Clone, Copy)]
enum AccuracyExpectation {
    #[cfg(lcms_native)]
    Required,
    KnownMoxDisagreement,
}

#[cfg(feature = "cms_oracle")]
fn cmyk_reference_control(
    backend: &dyn zenpixels_convert::cms::PluggableCms,
    artifact_name: &str,
    expectation: AccuracyExpectation,
) {
    use std::{fs, io::Write, path::PathBuf, process::Command};
    let oracle =
        std::env::var_os("SPATIAL_COLOR_ORACLE").expect("supply independent Little CMS executable");
    let root = PathBuf::from(
        std::env::var_os("SPATIAL_COLOR_ARTIFACTS").expect("supply fresh color artifact directory"),
    );
    fs::create_dir_all(&root).unwrap();
    let artifacts = root.join(artifact_name);
    fs::create_dir(&artifacts).unwrap();
    let (image, storage) = native_cmyk(Composition::Uncomposited);
    let buffer = image.with_pixels(stage_for_finalizer).unwrap();
    assert_eq!(
        buffer.descriptor().pixel_format(),
        PixelFormat::CmykF32Inverted
    );
    let profile = icc(&buffer);
    assert_eq!(&profile[16..20], b"CMYK");
    assert!(
        profile.len() < 8 * 1024 * 1024,
        "bounded single-profile witness"
    );
    assert!(
        u64::from(buffer.width()) * u64::from(buffer.height()) * 40 <= 64 * 1024 * 1024,
        "single native witness input/reference files must fit 64 MiB"
    );
    let original = buffer.as_slice().as_strided_bytes().to_vec();
    let context = buffer.color_context().cloned();
    let origin = ColorOrigin::from_icc(profile.clone());
    let ready = EncodeReady::finalize(
        &buffer,
        &origin,
        OutputProfile::Named(Cicp::SRGB),
        PixelFormat::RgbF32,
        Some(backend),
    )
    .unwrap();
    let output = ready.pixels();
    // An independent direct CMS call must match the finalizer bit-for-bit:
    // separate routing/striding errors from numerical backend disagreement.
    use zenpixels_convert::{AlphaPolicy, ColorProfileSource, ConvertOptions};
    let options = ConvertOptions::permissive().with_alpha_policy(AlphaPolicy::DiscardUnchecked);
    let mut direct = backend
        .build_source_transform(
            ColorProfileSource::Icc(&profile),
            ColorProfileSource::Cicp(Cicp::SRGB),
            PixelFormat::CmykF32Inverted,
            PixelFormat::RgbF32,
            &options,
        )
        .expect("selected CMS must accept the same declared route")
        .unwrap();
    for y in 0..buffer.height() {
        let bytes = buffer.width() as usize * 12;
        let mut row = vec![0xa5; bytes + 16];
        direct
            .try_transform_row(buffer.as_slice().row(y), &mut row, buffer.width())
            .unwrap();
        assert_eq!(&row[..bytes], output.row(y), "finalizer/direct CMS row {y}");
        assert!(row[bytes..].iter().all(|&b| b == 0xa5));
    }
    assert_eq!(output.descriptor().pixel_format(), PixelFormat::RgbF32);
    assert_eq!(
        (output.width(), output.rows()),
        (buffer.width(), buffer.height())
    );
    assert_eq!(output.color_context().unwrap().cicp, Some(Cicp::SRGB));
    assert_eq!(ready.metadata().cicp, Some(Cicp::SRGB));
    let profile_path = artifacts.join("source.icc");
    fs::write(&profile_path, &profile).unwrap();
    let input_path = artifacts.join("inverted-cmyk.f32");
    let mut input = fs::File::create(&input_path).unwrap();
    for y in 0..buffer.height() {
        input.write_all(buffer.as_slice().row(y)).unwrap();
    }
    drop(input);
    let run = |mode: &str, name: &str| {
        let path = artifacts.join(name);
        let result = Command::new(&oracle)
            .arg(mode)
            .arg(&profile_path)
            .arg("-")
            .arg(&input_path)
            .arg(&path)
            .arg(buffer.width().to_string())
            .arg(buffer.height().to_string())
            .arg("4")
            .output()
            .unwrap();
        fs::write(artifacts.join(format!("{name}.stdout")), &result.stdout).unwrap();
        fs::write(artifacts.join(format!("{name}.stderr")), &result.stderr).unwrap();
        assert!(result.status.success(), "LCMS failed: {:?}", result.status);
        fs::read(path).unwrap()
    };
    let reference = run("--cmyk-inverted", "reference-rgb.f32");
    let reversed = run("--cmyk", "wrong-polarity-rgb.f32");
    let samples = buffer.width() as usize * buffer.height() as usize * 3;
    assert_eq!(reference.len(), samples * 4);
    assert_eq!(reversed.len(), reference.len());
    let mut max_abs = 0.0f64;
    let mut violations = 0;
    let mut nonfinite = 0;
    let mut polarity_differences = 0;
    let mut out_of_gamut = 0;
    let mut worst = serde_json::Value::Null;
    for y in 0..output.rows() {
        for (channel, bytes) in output.row(y).as_chunks::<4>().0.iter().enumerate() {
            let index = y as usize * output.width() as usize * 3 + channel;
            let expected =
                f32::from_ne_bytes(reference[index * 4..index * 4 + 4].try_into().unwrap());
            let wrong = f32::from_ne_bytes(reversed[index * 4..index * 4 + 4].try_into().unwrap());
            let actual = f32::from_ne_bytes(*bytes);
            nonfinite +=
                usize::from(!actual.is_finite() || !expected.is_finite() || !wrong.is_finite());
            out_of_gamut += usize::from(!(0.0..=1.0).contains(&expected));
            // EncodeReady::finalize deliberately uses clipped permissive output.
            let expected_clipped = expected.clamp(0.0, 1.0);
            let delta = (f64::from(actual) - f64::from(expected_clipped)).abs();
            violations += usize::from(delta >= 0.0013);
            polarity_differences +=
                usize::from((wrong.clamp(0.0, 1.0) - expected_clipped).abs() >= 0.0013);
            if delta > max_abs {
                max_abs = delta;
                let x = channel / 3;
                let input: Vec<f32> = buffer.as_slice().row(y)[x * 16..x * 16 + 16]
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|v| f32::from_ne_bytes(*v))
                    .collect();
                worst = serde_json::json!({"x":x,"y":y,"channel":channel%3,
                    "native_inverted_cmyk":input,"actual":actual,
                    "reference_unclipped":expected,"reference_clipped":expected_clipped});
            }
        }
    }
    assert_eq!(buffer.as_slice().as_strided_bytes(), original);
    assert_eq!(buffer.color_context(), context.as_ref());
    jxl_packaged_storage::assert_charges(&storage, &[&image]);
    drop(image);
    assert_eq!(
        (storage.reserved_bytes(), storage.reserved_objects()),
        (0, 0)
    );
    let report = serde_json::json!({"samples":samples,"max_absolute_error":max_abs,
        "strict_bound":0.0013,"violations":violations,"nonfinite":nonfinite,
        "accuracy_qualified":violations == 0,"adapter_matches_backend":true,
        "wrong_polarity_differences":polarity_differences,"lcms_out_of_gamut":out_of_gamut,
        "intent":1,"flags":"BLACKPOINTCOMPENSATION|HIGHRESPRECALC",
        "finalizer_clip":true,"worst":worst});
    fs::write(
        artifacts.join("comparison.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    eprintln!("native CMYK finalization: {report}");
    assert_eq!(nonfinite, 0);
    assert!(
        polarity_differences > 0,
        "oracle must detect wrong ink polarity"
    );
    match expectation {
        #[cfg(lcms_native)]
        AccuracyExpectation::Required => {
            assert_eq!(violations, 0, "independent CMYK bound exceeded: {report}");
        }
        AccuracyExpectation::KnownMoxDisagreement => {
            // Owner-approved split: routing must remain bit-exact above, while
            // this pinned CMM is explicitly not qualified at the strict bound.
            // A numerical fix makes this fail so it must be promoted to Required.
            assert!(
                violations > 0,
                "Mox discrepancy no longer reproduced; qualify the strict gate: {report}"
            );
        }
    }
}
