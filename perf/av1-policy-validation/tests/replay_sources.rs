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

// Alternative to spool-then-copy: one up-front admission, fixed capacity, no
// payload relocation on freeze. Unused capacity stays charged to every view.
fn snapshot_reserved<R: std::io::Read>(
    reader: &mut R,
    budget: &StorageBudget,
    cap: usize,
    pulse: Option<&dyn Pulse>,
) -> Result<SharedStorage<u8>, Box<dyn Error + Send + Sync>> {
    let check = || -> Result<(), Box<dyn Error + Send + Sync>> {
        if let Some(pulse) = pulse {
            pulse
                .check()
                .map_err(zencodec_media::session::SessionError::<std::io::Error>::Cancelled)?;
        }
        Ok(())
    };
    check()?;
    let mut storage = budget.bytes(cap)?;
    let pointer = storage.as_slice().as_ptr();
    let mut used = 0;
    loop {
        check()?;
        let mut probe = [0];
        let out = if used < cap {
            let end = used.saturating_add(256).min(cap);
            &mut storage.as_mut_slice()[used..end]
        } else {
            &mut probe[..]
        };
        let n = match reader.read(out) {
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            other => other?,
        };
        if n > out.len() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "reader exceeded supplied buffer",
            )
            .into());
        }
        check()?;
        if n == 0 {
            break;
        }
        if used == cap {
            return Err(zencodec::source::SourceError::BeyondLimit {
                needed: u64::try_from(cap)?
                    .checked_add(1)
                    .ok_or("snapshot length overflow")?,
                limit: cap as u64,
            }
            .into());
        }
        used += n;
    }
    let saved = storage.freeze().slice(0..used)?;
    assert_eq!(saved.as_slice().as_ptr(), pointer, "freeze copied payload");
    Ok(saved)
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

    // Unknown length: the existing spool confirms EOF before snapshotting.
    // Account for both retained owners during the copy; this is not a single
    // allocation route and is not an operation-wide RAM guarantee.
    for cap in [0, bytes.len() - 1, bytes.len(), bytes.len() + 1] {
        let consumed = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut source = zencodec_io::BufferedReadAt::new(
            ShortReader {
                bytes: &bytes,
                consumed: consumed.clone(),
                stop_at: None,
            },
            zencodec_io::BufferPolicy::Spool { max_bytes: cap },
        );
        assert_eq!(source.len_hint(), None);
        if cap < bytes.len() {
            let error = source.len().unwrap_err();
            assert!(matches!(
                zencodec::find_cause::<zencodec::source::SourceError>(&error),
                Some(zencodec::source::SourceError::BeyondLimit { .. })
            ));
            assert_eq!(consumed.load(std::sync::atomic::Ordering::SeqCst), cap + 1);
            assert_eq!(source.retained_bytes(), cap);
            let before = consumed.load(std::sync::atomic::Ordering::SeqCst);
            assert!(source.len().is_err());
            assert_eq!(consumed.load(std::sync::atomic::Ordering::SeqCst), before);
        } else {
            assert_eq!(source.len().unwrap(), bytes.len() as u64);
            let before = consumed.load(std::sync::atomic::Ordering::SeqCst);
            let saved = snapshot(&mut source, &budget, None).unwrap();
            assert_eq!(consumed.load(std::sync::atomic::Ordering::SeqCst), before);
            assert_eq!(source.retained_bytes(), bytes.len());
            assert_eq!(budget.reserved_bytes(), bytes.len());
            drop(source);
            verify(&saved);
            drop(saved);
            assert_eq!((budget.reserved_bytes(), budget.reserved_objects()), (0, 0));
        }
    }
    let consumed = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut cancelled = zencodec_io::BufferedReadAt::new(
        ShortReader {
            bytes: &bytes,
            consumed: consumed.clone(),
            stop_at: Some(7),
        },
        zencodec_io::BufferPolicy::Spool {
            max_bytes: bytes.len(),
        },
    );
    let error = cancelled.len().unwrap_err();
    assert!(matches!(
        zencodec::find_cause::<zencodec_media::session::SessionError<std::io::Error>>(&error),
        Some(zencodec_media::session::SessionError::Cancelled(
            enough::StopReason::TimedOut
        ))
    ));
    assert_eq!(consumed.load(std::sync::atomic::Ordering::SeqCst), 7);
    assert_eq!(budget.reserved_bytes(), 0);
    drop(cancelled); // A failed acquisition is never turned into a replay owner.

    for cap in [bytes.len() - 1, bytes.len(), bytes.len() + 256] {
        let budget = StorageBudget::new(cap, 1);
        let consumed = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut reader = ShortReader {
            bytes: &bytes,
            consumed: consumed.clone(),
            stop_at: None,
        };
        let result = snapshot_reserved(&mut reader, &budget, cap, None);
        if cap < bytes.len() {
            assert!(matches!(
                result
                    .unwrap_err()
                    .downcast_ref::<zencodec::source::SourceError>(),
                Some(zencodec::source::SourceError::BeyondLimit { .. })
            ));
            assert_eq!(consumed.load(std::sync::atomic::Ordering::SeqCst), cap + 1);
            assert_eq!(budget.reserved_bytes(), 0);
        } else {
            let saved = result.unwrap();
            assert_eq!(saved.len(), bytes.len());
            assert_eq!(saved.charged_bytes(), cap);
            assert_eq!(
                (budget.reserved_bytes(), budget.reserved_objects()),
                (cap, 1)
            );
            verify(&saved);
            drop(saved);
            assert_eq!((budget.reserved_bytes(), budget.reserved_objects()), (0, 0));
        }
    }

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

