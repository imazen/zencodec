//! Per-file page-cache eviction before every iteration; no global drop_caches.
//! Setup, eviction and kernel read-byte verification are outside timed work.
#![forbid(unsafe_code)]
#[path = "../src/disk_support.rs"]
mod disk_support;
use disk_support::*;
use std::path::PathBuf;
use zenbench::prelude::*;
use zencodec::source::ReadAt;
use zencodec_io::StdSourceError;

fn read_bytes() -> u64 {
    std::fs::read_to_string("/proc/self/io")
        .unwrap()
        .lines()
        .find_map(|line| line.strip_prefix("read_bytes: "))
        .unwrap()
        .parse()
        .unwrap()
}

struct Sample {
    source: Box<dyn ReadAt<Error = StdSourceError> + Send>,
    output: Vec<u8>,
    offsets: Vec<u64>,
    before: u64,
}
impl Drop for Sample {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            let delta = read_bytes() - self.before;
            assert!(
                delta >= (self.output.len() * self.offsets.len()) as u64,
                "eviction did not produce the expected kernel-read charge: {delta}"
            );
        }
    }
}

fn cold(suite: &mut Suite) {
    let path = PathBuf::from(std::env::var("IO_DISK_FILE").expect("IO_DISK_FILE"));
    fixture(&path);
    std::fs::File::open(&path).unwrap().sync_all().unwrap();
    for size in [4096, 65536] {
        for random in [false, true] {
            let shape = if random { "dispersed" } else { "sequential" };
            suite.group(format!("evicted/{shape}/{size}/128-reads"), |g| {
                g.config()
                    .max_rounds(9)
                    .min_rounds(9)
                    .auto_rounds(false)
                    .warmup_time(std::time::Duration::ZERO)
                    .max_time(std::time::Duration::from_secs(2));
                g.config().min_iterations = 1;
                g.config().max_iterations = 1;
                for positional in [false, true] {
                    let path = path.clone();
                    let mut check = zencodec_io::FileReadAt::open(&path).unwrap();
                    verify(&mut check, size, &offsets(size, random));
                    g.bench(
                        if positional {
                            "file-positional"
                        } else {
                            "seek-read"
                        },
                        move |b| {
                            let path = path.clone();
                            b.with_input(move || {
                                let file = std::fs::File::open(&path).unwrap();
                                rustix::fs::fadvise(&file, 0, None, rustix::fs::Advice::DontNeed)
                                    .unwrap();
                                let source: Box<dyn ReadAt<Error = StdSourceError> + Send> =
                                    if positional {
                                        Box::new(zencodec_io::FileReadAt::new(file).unwrap())
                                    } else {
                                        Box::new(NativeErrors(SeekRead::open(&path).unwrap()))
                                    };
                                let output = vec![0; size];
                                let offsets = offsets(size, random);
                                Sample {
                                    source,
                                    output,
                                    offsets,
                                    before: read_bytes(),
                                }
                            })
                            .run(|mut sample| {
                                for &offset in &sample.offsets {
                                    sample
                                        .source
                                        .read_exact_at(offset, &mut sample.output)
                                        .unwrap();
                                    std::hint::black_box(&sample.output);
                                }
                                sample // Verification/drop run after the timing endpoint.
                            });
                        },
                    );
                }
            });
        }
    }
}
zenbench::main!(cold);
