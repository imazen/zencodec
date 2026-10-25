//! API policies through IMI's unchanged network-cost model; no WAN timing claims.
#![forbid(unsafe_code)]
#[path = "../admission.rs"]
mod admission;
#[path = "../transport_model.rs"]
#[allow(dead_code)]
mod transport;
use std::{cell::RefCell, rc::Rc};
use transport::{Config, Ledger, Transport};
use zencodec::decode::ByteRange;
use zencodec::source::{ReadAt, SliceReadAt};

fn run(policy: &str, rtt: f64, cache: u64, concurrency: usize) -> (f64, u64, u64, usize) {
    let bytes: Vec<u8> = (0..131072).map(|i| (i % 251) as u8).collect();
    let ledger = Rc::new(RefCell::new(Ledger::default()));
    let cfg = Config {
        ranges: true,
        cache_bytes: cache,
        latency_ms: rtt,
        bytes_per_ms: 12500.0,
        concurrency,
        merge_gap: if policy == "coalesced" { 4096 } else { 0 },
        max_range: 131072,
        part_size: 0,
        shared_cache: true,
    };
    let mut source = Transport::new(
        SliceReadAt::new(&bytes),
        "fixture".into(),
        cfg,
        ledger.clone(),
    )
    .unwrap();
    let requests: Vec<_> = (0..16).map(|i| ByteRange::new(i * 8192, 4096)).collect();
    let planned = if policy == "budgeted" {
        admission::plan_all(
            &requests,
            bytes.len() as u64,
            admission::Limits {
                bytes: 65536,
                descriptors: 16,
                range_bytes: 131072,
                merge_gap: 4096,
            },
        )
        .unwrap()
    } else {
        requests
    };
    // Two identical views: return to a previous viewport with the same object.
    for _ in 0..2 {
        let mut out: Vec<Vec<u8>> = planned.iter().map(|r| vec![0; r.length as usize]).collect();
        if policy == "serial" {
            for (r, buf) in planned.iter().zip(&mut out) {
                source.read_exact_at(r.offset, buf).unwrap();
            }
        } else {
            let mut batch: Vec<_> = planned
                .iter()
                .zip(&mut out)
                .map(|(r, buf)| (r.offset, buf.as_mut_slice()))
                .collect();
            source.read_many_at(&mut batch).unwrap();
        }
        for (r, buf) in planned.iter().zip(&out) {
            assert_eq!(
                buf,
                &bytes[r.offset as usize..(r.offset + r.length) as usize]
            );
        }
        assert!(source.retained() <= cache);
    }
    let l = ledger.borrow();
    (l.now_ms, l.transferred_bytes, l.cache_hit_bytes, l.rounds)
}
fn main() {
    println!(
        "policy,rtt_ms,cache_bytes,concurrency,simulated_ms,transferred_bytes,cache_hit_bytes,dependency_rounds"
    );
    for rtt in [0.0, 160.0] {
        for cache in [0, 131072] {
            for concurrency in [1, 8] {
                for policy in ["serial", "batch", "coalesced", "budgeted"] {
                    let (ms, bytes, hits, rounds) = run(policy, rtt, cache, concurrency);
                    println!(
                        "{policy},{rtt},{cache},{concurrency},{ms:.5},{bytes},{hits},{rounds}"
                    );
                }
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn batch_overlaps_rtt_without_multiplying_bandwidth() {
        let serial = run("serial", 160.0, 0, 8);
        let batch = run("batch", 160.0, 0, 8);
        assert!((serial.0 - 5130.48576).abs() < 0.00001);
        assert!((batch.0 - 650.48576).abs() < 0.00001);
        assert_eq!(serial.1, batch.1);
        assert_eq!(run("budgeted", 160.0, 0, 8), batch);
    }
    #[test]
    fn coalescing_spends_bytes_and_cache_reuses_object_identity() {
        assert_eq!(run("coalesced", 160.0, 0, 8).1, 253952);
        assert_eq!(run("coalesced", 160.0, 131072, 8).1, 126976);
        assert_eq!(run("batch", 160.0, 131072, 8).1, 65536);
        assert_eq!(run("batch", 160.0, 131072, 8).2, 65536);
    }
}
