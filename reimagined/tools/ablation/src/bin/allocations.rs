//! Heaptrack probe: operation counts, without an internal timer.
#![forbid(unsafe_code)]
use reimagined_driver_ablation::{execute, fixture};
use std::{hint::black_box, sync::Arc};

#[inline(never)]
fn operation(arm: &str, file: &Arc<[u8]>, runtime: &tokio::runtime::Runtime) -> usize {
    let sink = execute(arm, file, 4, 0, 64, runtime);
    let bytes = black_box(sink.bytes);
    black_box(bytes.len() ^ bytes[0] as usize)
}
fn main() {
    let mut args = std::env::args().skip(1);
    let arm = args.next().expect("arm");
    let operations: usize = args.next().expect("operations").parse().unwrap();
    assert!(args.next().is_none());
    let file = fixture(256, 256);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    // Keep the same warm operation in both count controls, including pool startup.
    black_box(operation(&arm, &file, &runtime));
    let mut result = 0;
    for _ in 0..operations {
        result ^= operation(&arm, &file, &runtime);
    }
    black_box(result);
}
