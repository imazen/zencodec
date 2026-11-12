//! Heaptrack probe: operation counts, without an internal timer.
#![forbid(unsafe_code)]
use reimagined_driver_ablation::{execute, fixture, older};
use std::{hint::black_box, sync::Arc};

#[inline(never)]
fn operation(
    arm: &str,
    file: &Arc<[u8]>,
    strip: u32,
    quota: usize,
    runtime: &tokio::runtime::Runtime,
) -> usize {
    let sink = execute(arm, file, strip, 0, quota, runtime);
    let bytes = black_box(sink.bytes);
    black_box(bytes.len() ^ bytes[0] as usize)
}
fn main() {
    let mut args = std::env::args().skip(1);
    let arm = args.next().expect("arm");
    let operations: usize = args.next().expect("operations").parse().unwrap();
    let size = args.next().map(|v| v.parse().unwrap()).unwrap_or(256);
    let strip = args.next().map(|v| v.parse().unwrap()).unwrap_or(4);
    let quota = args.next().map(|v| v.parse().unwrap()).unwrap_or(64);
    assert!(args.next().is_none(), "arm operations [size strip quota]");
    let file = fixture(size, size);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    // Keep the same warm operation in both count controls, including pool startup.
    let warm = execute(&arm, &file, strip, 0, quota, &runtime);
    assert_eq!(warm.active(), file[older::HEADER..]);
    assert_eq!(warm.rows, size);
    assert!(warm.ended);
    black_box(warm);
    let mut result = 0;
    for _ in 0..operations {
        result ^= operation(&arm, &file, strip, quota, &runtime);
    }
    black_box(result);
}
