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
    if arm == "fixture" {
        let size = args.next().expect("size").parse().unwrap();
        let path = args.next().expect("path");
        assert!(args.next().is_none());
        let mut out = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .unwrap();
        std::io::Write::write_all(&mut out, &fixture(size, size)).unwrap();
        return;
    }
    let operations: usize = args.next().expect("operations").parse().unwrap();
    let size_or_file = args.next().unwrap_or_else(|| "256".into());
    let strip = args.next().map(|v| v.parse().unwrap()).unwrap_or(4);
    let quota = args.next().map(|v| v.parse().unwrap()).unwrap_or(64);
    assert!(args.next().is_none(), "arm operations [size strip quota]");
    let file: Arc<[u8]> = match size_or_file.strip_prefix('@') {
        Some(path) => std::fs::read(path).unwrap().into(),
        None => {
            let size = size_or_file.parse().unwrap();
            fixture(size, size)
        }
    };
    let size = u32::from_le_bytes(file[4..8].try_into().unwrap());
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .build()
        .unwrap();
    if arm == "qualify" {
        reimagined_driver_ablation::qualify_file(&file, size, size, strip, &runtime);
        println!(
            "qualified SYN1 {size}x{size}, strip {strip}, {} bytes",
            file.len()
        );
        return;
    }
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