// Real short std::io::Read chunks; a typed interruption is never ErrorKind::Interrupted,
// which means retry to standard I/O adapters.
struct ShortReader<'a> {
    bytes: &'a [u8],
    consumed: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    stop_at: Option<usize>,
}
impl std::io::Read for ShortReader<'_> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let position = self.consumed.load(std::sync::atomic::Ordering::SeqCst);
        if self.stop_at.is_some_and(|limit| position >= limit) {
            return Err(std::io::Error::other(
                zencodec_media::session::SessionError::<std::io::Error>::Cancelled(
                    enough::StopReason::TimedOut,
                ),
            ));
        }
        let n = out
            .len()
            .min(7)
            .min(self.bytes.len().saturating_sub(position));
        out[..n].copy_from_slice(&self.bytes[position..position + n]);
        self.consumed
            .fetch_add(n, std::sync::atomic::Ordering::SeqCst);
        Ok(n)
    }
}

#[test]
fn reserved_snapshot_refuses_before_reads_and_polls_between_interrupted_reads() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    use zencodec::how_far::StopOnly;
    let consumed = Arc::new(AtomicUsize::new(0));
    let mut reader = ShortReader {
        bytes: &[1, 2],
        consumed: consumed.clone(),
        stop_at: None,
    };
    assert!(snapshot_reserved(&mut reader, &StorageBudget::new(1, 1), 2, None).is_err());
    assert!(snapshot_reserved(&mut reader, &StorageBudget::new(2, 0), 2, None).is_err());
    assert_eq!(consumed.load(Ordering::SeqCst), 0);
    let budget = StorageBudget::new(0, 1);
    let empty = snapshot_reserved(&mut std::io::empty(), &budget, 0, None).unwrap();
    assert!(empty.is_empty());
    assert_eq!(budget.reserved_objects(), 1);
    drop(empty);
    assert_eq!(budget.reserved_objects(), 0);

    struct Interrupts {
        flag: Arc<AtomicBool>,
        calls: usize,
    }
    impl std::io::Read for Interrupts {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            self.calls += 1;
            self.flag.store(true, Ordering::SeqCst);
            Err(std::io::ErrorKind::Interrupted.into())
        }
    }
    let flag = Arc::new(AtomicBool::new(false));
    let pulse = StopOnly::new(StopFlag(flag.clone()));
    let mut reader = Interrupts { flag, calls: 0 };
    let budget = StorageBudget::new(256, 1);
    let error = snapshot_reserved(&mut reader, &budget, 256, Some(&pulse)).unwrap_err();
    assert!(matches!(
        error.downcast_ref::<zencodec_media::session::SessionError<std::io::Error>>(),
        Some(zencodec_media::session::SessionError::Cancelled(
            enough::StopReason::TimedOut
        ))
    ));
    assert_eq!(reader.calls, 1, "interrupted reads spun past cancellation");
    assert_eq!((budget.reserved_bytes(), budget.reserved_objects()), (0, 0));
}

