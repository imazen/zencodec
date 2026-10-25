//! Measure the candidate's storage bookkeeping separately from codec work.
#![forbid(unsafe_code)]
#[path = "../src/admission.rs"]
mod admission;
use admission::*;
use std::hint::black_box;
use zenbench::prelude::*;
use zencodec::decode::ByteRange;

fn admission(suite: &mut Suite) {
    for size in [1024, 65536] {
        suite.group(format!("allocation-and-owner/{size}"), |g| {
            g.config()
                .max_time(std::time::Duration::from_secs(1))
                .max_rounds(20);
            let storage = Storage::new(Limits {
                bytes: size,
                descriptors: 1,
                range_bytes: size,
                merge_gap: 0,
            });
            let payload = vec![7; size];
            let raw = payload.clone();
            g.bench("vec-arc", move |b| {
                b.iter(|| {
                    let value = std::sync::Arc::new(raw.clone());
                    black_box(value.clone());
                    black_box(value);
                })
            });
            g.bench("reserved-lease", move |b| {
                b.iter(|| {
                    let mut transfer =
                        Transfer::new(&storage, Identity(1), ByteRange::new(0, size as u64))
                            .unwrap();
                    transfer.append(&payload).unwrap();
                    let lease = transfer.publish(Identity(1)).unwrap();
                    black_box(lease.clone());
                    black_box(lease);
                })
            });
            let budget = zencodec_io::storage::StorageBudget::new(size, 1);
            let payload = vec![7; size];
            g.bench("production-budget", move |b| {
                b.iter(|| {
                    let mut owned = budget.bytes(size).unwrap();
                    owned.as_mut_slice().copy_from_slice(&payload);
                    let shared = owned.freeze();
                    black_box(shared.clone());
                    black_box(shared);
                })
            });
        });
    }
}
zenbench::main!(admission);
