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
        let preview = guide::preview(&codecs, &source).unwrap();
        assert_eq!(
            (preview.width(), preview.height()),
            (fixtures::WIDTH, fixtures::HEIGHT)
        );
        let retained = guide::derivative(&codecs, &source).unwrap_or_else(|error| {
            let mut detail = error.to_string();
            let mut cause = std::error::Error::source(&error);
            while let Some(e) = cause {
                detail.push_str(&format!("\nCaused by: {e:?}"));
                cause = e.source();
            }
            panic!("{detail}")
        });
        assert_eq!(&retained.data()[..8], b"\x89PNG\r\n\x1a\n");
        let request = guide::web_request();
        let expected = request.transcode(&codecs, &source).unwrap();
        let conditional = guide::for_tenant(&request, &codecs, &source, false).unwrap();
        assert_eq!(conditional.data(), expected.data());
        let refusal = guide::for_tenant(&request, &codecs, &source, true).unwrap_err();
        assert!(zencodec::find_cause::<guide::WorkspaceEvidenceRequired>(&refusal).is_some());
        let path = artifacts.join(format!("checked-guide-{progressive}.jpg"));
        std::fs::write(&path, &source).unwrap();
        let file = guide::from_file(&request, &codecs, &path).unwrap();
        assert_eq!(file.data(), expected.data());
        let network =
            complete(guide::from_network(&request, &codecs, &mut Bytes(&source))).unwrap();
        assert_eq!(network.data(), expected.data());
        assert_eq!(
            network.report().decisions().count(),
            expected.report().decisions().count()
        );
    }
}
