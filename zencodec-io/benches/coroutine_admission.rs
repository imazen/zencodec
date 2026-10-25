//! End-to-end authored memcpy operation: identical control protocol and output,
//! different input/index and row storage. Includes construction and cleanup.
#![forbid(unsafe_code)]
mod fixture;
use fixture::*;
use zenbench::prelude::*;
use zencodec::operation::PolicySet;
use zencodec_io::{CollectSink, PolicyDecoder, drive_from_slice};

#[global_allocator]
static ALLOC: zenbench::AllocProfiler = zenbench::AllocProfiler::system();

fn run(file: &[u8], width: u32, strip: u32, window: usize, admitted: bool) -> CollectSink {
    let cap = (width * 4 * strip) as usize;
    let native = coroutine_controlled(strip, cap, admitted);
    let mut decoder = PolicyDecoder::new(native, PolicySet::new());
    let mut sink = CollectSink::new();
    drive_from_slice(&mut decoder, file, &mut sink, None, window).unwrap();
    sink
}

fn coroutine_admission(suite: &mut Suite) {
    for (width, height, strip) in [(1024, 1024, 16), (1024, 1024, 1), (256, 256, 4)] {
        let input = synthetic_file(width, height);
        for window in [1024, 65536] {
            for admitted in [false, true] {
                let output = run(&input, width, strip, window, admitted);
                assert_eq!(output.bytes(), &input[HEADER..]);
            }
            suite.group(
                format!("{width}x{height}/{strip}-rows/{window}-window"),
                |g| {
                    g.config()
                        .max_time(std::time::Duration::from_secs(1))
                        .max_rounds(20);
                    for admitted in [false, true] {
                        let file = input.clone();
                        g.bench(if admitted { "admitted" } else { "legacy" }, move |b| {
                            b.iter(|| run(&file, width, strip, window, admitted))
                        });
                    }
                },
            );
        }
    }
}
zenbench::main!(coroutine_admission);
