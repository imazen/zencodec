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
