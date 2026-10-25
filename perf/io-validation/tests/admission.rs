//! Candidate contracts exercised with delayed transport and real codec bytes.
#![forbid(unsafe_code)]
#[path = "../src/admission.rs"]
mod admission;
use admission::*;
use zencodec::decode::ByteRange;

fn limits(bytes: usize) -> Limits {
    Limits {
        bytes,
        descriptors: 16,
        range_bytes: 64,
        merge_gap: 32,
    }
}

#[test]
fn checked_plans_bound_coalescing_splitting_and_adversarial_arithmetic() {
    let input = [ByteRange::new(0, 32), ByteRange::new(64, 32)];
    let mut l = limits(64);
    l.range_bytes = 128;
    assert_eq!(
        plan_all(&input, 96, l).unwrap(),
        input,
        "no budget for the gap"
    );
    l.bytes = 96;
    assert_eq!(plan_all(&input, 96, l).unwrap(), [ByteRange::new(0, 96)]);
    l.range_bytes = 16;
    let split = plan_all(&[ByteRange::new(4, 48), ByteRange::new(16, 48)], 64, l).unwrap();
    assert_eq!(
        split.iter().map(|r| r.length).sum::<u64>(),
        60,
        "overlaps are transferred once"
    );
    assert!(split.iter().all(|r| r.length <= 16));
    assert_eq!(
        plan_all(&[ByteRange::new(u64::MAX, 1)], u64::MAX, l),
        Err(Refusal::Bounds)
    );
    assert_eq!(
        plan_all(&[ByteRange::new(0, 0)], 64, l),
        Err(Refusal::Bounds)
    );
    assert_eq!(
        plan_all(&[ByteRange::new(0, u64::MAX)], u64::MAX, l),
        Err(Refusal::Bytes)
    );
    l.descriptors = 2;
    assert_eq!(
        plan_all(&[ByteRange::new(0, 64)], 64, l),
        Err(Refusal::Descriptors)
    );
}

fn ready(storage: &Storage, id: Identity, value: u8, size: usize) -> Lease {
    let mut transfer = Transfer::new(storage, id, ByteRange::new(0, size as u64)).unwrap();
    transfer.append(&vec![value; size]).unwrap();
    transfer.publish(id).unwrap()
}

#[test]
fn storage_charge_survives_transfer_cache_decode_and_shared_clones() {
    let storage = Storage::new(limits(8));
    let lease = ready(&storage, Identity(1), 7, 8);
    let decoder = lease.clone();
    assert_eq!(lease.bytes().as_ptr(), decoder.bytes().as_ptr());
    assert_eq!(storage.occupancy().bytes, 8);
    assert!(matches!(storage.reserve(1), Err(Refusal::Bytes)));
    drop(lease);
    assert_eq!(storage.occupancy().bytes, 8);
    drop(decoder);
    assert_eq!(storage.occupancy().bytes, 0);
    assert_eq!(storage.occupancy().objects, 0);
}

#[test]
fn incomplete_wrong_identity_and_oversized_responses_never_publish() {
    let storage = Storage::new(limits(8));
    let partial = Transfer::new(&storage, Identity(1), ByteRange::new(0, 8)).unwrap();
    assert!(matches!(
        partial.publish(Identity(1)),
        Err(Refusal::Incomplete)
    ));
    assert_eq!(storage.occupancy().bytes, 0);
    let mut wrong = Transfer::new(&storage, Identity(1), ByteRange::new(0, 8)).unwrap();
    wrong.append(&[1; 8]).unwrap();
    assert!(matches!(wrong.publish(Identity(2)), Err(Refusal::Identity)));
    let mut too_big = Transfer::new(&storage, Identity(1), ByteRange::new(0, 8)).unwrap();
    assert_eq!(too_big.append(&[1; 9]), Err(Refusal::Bytes));
    drop(too_big);
    assert_eq!(storage.occupancy().bytes, 0);
}

