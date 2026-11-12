//! Timer-free native operations for external heaptrack/perf capture.
use reimagined_native_slice::{Kind, execute, fixture};
use std::hint::black_box;
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    assert_eq!(args.len(), 4, "codec arm size operations");
    let kind = Kind::parse(&args[0]);
    let file = fixture(kind, args[2].parse().unwrap());
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    black_box(execute(&args[1], kind, &file, &rt));
    for _ in 0..args[3].parse::<usize>().unwrap() {
        black_box(execute(&args[1], kind, &file, &rt));
    }
}
