//! Independent CMS runs on stored source samples. A separate EXIF table maps
//! those results into the rendered canvas: no production projection/conversion
//! helper participates in expected-value generation.
use super::*;
use std::{fs, io::Write, path::Path, process::Command};

fn save(path: &Path, data: &[u8]) {
    fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .unwrap()
        .write_all(data)
        .unwrap();
}

#[test]
fn canvas_render_independent_cms_after_native_jxl_and_tiff_reconstruction() {
    let root = std::path::PathBuf::from(
        std::env::var_os("SPATIAL_COLOR_ARTIFACTS").expect("set fresh artifact directory"),
    )
    .join("combined-render");
    fs::create_dir_all(&root).unwrap();
    let oracle =
        std::env::var_os("SPATIAL_COLOR_ORACLE").expect("supply pinned independent CMS helper");
    let mut sources = Vec::new();
    for name in [
        "orientation1_identity.jxl",
        "orientation2_flip_horizontal.jxl",
        "orientation3_rotate_180.jxl",
        "orientation4_flip_vertical.jxl",
        "orientation5_transpose.jxl",
        "orientation6_rotate_90_cw.jxl",
        "orientation7_anti_transpose.jxl",
        "orientation8_rotate_90_ccw.jxl",
        "conformance_test_images/animation_spline.jxl",
    ] {
        sources.push((name.to_owned(), native_jxl(name, Composition::Composited)));
    }
    for exif in 1..=8 {
        sources.push((format!("tiff-{exif}"), native_tiff(exif, exif % 2 == 0, 32)));
    }
    let mut receipts = Vec::new();
    for (index, (name, source)) in sources.into_iter().enumerate() {
        let dir = root.join(index.to_string());
        fs::create_dir(&dir).unwrap();
        let renderer = renderer(
            &source,
            Some(Rect {
                x: 1,
                y: 1,
                width: 3,
                height: 2,
            }),
        );
        let mut original = Vec::new();
        let (width, rows, channels) = source
            .with_pixels(|pixels| {
                assert_eq!(
                    pixels.descriptor().channel_type(),
                    zenpixels::ChannelType::F32
                );
                let profile = pixels.color_context().unwrap().icc.as_ref().unwrap();
                save(&dir.join("source.icc"), profile);
                for y in 0..pixels.rows() {
                    original.extend_from_slice(pixels.row(y));
                }
                (
                    pixels.width(),
                    pixels.rows(),
                    pixels.descriptor().layout().channels(),
                )
            })
            .unwrap();
        save(&dir.join("source.f32"), &original);
        save(&dir.join("target.icc"), display_p3_icc(false));
        let result = Command::new("timeout")
            .args(["--kill-after=2s", "30s"])
            .arg(&oracle)
            .arg(dir.join("source.icc"))
            .arg(dir.join("target.icc"))
            .arg(dir.join("source.f32"))
            .arg(dir.join("expected.f32"))
            .arg(width.to_string())
            .arg(rows.to_string())
            .arg(channels.to_string())
            .output()
            .unwrap();
        save(&dir.join("oracle.stdout"), &result.stdout);
        save(&dir.join("oracle.stderr"), &result.stderr);
        assert!(
            result.status.success(),
            "CMS reader failed: {:?}",
            result.status
        );
        let expected = fs::read(dir.join("expected.f32")).unwrap();
        assert_eq!(expected.len(), original.len());
        let exif = source.interpretation().orientation.unwrap().to_exif();
        let output = renderer
            .render(
                source.clone(),
                &mut PolicySet::new(),
                PolicySet::new(),
                &Unstoppable,
            )
            .unwrap();
        exact(&source, output.output(), false);
        let mut max_abs = 0.0f64;
        let mut compared = 0;
        let mut changed = 0;
        let mut alpha = 0;
        output
            .with_pixels(|pixels| {
                assert_eq!(
                    pixels.color_context().unwrap().icc.as_deref(),
                    Some(display_p3_icc(false))
                );
                let region = output.interpretation().geometry.region;
                let mut actual = Vec::new();
                for y in 0..pixels.rows() {
                    actual.extend_from_slice(pixels.row(y));
                }
                save(&dir.join("actual.f32"), &actual);
                for sy in 0..rows {
                    for sx in 0..width {
                        let (dx, dy) = position(exif, sx, sy, width, rows);
                        if dx < region.x
                            || dy < region.y
                            || dx >= region.x + region.width
                            || dy >= region.y + region.height
                        {
                            continue;
                        }
                        let row = pixels.row(dy - region.y);
                        for channel in 0..channels {
                            let a = ((dx - region.x) as usize * channels + channel) * 4;
                            let b = ((sy * width + sx) as usize * channels + channel) * 4;
                            if channels == 4 && channel == 3 {
                                assert_eq!(&row[a..a + 4], &original[b..b + 4]);
                                assert_eq!(&expected[b..b + 4], &original[b..b + 4]);
                                alpha += 1;
                            } else {
                                let actual = f32::from_ne_bytes(row[a..a + 4].try_into().unwrap());
                                let reference =
                                    f32::from_ne_bytes(expected[b..b + 4].try_into().unwrap());
                                assert!(actual.is_finite() && reference.is_finite());
                                max_abs =
                                    max_abs.max((f64::from(actual) - f64::from(reference)).abs());
                                changed += usize::from(row[a..a + 4] != original[b..b + 4]);
                                compared += 1;
                            }
                        }
                    }
                }
            })
            .unwrap();
        assert_eq!(compared, 18); // six RGB pixels, independent of alpha
        assert!(
            max_abs <= 0.0001,
            "{name}: independent float CMS difference {max_abs}"
        );
        if name.ends_with(".jxl") {
            assert!(changed > 0, "color conversion not exercised");
        }
        receipts.push(
            serde_json::json!({"source":name,"orientation":exif,"color_samples":compared,
            "max_absolute_difference":max_abs,"limit":0.0001,"changed_color_samples":changed,
            "exact_alpha_samples":alpha,"held_output_bytes":renderer.storage.reserved_bytes(),
            "held_output_objects":renderer.storage.reserved_objects(),
            "oracle":String::from_utf8_lossy(&result.stdout).trim()}),
        );
        drop(output);
        assert_eq!(renderer.storage.reserved_bytes(), 0);
    }
    save(
        &root.join("results.json"),
        &serde_json::to_vec_pretty(&receipts).unwrap(),
    );
}