#[test]
fn extra_body_bytes_poison_even_an_already_complete_transfer() {
    let storage = Storage::new(limits(8));
    let mut transfer = Transfer::new(&storage, Identity(1), ByteRange::new(0, 8)).unwrap();
    transfer.append(&[1; 8]).unwrap();
    assert_eq!(transfer.append(&[2]), Err(Refusal::Bytes));
    assert!(matches!(transfer.publish(Identity(1)), Err(Refusal::Bytes)));
    assert_eq!(storage.occupancy().bytes, 0);
}

#[test]
fn later_completions_cannot_take_the_missing_heads_queue_slot() {
    let storage = Storage::new(limits(8));
    let mut ordered = Ordered::new(3);
    for ordinal in [2, 1] {
        ordered
            .insert(0, ordinal, ready(&storage, Identity(1), ordinal as u8, 1))
            .unwrap();
    }
    assert_eq!(
        ordered.insert(0, 3, ready(&storage, Identity(1), 3, 1)),
        Err(Refusal::Descriptors)
    );
    ordered
        .insert(0, 0, ready(&storage, Identity(1), 0, 1))
        .unwrap();
    for expected in 0..3 {
        assert_eq!(ordered.pop().unwrap().bytes(), [expected]);
    }
    assert_eq!(storage.occupancy().bytes, 0);
}

#[test]
fn small_range_plans_preserve_an_independent_byte_coverage_oracle() {
    // Enumerate overlapping, reversed and disjoint dependencies. The oracle
    // marks individual source bytes; it does not reproduce the merge algorithm.
    for a in 0..8 {
        for b in 0..8 {
            for length in 1..=8 {
                let input = [ByteRange::new(b, length), ByteRange::new(a, length)];
                let mut needed = [false; 16];
                for r in input {
                    needed[r.offset as usize..r.end() as usize].fill(true);
                }
                for cap in [0, 4, 8, 16] {
                    let mut l = limits(cap);
                    l.range_bytes = 3;
                    if let Ok(plan) = plan_all(&input, 16, l) {
                        let mut fetched = [false; 16];
                        let mut total = 0;
                        for r in &plan {
                            assert!(r.length > 0 && r.length <= 3);
                            for present in &mut fetched[r.offset as usize..r.end() as usize] {
                                assert!(!*present, "transferred a byte twice");
                                *present = true;
                            }
                            total += r.length as usize;
                        }
                        assert!(total <= cap && plan.len() <= l.descriptors);
                        assert!(needed.iter().zip(fetched).all(|(need, have)| !need || have));
                    } else {
                        assert!(needed.iter().filter(|v| **v).count() > cap);
                    }
                }
            }
        }
    }
}

#[test]
fn video_completion_order_is_separate_from_decode_order_and_seek_generation() {
    let storage = Storage::new(limits(4));
    // Admit the head before later packets, even when it completes last.
    let mut head = Transfer::new(&storage, Identity(1), ByteRange::new(0, 1)).unwrap();
    let mut ordered = Ordered::new(3);
    ordered
        .insert(0, 2, ready(&storage, Identity(1), 2, 1))
        .unwrap();
    ordered
        .insert(0, 1, ready(&storage, Identity(1), 1, 1))
        .unwrap();
    assert!(ordered.pop().is_none());
    assert_eq!(storage.occupancy().bytes, 3);
    head.append(&[0]).unwrap();
    ordered
        .insert(0, 0, head.publish(Identity(1)).unwrap())
        .unwrap();
    for expected in 0..3 {
        assert_eq!(ordered.pop().unwrap().bytes(), [expected]);
    }
    assert_eq!(storage.occupancy().bytes, 0);
    ordered.reset(1);
    assert_eq!(
        ordered.insert(0, 0, ready(&storage, Identity(1), 0, 1)),
        Err(Refusal::Identity)
    );
    ordered
        .insert(1, 1, ready(&storage, Identity(1), 1, 1))
        .unwrap();
    ordered.reset(2);
    assert_eq!(storage.occupancy().bytes, 0);
}

