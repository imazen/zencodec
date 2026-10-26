//! Test-only, independently executed ICC math after native reconstruction.
use super::*;
use std::{fs, io::Write, path::Path, process::Command};
use zencodec::{
    image::{Meaning, Row},
    operation::{Decision, PolicySet, PublicationOffer, Question, Selection, resolve_image},
};
use zenpixels::{AlphaMode, PixelFormat};
use zenpixels_convert::icc_profiles::display_p3_icc;

fn save(path: &Path, data: &[u8]) {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .unwrap()
        .write_all(data)
        .unwrap();
}

fn save_rows(path: &Path, pixels: PixelSlice<'_>) {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .unwrap();
    for y in 0..pixels.rows() {
        file.write_all(pixels.row(y)).unwrap();
    }
}

mod planning {
    // EXAMPLE: native_float_color
    use std::sync::Arc;
    use zencodec::At;
    use zenpixels::PixelSlice;
    use zenpixels_convert::{
        ColorOrigin, ConvertError, ConvertOptions, OutputPlan, OutputProfile, cms_moxcms::MoxCms,
    };

    pub(super) fn plan_float_export(
        source: PixelSlice<'_>,
        origin: &ColorOrigin,
        target_icc: Arc<[u8]>,
    ) -> Result<OutputPlan, At<ConvertError>> {
        // ANCHOR: native_float_color
        let options = ConvertOptions::forbid_lossy().with_clip_out_of_gamut(false);
        let plan = OutputPlan::new(
            source.descriptor(),
            source.color_context().cloned(),
            origin,
            OutputProfile::Icc(target_icc),
            source.descriptor().pixel_format(),
            &options,
            Some(&MoxCms),
        )?;
        // END: native_float_color
        // The adapter prepares row capacity, admits destination storage, and
        // executes this same plan. Its metadata describes successful output.
        // This color stage does not publish a new auxiliary-plane owner.
        Ok(plan)
    }
    // END EXAMPLE: native_float_color
}

