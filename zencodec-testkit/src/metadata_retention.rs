//! Compare required metadata against completed output readback.
use crate::{Conformance, Metadata, fail};

/// Require every supplied field to survive completed output readback.
///
/// Pass the resolved output record (after policy filtering and any explicitly
/// requested spatial transform) and metadata inspected from the
/// finished encoded output, preferably by an independent implementation. This
/// helper never consults capability flags: a missing required field fails even
/// when a codec advertises that carrier as unsupported. Check admission errors
/// separately; an arbitrary encode error is not evidence of correct refusal.
///
/// Opaque ICC, EXIF and XMP bytes must match exactly. Typed values must compare
/// equal, including orientation; no HDR rounding tolerance is implied. Apply
/// any authorized EXIF/orientation reconciliation before constructing `expected`.
/// Unknown/incomplete readback is not proof of retention and must not be passed
/// off as complete inspection. This helper does not inspect opaque data for PII,
/// prove pixel/color consistency, or reject additional format-mandated metadata.
/// Use privacy conformance separately to check discarded or unexpected fields.
pub fn check_metadata_retained(expected: &Metadata, actual: &Metadata) -> Conformance {
    let matches = [
        (
            "ICC",
            expected.icc_profile.is_none() || expected.icc_profile == actual.icc_profile,
        ),
        (
            "EXIF",
            expected.exif.is_none() || expected.exif == actual.exif,
        ),
        ("XMP", expected.xmp.is_none() || expected.xmp == actual.xmp),
        (
            "CICP",
            expected.cicp.is_none() || expected.cicp == actual.cicp,
        ),
        (
            "content light level",
            expected.content_light_level.is_none()
                || expected.content_light_level == actual.content_light_level,
        ),
        (
            "mastering display",
            expected.mastering_display.is_none()
                || expected.mastering_display == actual.mastering_display,
        ),
        (
            "diffuse white",
            expected.diffuse_white.is_none() || expected.diffuse_white == actual.diffuse_white,
        ),
        ("orientation", expected.orientation == actual.orientation),
    ];
    let missing: Vec<_> = matches
        .into_iter()
        .filter_map(|(name, matches)| (!matches).then_some(name))
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        // Name channels without leaking arbitrary metadata into diagnostics.
        Err(fail(
            "metadata_retained",
            format!(
                "required metadata missing or changed: {}",
                missing.join(", ")
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zencodec::{Cicp, ContentLightLevel, Orientation};

    #[test]
    fn detects_silent_omission_and_substitution_without_capability_flags() {
        let expected = Metadata::none()
            .with_icc(&b"opaque secret profile"[..])
            .with_exif(&b"opaque secret exif"[..])
            .with_xmp(&b"opaque secret xmp"[..])
            .with_cicp(Cicp::SRGB)
            .with_content_light_level(ContentLightLevel {
                max_content_light_level: 1000,
                max_frame_average_light_level: 400,
            })
            .with_mastering_display(zencodec::MasteringDisplay::HDR10_REFERENCE)
            .with_diffuse_white(zenpixels::hdr::DiffuseWhite::new(100.0));
        check_metadata_retained(&expected, &expected).unwrap();
        let failure = check_metadata_retained(&expected, &Metadata::none()).unwrap_err();
        for field in [
            "ICC",
            "EXIF",
            "XMP",
            "CICP",
            "content light level",
            "mastering display",
            "diffuse white",
        ] {
            assert!(failure.detail.contains(field));
        }
        assert!(!failure.detail.contains("secret"));
        type Substitution = (&'static str, fn(&mut Metadata));
        let substitutions: [Substitution; 7] = [
            ("ICC", |m| m.icc_profile = Some((&b"changed"[..]).into())),
            ("EXIF", |m| m.exif = Some((&b"changed"[..]).into())),
            ("XMP", |m| m.xmp = Some((&b"changed"[..]).into())),
            ("CICP", |m| m.cicp.as_mut().unwrap().color_primaries = 9),
            ("content light level", |m| {
                m.content_light_level
                    .as_mut()
                    .unwrap()
                    .max_content_light_level += 1
            }),
            ("mastering display", |m| {
                m.mastering_display.as_mut().unwrap().max_luminance += 1.0
            }),
            ("diffuse white", |m| {
                m.diffuse_white = Some(zenpixels::hdr::DiffuseWhite::new(101.0))
            }),
        ];
        for (field, substitute) in substitutions {
            let mut changed = expected.clone();
            substitute(&mut changed);
            assert_eq!(
                check_metadata_retained(&expected, &changed)
                    .unwrap_err()
                    .detail,
                format!("required metadata missing or changed: {field}")
            );
        }
        let changed = expected.clone().with_xmp(&b"different"[..]);
        assert_eq!(
            check_metadata_retained(&expected, &changed)
                .unwrap_err()
                .detail,
            "required metadata missing or changed: XMP"
        );
        let mut changed = expected.clone();
        changed.orientation = Orientation::Rotate90;
        assert!(check_metadata_retained(&expected, &changed).is_err());
    }

    #[test]
    fn additional_signaling_is_outside_retention_only_scope() {
        let actual = Metadata::none().with_cicp(Cicp::SRGB);
        check_metadata_retained(&Metadata::none(), &actual).unwrap();
        let empty_present = Metadata::none().with_xmp(&b""[..]);
        assert!(check_metadata_retained(&empty_present, &Metadata::none()).is_err());
    }
}