#[tokio::test]
async fn cancellation_releases_inflight_storage_without_retiring_other_readers() {
    let storage = Storage::new(limits(8));
    let cached = ready(&storage, Identity(1), 1, 4);
    let (started, wait) = tokio::sync::oneshot::channel();
    let worker_storage = storage.clone();
    let task = tokio::spawn(async move {
        let transfer = Transfer::new(&worker_storage, Identity(1), ByteRange::new(4, 4)).unwrap();
        started.send(()).unwrap();
        std::future::pending::<()>().await;
        transfer.publish(Identity(1))
    });
    wait.await.unwrap();
    assert_eq!(storage.occupancy().bytes, 8);
    task.abort();
    assert!(matches!(task.await, Err(e) if e.is_cancelled()));
    assert_eq!(storage.occupancy().bytes, 4);
    assert_eq!(cached.bytes(), [1; 4]);
    drop(cached);
    assert_eq!(storage.occupancy().bytes, 0);
}

#[tokio::test]
async fn reserved_fragmented_qoi_bytes_decode_exactly_through_the_real_driver() {
    use zenbitmaps::{QoiDecoderConfig, QoiEncoderConfig};
    use zencodec::decode::DecoderConfig;
    use zencodec::encode::EncoderConfig;
    use zencodec_io::tokio_io::{AsyncSource, BytesSource};
    use zenpixels::{PixelDescriptor, PixelSlice};
    let (width, height, stride) = (9, 17, 48);
    let mut raw = vec![0xED; height * stride];
    let mut expected = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let pixel = [x as u8, y as u8, (x * y) as u8, 128];
            raw[y * stride + x * 4..y * stride + x * 4 + 4].copy_from_slice(&pixel);
            expected.extend_from_slice(&pixel);
        }
    }
    let encoded = QoiEncoderConfig::new()
        .encode(
            PixelSlice::new(
                &raw,
                width as u32,
                height as u32,
                stride,
                PixelDescriptor::RGBA8_SRGB,
            )
            .unwrap(),
        )
        .unwrap();
    let storage = Storage::new(limits(encoded.len()));
    let mut transfer = Transfer::new(
        &storage,
        Identity(5),
        ByteRange::new(0, encoded.len() as u64),
    )
    .unwrap();
    let mut source = BytesSource(encoded.data().into());
    let mut offset = 0;
    while offset < encoded.len() {
        let mut fragment = [0; 7];
        let n = source.read_at(offset as u64, &mut fragment).await.unwrap();
        assert!(n > 0);
        transfer.append(&fragment[..n]).unwrap();
        offset += n;
    }
    let cached = transfer.publish(Identity(5)).unwrap();
    let decoder_lease = cached.clone();
    drop(cached);
    let mut decoder = QoiDecoderConfig::new()
        .job()
        .into_incremental_decoder(&[])
        .unwrap();
    let mut sink = zencodec_io::CollectSink::new();
    zencodec_io::drive_from_slice(&mut decoder, decoder_lease.bytes(), &mut sink, None, 3).unwrap();
    assert_eq!(sink.bytes(), expected);
    assert_eq!(storage.occupancy().bytes, encoded.len());
    drop(decoder_lease);
    assert_eq!(storage.occupancy().bytes, 0);
}

#[test]
fn concurrent_consumers_cannot_over_admit_or_release_twice() {
    let storage = Storage::new(limits(64));
    std::thread::scope(|scope| {
        for _ in 0..4 {
            let storage = storage.clone();
            scope.spawn(move || {
                for _ in 0..1000 {
                    if let Ok(reservation) = storage.reserve(16) {
                        assert!(storage.occupancy().bytes <= 64);
                        drop(reservation);
                    }
                }
            });
        }
    });
    assert_eq!(storage.occupancy().bytes, 0);
    assert_eq!(storage.occupancy().objects, 0);
}
