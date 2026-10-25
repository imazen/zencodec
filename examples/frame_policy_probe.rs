#![forbid(unsafe_code)]
#[path = "../perf/frame-policy-cost.rs"]
mod workload;

fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert_eq!(args.len(), 3, "MODE COUNT (record, resolve, combined)");
    assert!(["record", "resolve", "combined"].contains(&args[1].as_str()));
    let count: usize = args[2].parse().unwrap();
    assert!(count <= 10_000_000);
    workload::Workload::new().verify(&args[1], count);
}
