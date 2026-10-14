use zencodec::encode::{
    DynEncoderConfig, EncoderConfig, Fidelity, FidelitySelectionError, FidelityTargetError,
};
use zencodec_testkit::{ReferenceEncoderConfig as Reference, TestImage};
use zenpixels::PixelDescriptor;

#[derive(Clone)]
struct Selective {
    target: Option<Fidelity>,
    changed: bool,
}
impl EncoderConfig for Selective {
    type Error = <Reference as EncoderConfig>::Error;
    type Job = <Reference as EncoderConfig>::Job;
    fn format() -> zencodec::ImageFormat {
        <Reference as EncoderConfig>::format()
    }
    fn supported_descriptors() -> &'static [PixelDescriptor] {
        <Reference as EncoderConfig>::supported_descriptors()
    }
    fn with_fidelity(mut self, requested: Fidelity) -> Self {
        // A rejected candidate poisons this instance: preference selection must
        // start again from the original rather than chaining candidate setters.
        self.target = if !self.changed && requested == Fidelity::codec_quality(80.0) {
            Some(requested)
        } else {
            Some(Fidelity::codec_quality(50.0))
        };
        self.changed = true;
        self
    }
    fn resolved_target_fidelity(&self) -> Option<Fidelity> {
        self.target
    }
    fn job(self) -> Self::Job {
        Reference::new().job()
    }
}
fn config() -> Selective {
    Selective {
        target: None,
        changed: false,
    }
}

#[test]
fn selection_is_ordered_atomic_and_shared_by_erased_jobs() {
    let c = config();
    let preferences = [
        Fidelity::Lossless,
        Fidelity::ssim2(80.0),
        Fidelity::codec_quality(80.0),
    ];
    assert_eq!(
        EncoderConfig::check_fidelity_target(&c, preferences[0]),
        Err(FidelityTargetError::Substituted {
            resolved: Fidelity::codec_quality(50.0)
        })
    );
    let (index, selected) = c.select_fidelity(&preferences).unwrap();
    assert_eq!(index, 2);
    assert_eq!(selected.resolved_target_fidelity(), Some(preferences[2]));
    assert!(!c.changed);
    let dynamic: &dyn DynEncoderConfig = &c;
    for target in preferences {
        assert_eq!(
            EncoderConfig::check_fidelity_target(&c, target),
            dynamic.check_fidelity_target(target)
        );
    }
    let (index, job) = dynamic.select_fidelity_job(&preferences).unwrap();
    assert_eq!(index, 2);
    job.encode(TestImage::rgb8_gradient(3, 2).as_slice())
        .unwrap();
    assert_eq!(
        c.select_fidelity(&[preferences[2], preferences[0]])
            .unwrap()
            .0,
        0
    );
}

#[test]
fn rejection_report_retains_every_index_target_and_cause() {
    let preferences = [Fidelity::Lossless, Fidelity::butteraugli(1.0)];
    let Err(FidelitySelectionError::Rejected(errors)) = config().select_fidelity(&preferences)
    else {
        panic!("expected rejected list")
    };
    assert_eq!(errors.len(), preferences.len());
    for (index, rejection) in errors.iter().enumerate() {
        assert_eq!(rejection.index, index);
        assert_eq!(rejection.target, preferences[index]);
        assert_eq!(
            rejection.reason,
            FidelityTargetError::Substituted {
                resolved: Fidelity::codec_quality(50.0)
            }
        );
    }
    let dynamic: &dyn DynEncoderConfig = &config();
    let Err(FidelitySelectionError::Rejected(erased)) = dynamic.select_fidelity_job(&preferences)
    else {
        panic!("expected rejected list")
    };
    assert_eq!(errors, erased);
}

#[test]
fn malformed_and_empty_lists_do_not_silently_fall_back() {
    assert!(matches!(
        config().select_fidelity(&[]),
        Err(FidelitySelectionError::Empty)
    ));
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        for target in [
            Fidelity::codec_quality(value),
            Fidelity::ssim2(value),
            Fidelity::butteraugli(value),
            Fidelity::zensim_b(value),
        ] {
            assert!(matches!(
                config().try_with_fidelity(target),
                Err(FidelityTargetError::NonFinite)
            ));
            // Invalid entries are rejected even after a supported preference.
            assert!(matches!(
                config().select_fidelity(&[Fidelity::codec_quality(80.0), target]),
                Err(FidelitySelectionError::InvalidPreference {
                    index: 1,
                    reason: FidelityTargetError::NonFinite
                })
            ));
        }
    }
}

#[test]
fn lossless_reference_confirms_lossless_but_reports_lossy_substitution() {
    let c = Reference::new();
    assert!(c.clone().try_with_fidelity(Fidelity::Lossless).is_ok());
    assert!(matches!(
        c.try_with_fidelity(Fidelity::codec_quality(80.0)),
        Err(FidelityTargetError::Substituted {
            resolved: Fidelity::Lossless
        })
    ));
}

#[test]
fn codec_set_exposes_checked_preferences_without_a_second_registry() {
    let set = zencodec::CodecSet::new().with_encoder(Reference::new());
    let encoder = set
        .encoder_for(<Reference as EncoderConfig>::format())
        .unwrap();
    let (index, job) = encoder
        .select_fidelity_job(&[Fidelity::codec_quality(80.0), Fidelity::Lossless])
        .unwrap();
    assert_eq!(index, 1);
    let pixels = TestImage::rgb8_gradient(3, 2);
    let output = job.encode(pixels.as_slice()).unwrap();
    assert!(!output.data().is_empty());
}
