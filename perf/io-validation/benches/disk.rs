//! Warm page-cache comparison. Opening and fixture generation are not timed.
#![forbid(unsafe_code)]
#[path = "../src/disk_support.rs"]
mod disk_support;
use disk_support::*;
use zenbench::prelude::*;
use zencodec::source::ReadAt;
fn disk(suite: &mut Suite) {
    let path = std::path::PathBuf::from(
        std::env::var("IO_DISK_FILE").expect("set IO_DISK_FILE to a disk-backed scratch file"),
    );
    fixture(&path);
    for size in [256, 4096, 65536] {
        for random in [false, true] {
            let shape = if random { "random" } else { "sequential" };
            suite.group(format!("{shape}/{size}-byte/128-reads"), |g| {
                g.config()
                    .max_time(std::time::Duration::from_secs(2))
                    .max_rounds(30);
                for positional in [false, true] {
                    let mut source: Box<dyn ReadAt<Error = zencodec::source::SourceError> + Send> =
                        if positional {
                            Box::new(zencodec_io::FileReadAt::open(&path).unwrap())
                        } else {
                            Box::new(SeekRead::open(&path).unwrap())
                        };
                    let offsets = offsets(size, random);
                    verify(&mut *source, size, &offsets);
                    let label = if positional {
                        "file-positional"
                    } else {
                        "seek-read"
                    };
                    g.bench(label, move |b| {
                        let mut out = vec![0u8; size];
                        b.iter(|| {
                            for &at in &offsets {
                                source.read_exact_at(at, &mut out).unwrap();
                                std::hint::black_box(&out);
                            }
                        })
                    });
                }
            });
        }
    }
}
zenbench::main!(disk);