#[test]
fn retained_policy_admits_snapshot_factory_and_guards_native_frames() {
    use std::{
        cell::Cell,
        convert::Infallible,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };
    use zencodec::{
        At,
        operation::{
            BoundPolicy, Decision, MetadataTransfer, PolicyRequirement, PolicySet, Question,
            RequirementEvidence, UnmetRequirements,
        },
    };
    use zencodec_media::policy::PolicyDecoder;
    let settings = fixtures::settings(8, Subsampling::Yuv444, true);
    let frames: Vec<_> = (0..3).map(|i| fixtures::frame(&settings, i)).collect();
    let (config, packets) = fixtures::encode(settings, &frames);
    let bytes: Vec<_> = packets.iter().flat_map(|p| p.bytes()).copied().collect();
    for case in 0..4 {
        let requirements = match case {
            1 => PolicySet::new().with_workspace_limit(0),
            2 => PolicySet::new().with_workspace_limit(u64::MAX),
            3 => PolicySet::new().require_metadata(MetadataTransfer::DiscardDescriptive),
            _ => PolicySet::new(),
        };
        let decisions = Cell::new(0);
        let factories = Cell::new(0);
        let consumed = Arc::new(AtomicUsize::new(0));
        let budget = StorageBudget::new(bytes.len(), 1);
        // This caller owns the snapshot separately from the native decoder.
        let mut saved = None;
        let policy = BoundPolicy::new(
            requirements,
            |question: Question<'_>| -> Result<Decision, At<Infallible>> {
                let Question::Frame { choices, .. } = question else {
                    panic!("unexpected question")
                };
                decisions.set(decisions.get() + 1);
                Ok(Decision::choose(choices.first().unwrap()))
            },
        );
        let result = PolicyDecoder::try_new(
            || -> Result<_, std::io::Error> {
                factories.set(factories.get() + 1);
                let mut reader = ShortReader {
                    bytes: &bytes,
                    consumed: consumed.clone(),
                    stop_at: None,
                };
                saved = Some(
                    snapshot_reserved(&mut reader, &budget, bytes.len(), None)
                        .map_err(std::io::Error::other)?,
                );
                let mut settings = rav1d_safe::Settings::default();
                settings.threads = 1;
                zenavif::media::Av1VideoDecoder::with_settings(settings)
                    .map_err(std::io::Error::other)
            },
            policy,
            None,
        );
        if case == 1 || case == 2 {
            let error = match result {
                Err(error) => error,
                Ok(_) => panic!("unqualified workspace admitted"),
            };
            let unmet = zencodec::find_cause::<UnmetRequirements>(&error).unwrap();
            assert_eq!(
                unmet.iter().collect::<Vec<_>>(),
                vec![(PolicyRequirement::Workspace, RequirementEvidence::Unknown)]
            );
            assert_eq!(factories.get(), 0);
            assert_eq!(decisions.get(), 0);
            assert_eq!(consumed.load(Ordering::SeqCst), 0);
            assert!(saved.is_none());
            assert_eq!(budget.reserved_bytes(), 0);
            continue;
        }
        let mut decoder = result.unwrap();
        let saved = saved.unwrap();
        assert_eq!(factories.get(), 1);
        assert_eq!(consumed.load(Ordering::SeqCst), bytes.len());
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
        let output = fixtures::decode(&mut decoder, &config, &replay, None);
        if case == 3 {
            assert!(
                output.is_err(),
                "frame publication cannot prove metadata removal"
            );
            assert!(decoder.is_failed());
            assert_eq!(
                decisions.get(),
                0,
                "requirements must filter before callback"
            );
        } else {
            let output = output.unwrap();
            assert_eq!(output.len(), frames.len());
            assert_eq!(decisions.get(), frames.len());
            for (actual, expected) in output.iter().zip(&frames) {
                assert_eq!(fixtures::samples(actual), fixtures::samples(expected));
            }
        }
        drop(decoder);
        drop(replay);
        assert_eq!(
            budget.reserved_bytes(),
            bytes.len(),
            "caller still owns snapshot"
        );
        drop(saved);
        assert_eq!(budget.reserved_bytes(), 0);
    }
}
