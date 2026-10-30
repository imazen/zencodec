use crate::{checked_guide as guide, fixtures};
use core::{
    future::Future,
    task::{Context, Poll, Waker},
};
use zencodec_io::tokio_io::AsyncSource;

struct Bytes<'a>(&'a [u8]);
impl AsyncSource for Bytes<'_> {
    async fn read_at(
        &mut self,
        offset: u64,
        out: &mut [u8],
    ) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        let remaining = self.0.get(usize::try_from(offset)?..).unwrap_or_default();
        let n = remaining.len().min(out.len()).min(31);
        out[..n].copy_from_slice(&remaining[..n]);
        Ok(n)
    }
    async fn len(&mut self) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self.0.len() as u64)
    }
    fn len_hint(&self) -> Option<u64> {
        Some(self.0.len() as u64)
    }
}
fn complete<F: Future>(future: F) -> F::Output {
    let mut future = Box::pin(future);
    let mut cx = Context::from_waker(Waker::noop());
    for _ in 0..100_000 {
        if let Poll::Ready(output) = future.as_mut().poll(&mut cx) {
            return output;
        }
    }
    panic!("guide future made no bounded progress");
}

#[test]
fn public_guide_executes_all_transport_and_policy_recipes() {
    let artifacts = std::path::PathBuf::from(
        std::env::var_os("JPEG_POLICY_ARTIFACTS")
            .expect("the caller must select the native fixture output directory"),
    );
    std::fs::create_dir_all(&artifacts).unwrap();
    let codecs = guide::gallery_codecs();
    for progressive in [false, true] {
        let source = fixtures::fixture(progressive, fixtures::Placement::Header);
        let png = guide::derivative(&codecs, &source).unwrap();
        for (format, source) in [("jpeg", source.as_slice()), ("png", png.data())] {
            let preview = guide::preview(&codecs, source).unwrap();
            assert_eq!(
                (preview.width(), preview.height()),
                (fixtures::WIDTH, fixtures::HEIGHT)
            );
            let retained = guide::derivative(&codecs, source).unwrap_or_else(|error| {
                let mut detail = error.to_string();
                let mut cause = std::error::Error::source(&error);
                while let Some(e) = cause {
                    detail.push_str(&format!("\nCaused by: {e:?}"));
                    cause = e.source();
                }
                panic!("{detail}")
            });
            assert_eq!(&retained.data()[..8], b"\x89PNG\r\n\x1a\n");
            let (request, mut policy) = guide::web_request();
            let expected = request
                .transcode_with(&codecs, source, &mut policy)
                .unwrap();
            let conditional = guide::for_tenant(&request, &codecs, source, policy, false).unwrap();
            assert_eq!(conditional.data(), expected.data());
            let refusal = guide::for_tenant(&request, &codecs, source, policy, true).unwrap_err();
            assert!(zencodec::find_cause::<guide::WorkspaceEvidenceRequired>(&refusal).is_some());
            let path = artifacts.join(format!("checked-guide-{progressive}.{format}"));
            std::fs::write(&path, source).unwrap();
            let file = guide::from_file(&request, &codecs, &path, &mut policy).unwrap();
            assert_eq!(file.data(), expected.data());
            let network = complete(guide::from_network(
                &request,
                &codecs,
                &mut Bytes(source),
                &mut policy,
            ))
            .unwrap();
            assert_eq!(network.data(), expected.data());
            assert_eq!(
                network.report().decisions().count(),
                expected.report().decisions().count()
            );
        }
    }
}

#[test]
fn public_guide_required_density_survives_or_refuses_before_encoding() {
    use zencodec::{Resolution, ResolutionError, ResolutionUnit};
    let codecs = guide::gallery_codecs();
    let source = fixtures::fixture(false, fixtures::Placement::Header);
    let preview = guide::preview(&codecs, &source).unwrap();
    for resolution in [
        Resolution {
            x: 254.0,
            y: 127.0,
            unit: ResolutionUnit::Inch,
        },
        Resolution {
            x: 100.0,
            y: 50.0,
            unit: ResolutionUnit::Centimeter,
        },
        Resolution {
            x: 120.0,
            y: 80.0,
            unit: ResolutionUnit::Unknown,
        },
    ] {
        let encoded = guide::with_density(&codecs, preview.pixels(), resolution).unwrap();
        // The native integration also checks pHYs with an independent PNG reader.
        // This gate executes the exact example and its default transcode policy.
        let decoded = guide::preview(&codecs, encoded.data()).unwrap();
        assert!(resolution.is_equivalent_to(&decoded.info().resolution.unwrap()));
        let retained = guide::derivative(&codecs, encoded.data()).unwrap();
        let decoded = guide::preview(&codecs, retained.data()).unwrap();
        assert!(resolution.is_equivalent_to(&decoded.info().resolution.unwrap()));
        for row in 0..preview.height() {
            assert_eq!(preview.pixels().row(row), decoded.pixels().row(row));
        }
    }
    let error = guide::with_density(
        &codecs,
        preview.pixels(),
        Resolution {
            x: 300.0,
            y: 150.0,
            unit: ResolutionUnit::Inch,
        },
    )
    .unwrap_err();
    assert!(matches!(
        zencodec::find_cause::<ResolutionError>(&error),
        Some(ResolutionError::NotRepresentable)
    ));
}

