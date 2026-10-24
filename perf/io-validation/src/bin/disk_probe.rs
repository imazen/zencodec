//! Fixed-work syscall-count companion to the zenbench comparison.
#![forbid(unsafe_code)]
#[path = "../disk_support.rs"]
mod disk_support;
use disk_support::*;
use zencodec::source::{ReadAt, SourceError};
fn main() {
    let args: Vec<String> = std::env::args().collect();
    assert_eq!(args.len(), 4, "disk_probe seek|positional repetitions file");
    let repetitions: usize = args[2].parse().unwrap();
    let path = std::path::Path::new(&args[3]);
    fixture(path);
    let mut source: Box<dyn ReadAt<Error = SourceError>> = match args[1].as_str() {
        "seek" => Box::new(zencodec_io::FileReadAt::open(path).unwrap()),
        "positional" => Box::new(Positional::new(std::fs::File::open(path).unwrap()).unwrap()),
        _ => panic!("unknown mode"),
    };
    let offsets = offsets(4096, true);
    verify(&mut *source, 4096, &offsets);
    let mut out = [0u8; 4096];
    for _ in 0..repetitions {
        for &at in &offsets {
            source.read_exact_at(at, &mut out).unwrap();
            std::hint::black_box(&out);
        }
    }
    println!(
        "mode={} reads={} bytes_per_read=4096 parity=true",
        args[1],
        (repetitions + 1) * 128
    );
}
