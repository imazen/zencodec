//! Private source-admission witness; no new public source/transport API.
#[allow(dead_code)]
#[path = "../src/fixtures.rs"]
mod fixtures;
#[path = "../../spatial-validation/src/http_source.rs"]
mod http_source;
use std::{error::Error, path::PathBuf};
use zencodec::{
    Pulse,
    source::ReadAt,
    storage::{SharedStorage, StorageBudget},
};
use zencodec_media::{frame::Subsampling, packet::Packet};

// The ReadAt implementation must enforce a coherent representation during the
// copy. Freezing prevents subsequent source changes from affecting replay.
fn snapshot<S: ReadAt>(
    source: &mut S,
    budget: &StorageBudget,
    pulse: Option<&dyn Pulse>,
) -> Result<SharedStorage<u8>, Box<dyn Error>> {
    if let Some(pulse) = pulse {
        pulse
            .check()
            .map_err(zencodec_media::session::SessionError::<std::io::Error>::Cancelled)?;
    }
    let len = usize::try_from(source.len_hint().ok_or("unknown snapshot length")?)?;
    let mut bytes = budget.bytes(len)?; // Admission precedes body reads.
    for (index, chunk) in bytes.as_mut_slice().chunks_mut(256).enumerate() {
        if let Some(pulse) = pulse {
            pulse
                .check()
                .map_err(zencodec_media::session::SessionError::<std::io::Error>::Cancelled)?;
        }
        source.read_exact_at((index * 256) as u64, chunk)?;
        if let Some(pulse) = pulse {
            pulse
                .check()
                .map_err(zencodec_media::session::SessionError::<std::io::Error>::Cancelled)?;
        }
    }
    Ok(bytes.freeze())
}

#[test]
fn native_replay_uses_admitted_immutable_file_and_conditional_http_snapshots() {
    let root = PathBuf::from(std::env::var_os("SPATIAL_HTTP_ARTIFACTS").unwrap());
    let url = std::env::var("SPATIAL_HTTP_URL").unwrap();
    let settings = fixtures::settings(8, Subsampling::Yuv444, true);
    let frames: Vec<_> = (0..3).map(|i| fixtures::frame(&settings, i)).collect();
    let (config, packets) = fixtures::encode(settings, &frames);
    let bytes: Vec<_> = packets
        .iter()
        .flat_map(|packet| packet.bytes())
        .copied()
        .collect();
    let path = root.join("replay.bin");
    std::fs::write(&path, &bytes).unwrap();
    let verify = |saved: &SharedStorage<u8>| {
        let mut offset = 0;
        let replay: Vec<_> = packets
            .iter()
            .map(|packet| {
                let end = offset + packet.bytes().len();
                let view = saved.slice(offset..end).unwrap();
                offset = end;
                Packet::from_accounted(view)
                    .unwrap()
                    .with_pts(packet.pts().unwrap())
                    .with_duration_ticks(packet.duration_ticks().unwrap())
            })
            .collect();
        for _ in 0..2 {
            let mut settings = rav1d_safe::Settings::default();
            settings.threads = 1;
            let mut decoder = zenavif::media::Av1VideoDecoder::with_settings(settings).unwrap();
            let actual = fixtures::decode(&mut decoder, &config, &replay, None).unwrap();
            assert_eq!(actual.len(), frames.len());
            for (actual, expected) in actual.iter().zip(&frames) {
                assert_eq!(fixtures::samples(actual), fixtures::samples(expected));
            }
        }
    };
    let budget = StorageBudget::new(bytes.len(), 1);
    let mut file = zencodec_io::FileReadAt::open(&path).unwrap();
    let saved = snapshot(&mut file, &budget, None).unwrap();
    drop(file); // No external mutation during FileReadAt's coherent-read contract.
    std::fs::write(&path, vec![0; bytes.len()]).unwrap();
    verify(&saved);
    drop(saved);
    assert_eq!((budget.reserved_bytes(), budget.reserved_objects()), (0, 0));
    std::fs::write(&path, &bytes).unwrap();

    let mut http = http_source::HttpSource::open(format!("{url}/stable/replay.bin"), 256).unwrap();
    let attempts = http.attempts.get();
    let denied = snapshot(&mut http, &StorageBudget::new(bytes.len() - 1, 1), None).unwrap_err();
    assert!(
        denied
            .downcast_ref::<zencodec::storage::StorageError>()
            .is_some()
    );
    assert_eq!(
        http.attempts.get(),
        attempts,
        "refusal consumed source bytes"
    );
    let saved = snapshot(&mut http, &budget, None).unwrap();
    verify(&saved);
    let retained = saved.clone();
    drop(saved);
    assert_eq!(budget.reserved_bytes(), bytes.len());
    drop(retained);
    assert_eq!((budget.reserved_bytes(), budget.reserved_objects()), (0, 0));

    for (mode, expected) in [
        ("change", http_source::FailureKind::Status(412)),
        ("changed_tag", http_source::FailureKind::Validator),
        ("wrong_total", http_source::FailureKind::ContentRange),
        ("short_body", http_source::FailureKind::Body),
    ] {
        let mut http =
            http_source::HttpSource::open(format!("{url}/{mode}/replay.bin"), 256).unwrap();
        let error = snapshot(&mut http, &budget, None).unwrap_err();
        assert_eq!(
            error.downcast_ref::<http_source::Failure>().unwrap().kind,
            expected
        );
        assert_eq!((budget.reserved_bytes(), budget.reserved_objects()), (0, 0));
        let attempts = http.attempts.get();
        assert!(http.read_exact_at(0, &mut [0]).is_err());
        assert_eq!(
            http.attempts.get(),
            attempts,
            "failed source silently reopened"
        );
    }
}