mod cancellation_and_publication {
    use super::{fixtures, guide};
    use std::{
        cell::Cell,
        path::{Path, PathBuf},
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
    };
    use zencodec::operation::{BoundPolicy, Decision, OperationError, Question};
    use zencodec::{
        At, SharedPulse,
        enough::{Stop, StopReason},
        how_far::StopOnly,
    };

    #[derive(Clone)]
    struct Cancel(Arc<AtomicBool>);
    impl Stop for Cancel {
        fn check(&self) -> Result<(), StopReason> {
            if self.0.load(Ordering::SeqCst) {
                Err(StopReason::Cancelled)
            } else {
                Ok(())
            }
        }
    }

    const SENTINEL: &[u8] = b"existing destination must survive refusal";

    fn destinations(label: &str) -> (PathBuf, PathBuf) {
        let artifacts = PathBuf::from(
            std::env::var_os("JPEG_POLICY_ARTIFACTS")
                .expect("the caller must select the native fixture output directory"),
        );
        std::fs::create_dir_all(&artifacts).unwrap();
        // Never reuse or remove an earlier run's artifacts. A collision fails.
        let directory = artifacts.join(format!(
            "checked-publication-{label}-{}",
            std::process::id()
        ));
        std::fs::create_dir(&directory).unwrap();
        let absent = directory.join("new.png");
        let existing = directory.join("existing.png");
        std::fs::write(&existing, SENTINEL).unwrap();
        (absent, existing)
    }

    fn assert_untouched(absent: &Path, existing: &Path) {
        assert!(
            !absent.try_exists().unwrap(),
            "refusal created a destination"
        );
        assert_eq!(std::fs::read(existing).unwrap(), SENTINEL);
    }