#[test]
fn native_color_profiles_match_independent_lcms_with_exact_alpha_and_admission() {
    let oracle =
        std::env::var_os("SPATIAL_COLOR_ORACLE").expect("supply independent Little CMS executable");
    let artifacts = std::path::PathBuf::from(
        std::env::var_os("SPATIAL_COLOR_ARTIFACTS").expect("supply fresh color artifact directory"),
    );
    fs::create_dir_all(&artifacts).unwrap();
    let target: Arc<[u8]> = Arc::from(display_p3_icc(false));
    let mut total = 0;
    let mut color_failures = Vec::new();
    for (name, view, count) in [
        (
            "conformance_test_images/animation_spline.jxl",
            Composition::Composited,
            1,
        ),
        (
            "multiple_layers_noise_spline.jxl",
            Composition::Uncomposited,
            5,
        ),
    ] {
        let bytes = fixture(name);
        let mut source = ReadAtShim(SliceReadAt::new(&bytes));
        let (codecs, pool) = jxl_packaged_storage::factories(64, 64);
        let mut images =
            jxl_packaged_storage::open_admitted(&codecs, &mut source, view, 64, &pool).unwrap();
        let mut cursor = images.images().unwrap();
        for _ in 0..count {
            let id = images
                .next(&mut cursor)
                .unwrap()
                .expect("required native selection");
            let image = images
                .decode(&id, &ImageRequest::new())
                .unwrap()
                .into_output();
            let directory = artifacts.join(format!("selection-{total}"));
            fs::create_dir(&directory).unwrap();
            let original = image.clone();
            let source_charges = (pool.reserved_bytes(), pool.reserved_objects());
            let destination_pool = StorageBudget::new(256 * 1024 * 1024, 1);
            let color = image.with_pixels(|pixels| {
                let format = pixels.descriptor().pixel_format();
                assert!(matches!(format, PixelFormat::RgbF32 | PixelFormat::RgbaF32));
                assert!(matches!(pixels.descriptor().alpha(), None | Some(AlphaMode::Straight)));
                let channels = pixels.descriptor().layout().channels();
                let profile = pixels.color_context().unwrap().icc.clone().unwrap();
                save(&directory.join("source.icc"), &profile);
                save(&directory.join("target.icc"), &target);
                save_rows(&directory.join("source.f32"), pixels.clone());
                let mut plan = planning::plan_float_export(
                    pixels.clone(), &ColorOrigin::from_icc(profile), target.clone(),
                ).unwrap();
                assert!(plan.borrow_rows(pixels.clone()).unwrap().is_none());
                let color = convert(pixels.clone(), &mut plan, &destination_pool).unwrap();
                let converted = color.pixels();
                assert_eq!(converted.color_context().unwrap().icc, Some(target.clone()));
                assert_eq!(destination_pool.reserved_bytes(), color.stride * color.rows as usize);
                save_rows(&directory.join("converted.f32"), converted.clone());
                let output = Command::new("timeout").arg("30s").arg(&oracle)
                    .arg(directory.join("source.icc")).arg(directory.join("target.icc"))
                    .arg(directory.join("source.f32")).arg(directory.join("oracle.f32"))
                    .arg(pixels.width().to_string()).arg(pixels.rows().to_string())
                    .arg(channels.to_string()).output().unwrap();
                save(&directory.join("stdout.txt"), &output.stdout);
                save(&directory.join("stderr.txt"), &output.stderr);
                assert!(output.status.success(), "independent CMS failed: {:?}", output.status);
                let expected = fs::read(directory.join("oracle.f32")).unwrap();
                let row_bytes = pixels.width() as usize * channels * 4;
                assert_eq!(expected.len(), row_bytes * pixels.rows() as usize);
                let mut max_abs = 0.0f64;
                let mut worst = (0, 0, 0.0f32, 0.0f32);
                let mut changed = 0usize;
                let mut nonfinite = 0usize;
                let mut alpha_mismatch = 0usize;
                for y in 0..pixels.rows() {
                    for (index, ((actual, reference), original)) in converted.row(y)
                        .as_chunks::<4>().0.iter()
                        .zip(expected[y as usize * row_bytes..(y as usize + 1) * row_bytes].as_chunks::<4>().0)
                        .zip(pixels.row(y).as_chunks::<4>().0).enumerate() {
                        if channels == 4 && index % channels == 3 {
                            alpha_mismatch += usize::from(actual != original || reference != original);
                        } else {
                            let a = f32::from_ne_bytes(*actual);
                            let b = f32::from_ne_bytes(*reference);
                            nonfinite += usize::from(!a.is_finite() || !b.is_finite());
                            let difference = (f64::from(a) - f64::from(b)).abs();
                            if difference > max_abs {
                                max_abs = difference;
                                worst = (y, index, a, b);
                            }
                            changed += usize::from(actual != original);
                        }
                    }
                    let all = bytemuck::cast_slice::<u32, u8>(color.data.as_slice());
                    assert!(all[y as usize * color.stride + row_bytes..(y as usize + 1) * color.stride]
                        .iter().all(|b| *b == 0xa5));
                }
                let report = serde_json::json!({
                    "fixture": name, "selection": total, "width": pixels.width(), "height": pixels.rows(),
                    "channels": channels, "intent": "relative-colorimetric", "lcms_flags": "NOOPTIMIZE|COPY_ALPHA",
                    "clip_out_of_gamut": false,
                    "max_absolute_color_error": max_abs, "color_error_limit": 0.0001,
                    "worst": worst, "nonfinite": nonfinite, "alpha_mismatch": alpha_mismatch,
                    "changed_color_samples": changed, "admitted_destination_bytes": destination_pool.reserved_bytes(),
                    "oracle_identity": String::from_utf8_lossy(&output.stdout).trim(),
                });
                save(&directory.join("comparison.json"), &serde_json::to_vec_pretty(&report).unwrap());
                assert_eq!(nonfinite, 0, "{report}");
                assert_eq!(alpha_mismatch, 0, "{report}");
                assert!(changed > 0, "profile conversion exercised no changed samples: {report}");
                // Frozen before the first run: independent F32 CMS arithmetic
                // may differ, but alpha and same-route sample copies are exact.
                if max_abs > 0.0001 {
                    color_failures.push(report);
                }
                color
            }).unwrap();
            let color_address = color.pixels().as_strided_bytes().as_ptr();
            let color_charge = color.data.charged_bytes();
            let derived = image.replace_packed_color(color).unwrap();
            assert!(std::ptr::eq(
                original.interpretation(),
                derived.interpretation()
            ));
            let (auxiliaries, auxiliary_bytes) = {
                let before = original.map().unwrap();
                let after = derived.map().unwrap();
                assert_eq!(before.plane_count(), after.plane_count());
                let mut auxiliary_bytes = 0;
                for index in 1..before.plane_count() {
                    let a = before.plane(index).unwrap().unwrap();
                    let b = after.plane(index).unwrap().unwrap();
                    auxiliary_bytes += a.samples.stride_bytes() * a.samples.dimensions().1;
                    assert!(matches!(a.meaning, Meaning::Auxiliary(_)));
                    assert_eq!(a.meaning, b.meaning);
                    assert_eq!(a.samples.dimensions(), b.samples.dimensions());
                    assert_eq!(a.samples.stride_bytes(), b.samples.stride_bytes());
                    for y in 0..a.samples.dimensions().1 {
                        let (Some(Row::Bytes(a)), Some(Row::Bytes(b))) =
                            (a.samples.row(y), b.samples.row(y))
                        else {
                            panic!("native scalar values")
                        };
                        assert_eq!(a.as_ptr(), b.as_ptr());
                        assert_eq!(a, b);
                    }
                }
                (before.plane_count() - 1, auxiliary_bytes)
            };
            assert_eq!(
                (pool.reserved_bytes(), pool.reserved_objects()),
                source_charges
            );
            drop(original);
            assert_eq!(
                (pool.reserved_bytes(), pool.reserved_objects()),
                (auxiliary_bytes, auxiliaries)
            );
            let mut calls = 0;
            let published = resolve_image(
                derived,
                &mut |question: Question<'_>| -> Result<Decision, zencodec::At<core::convert::Infallible>> {
                    calls += 1;
                    let Question::Image { image, facts, .. } = question else { panic!() };
                    assert_eq!(facts.auxiliaries, auxiliaries);
                    image.with_pixels(|pixels| {
                        assert_eq!(pixels.as_strided_bytes().as_ptr(), color_address);
                        assert_eq!(pixels.color_context().unwrap().icc, Some(target.clone()));
                    }).unwrap();
                    Ok(Decision::UsePreset)
                },
                PolicySet::new(), &zencodec::Unstoppable,
            ).unwrap();
            assert_eq!(calls, 1);
            assert!(matches!(
                published.report().decisions().next().unwrap().selected,
                Some(Selection::Image(PublicationOffer::ReturnHeld))
            ));
            assert_eq!(destination_pool.reserved_bytes(), color_charge);
            save(
                &directory.join("publication.json"),
                &serde_json::to_vec_pretty(&serde_json::json!({
                    "auxiliary_planes_retained": auxiliaries,
                    "source_bytes_before": source_charges.0,
                    "source_bytes_after": pool.reserved_bytes(),
                    "auxiliary_bytes": auxiliary_bytes,
                    "source_color_released": true,
                    "destination_bytes": color_charge,
                    "publication": "ReturnHeld",
                    "same_auxiliary_addresses": true,
                }))
                .unwrap(),
            );
            drop(published);
            assert_eq!(
                (
                    destination_pool.reserved_bytes(),
                    destination_pool.reserved_objects()
                ),
                (0, 0)
            );
            assert_eq!((pool.reserved_bytes(), pool.reserved_objects()), (0, 0));
            total += 1;
        }
    }
    assert_eq!(total, 6);
    assert!(
        color_failures.is_empty(),
        "independent CMS disagreements: {color_failures:?}"
    );
}
