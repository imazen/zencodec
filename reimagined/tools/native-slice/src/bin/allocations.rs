//! Timer-free native operations for external heaptrack/perf capture.
use reimagined_native_slice::{Kind, execute, fixture, qualify};
use std::{hint::black_box, sync::Arc};
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("fixture") {
        assert_eq!(args.len(), 4, "fixture codec size path");
        let file = fixture(Kind::parse(&args[1]), args[2].parse().unwrap());
        let mut out = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&args[3])
            .unwrap();
        std::io::Write::write_all(&mut out, &file).unwrap();
        return;
    }
    assert_eq!(args.len(), 4, "codec arm size-or-@file operations");
    let kind = Kind::parse(&args[0]);
    let file: Arc<[u8]> = match args[2].strip_prefix('@') {
        Some(path) => std::fs::read(path).unwrap().into(),
        None => fixture(kind, args[2].parse().unwrap()),
    };
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .build()
        .unwrap();
    if args[1] == "qualify" {
        qualify(kind, &file, &rt);
        println!("qualified {} {} bytes", kind.name(), file.len());
        return;
    }
    black_box(execute(&args[1], kind, &file, &rt));
    for _ in 0..args[3].parse::<usize>().unwrap() {
        black_box(execute(&args[1], kind, &file, &rt));
    }
}