    #[test]
    fn public_guide_cancellation_handle_reaches_registered_decode() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let codecs = guide::gallery_codecs_with_cancellation(
            SharedPulse::new(StopOnly::new(Cancel(cancelled.clone()))),
            4_000_000,
        );
        let source = fixtures::fixture(false, fixtures::Placement::Header);
        let decoded = guide::preview(&codecs, &source).unwrap();
        assert_eq!(
            (decoded.width(), decoded.height()),
            (fixtures::WIDTH, fixtures::HEIGHT)
        );
        cancelled.store(true, Ordering::SeqCst);
        assert_eq!(codecs.pulse().check(), Err(StopReason::Cancelled));
        let error = guide::preview(&codecs, &source).unwrap_err();
        assert!(matches!(
            zencodec::find_cause::<OperationError>(&error),
            Some(OperationError::Stopped(StopReason::Cancelled))
        ));
    }

    #[test]
    fn public_guide_png_file_matches_completed_bytes_and_report() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let codecs = guide::gallery_codecs_with_cancellation(
            SharedPulse::new(StopOnly::new(Cancel(cancelled))),
            4_000_000,
        );
        for progressive in [false, true] {
            let source = fixtures::fixture(progressive, fixtures::Placement::Header);
            let (request, mut policy) = guide::web_request();
            let expected = request
                .transcode_with(&codecs, &source, &mut policy)
                .unwrap();
            let (destination, existing) = destinations(if progressive {
                "progressive"
            } else {
                "baseline"
            });
            let output =
                guide::transcode_to_file(&request, &codecs, &source, &destination, &mut policy)
                    .unwrap();
            let stored = std::fs::read(&destination).unwrap();
            assert!(stored.starts_with(b"\x89PNG\r\n\x1a\n"));
            assert_eq!(stored.as_slice(), expected.data());
            assert_eq!(stored.as_slice(), output.data());
            assert_eq!(
                output.report().decisions().count(),
                expected.report().decisions().count()
            );
            let decoded = guide::preview(&codecs, &stored).unwrap();
            let expected_pixels = guide::preview(&codecs, expected.data()).unwrap();
            assert_eq!(
                (decoded.width(), decoded.height()),
                (fixtures::WIDTH, fixtures::HEIGHT)
            );
            for row in 0..decoded.height() {
                assert_eq!(decoded.pixels().row(row), expected_pixels.pixels().row(row));
            }
            assert_eq!(std::fs::read(existing).unwrap(), SENTINEL);
        }
    }

    #[test]
    fn public_guide_pre_cancelled_png_file_never_touches_destinations() {
        let cancelled = Arc::new(AtomicBool::new(true));
        let codecs = guide::gallery_codecs_with_cancellation(
            SharedPulse::new(StopOnly::new(Cancel(cancelled))),
            4_000_000,
        );
        let source = fixtures::fixture(false, fixtures::Placement::Header);
        let (request, mut policy) = guide::web_request();
        let (absent, existing) = destinations("pre-cancelled");
        for destination in [&absent, &existing] {
            let error =
                guide::transcode_to_file(&request, &codecs, &source, destination, &mut policy)
                    .unwrap_err();
            assert!(matches!(
                zencodec::find_cause::<OperationError>(error.as_ref()),
                Some(OperationError::Stopped(StopReason::Cancelled))
            ));
            assert_untouched(&absent, &existing);
        }
    }

    #[test]
    fn public_guide_cancellation_after_encode_prevents_publication() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let codecs = guide::gallery_codecs_with_cancellation(
            SharedPulse::new(StopOnly::new(Cancel(cancelled.clone()))),
            4_000_000,
        );
        let source = fixtures::fixture(false, fixtures::Placement::Header);
        let (request, mut policy) = guide::web_request();
        let completed = request
            .transcode_with(&codecs, &source, &mut policy)
            .unwrap();
        assert!(completed.data().starts_with(b"\x89PNG\r\n\x1a\n"));
        let before = completed.data().to_vec();
        let (absent, existing) = destinations("post-encode-cancelled");
        // Deterministic boundary injection, independent of codec check counts.
        cancelled.store(true, Ordering::SeqCst);
        for destination in [&absent, &existing] {
            let error =
                guide::publish_completed(&codecs, completed.output(), destination).unwrap_err();
            assert!(matches!(
                zencodec::find_cause::<OperationError>(error.as_ref()),
                Some(OperationError::Stopped(StopReason::Cancelled))
            ));
            assert_eq!(completed.data(), before.as_slice());
            assert_untouched(&absent, &existing);
        }
    }

    #[derive(Debug)]
    struct ExportRefused;
    impl std::fmt::Display for ExportRefused {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("application refused the export")
        }
    }
    impl std::error::Error for ExportRefused {}

    #[test]
    fn public_guide_refused_encode_never_touches_destinations() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let codecs = guide::gallery_codecs_with_cancellation(
            SharedPulse::new(StopOnly::new(Cancel(cancelled))),
            4_000_000,
        );
        let source = fixtures::fixture(false, fixtures::Placement::Header);
        let (request, requirements) = guide::web_request();
        let (absent, existing) = destinations("policy-refusal");
        for destination in [&absent, &existing] {
            let called = Cell::new(false);
            let mut policy = BoundPolicy::new(
                requirements,
                |_: Question<'_>| -> Result<Decision, At<ExportRefused>> {
                    called.set(true);
                    Err(At::wrap(ExportRefused))
                },
            );
            let error =
                guide::transcode_to_file(&request, &codecs, &source, destination, &mut policy)
                    .unwrap_err();
            assert!(
                called.get(),
                "the bound application callback was not consulted"
            );
            assert!(zencodec::find_cause::<ExportRefused>(error.as_ref()).is_some());
            assert_untouched(&absent, &existing);
        }
    }
}

#[test]
fn checked_metadata_guide_retains_supplied_xmp_and_refuses_unsupported_hdr() {
    use zencodec::{Metadata, UnsupportedOperation};
    use zenjpeg::JpegDecoderConfig;
    let codecs = guide::metadata_codecs();
    let raw = fixtures::raw();
    let expected = Metadata::none().with_xmp(b"<x xmlns='urn:example'>retained</x>".as_slice());
    let encoded =
        guide::with_required_metadata(&codecs, fixtures::pixels(&raw), expected.clone()).unwrap();
    let info = JpegDecoderConfig::new()
        .probe_full_metadata(encoded.data())
        .unwrap();
    zencodec_testkit::check_metadata_retained(&expected, &info.metadata()).unwrap();
    // Independently inspect the standard APP1 payload bytes, not a decoder's interpretation.
    let namespace = b"http://ns.adobe.com/xap/1.0/\0";
    let payload = [namespace.as_slice(), expected.xmp.as_deref().unwrap()].concat();
    assert!(
        encoded
            .data()
            .windows(payload.len())
            .any(|part| part == payload)
    );
    let hdr = Metadata::none()
        .with_content_light_level(zenpixels::hdr::ContentLightLevel::new(1000, 400));
    let error = guide::with_required_metadata(&codecs, fixtures::pixels(&raw), hdr).unwrap_err();
    assert_eq!(
        zencodec::find_cause::<UnsupportedOperation>(&error),
        Some(&UnsupportedOperation::MetadataRetention)
    );
}
