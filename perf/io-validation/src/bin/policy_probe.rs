//! Run without a benchmark harness so heaptrack can isolate engine allocations.
#![forbid(unsafe_code)]
#[path = "../policy/mod.rs"]
mod policy;
#[path = "../policy_benchmark.rs"]
mod workload;
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let name = args.get(1).expect("mode");
    let count: usize = args.get(2).expect("iterations").parse().unwrap();
    let mode = workload::Mode::ALL
        .into_iter()
        .find(|(_, n)| n == name)
        .map(|(m, _)| m)
        .expect("known mode");
    for _ in 0..count {
        std::hint::black_box(workload::operation(mode));
    }
}