struct StopFlag(std::sync::Arc<std::sync::atomic::AtomicBool>);
impl enough::Stop for StopFlag {
    fn check(&self) -> Result<(), enough::StopReason> {
        if self.0.load(std::sync::atomic::Ordering::SeqCst) {
            Err(enough::StopReason::TimedOut)
        } else {
            Ok(())
        }
    }
}
struct StopsAfterRead<S> {
    inner: S,
    flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    reads: usize,
}
impl<S: ReadAt> ReadAt for StopsAfterRead<S> {
    type Error = S::Error;
    fn len(&mut self) -> Result<u64, Self::Error> {
        self.inner.len()
    }
    fn len_hint(&self) -> Option<u64> {
        self.inner.len_hint()
    }
    fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), Self::Error> {
        self.reads += 1;
        self.inner.read_exact_at(offset, out)?;
        self.flag.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}
#[test]
fn snapshot_cancellation_releases_admission_before_any_replay_owner_exists() {
    use std::sync::{Arc, atomic::AtomicBool};
    use zencodec::how_far::StopOnly;
    let flag = Arc::new(AtomicBool::new(false));
    let bytes = [9u8; 1024];
    let mut source = StopsAfterRead {
        inner: zencodec::source::SliceReadAt::new(&bytes),
        flag: flag.clone(),
        reads: 0,
    };
    let budget = StorageBudget::new(bytes.len(), 1);
    let pulse = StopOnly::new(StopFlag(flag));
    let error = snapshot(&mut source, &budget, Some(&pulse)).unwrap_err();
    assert!(matches!(
        error.downcast_ref::<zencodec_media::session::SessionError<std::io::Error>>(),
        Some(zencodec_media::session::SessionError::Cancelled(
            enough::StopReason::TimedOut
        ))
    ));
    assert_eq!(source.reads, 1);
    assert_eq!((budget.reserved_bytes(), budget.reserved_objects()), (0, 0));
    // Already cancelled: neither allocation nor another read occurs.
    assert!(snapshot(&mut source, &budget, Some(&pulse)).is_err());
    assert_eq!(source.reads, 1);
    assert_eq!((budget.reserved_bytes(), budget.reserved_objects()), (0, 0));
}
