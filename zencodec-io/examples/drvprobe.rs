#[path = "../benches/fixture.rs"]
mod fixture;
use fixture::*;
fn main() {
    let which = std::env::args().nth(1).unwrap_or_default();
    let n: usize = std::env::args()
        .nth(2)
        .map_or(20000, |s| s.parse().unwrap());
    let file = synthetic_file(256, 256);
    #[cfg(feature = "tokio")]
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let mut total = 0usize;
    let t = std::time::Instant::now();
    for _ in 0..n {
        let mut sink = zencodec_io::CollectSink::new();
        match which.as_str() {
            "direct" => direct_decode(&file, 4, &mut sink),
            "coro" => {
                let mut d = coroutine_sequential(4);
                zencodec_io::drive_from_slice(&mut d, &file, &mut sink, None, 1 << 20).unwrap();
            }
            "pump" => {
                let mut d = HandWritten::new(4);
                zencodec_io::pump_read_to_sink(
                    std::io::Cursor::new(&file),
                    &mut d,
                    &mut sink,
                    None,
                    64 << 10,
                    16 << 20,
                )
                .unwrap();
            }
            "at" => {
                let mut d = coroutine_ranges(4);
                let mut src = zencodec::source::SliceReadAt::new(&file);
                zencodec_io::drive_from_read_at(
                    &mut d,
                    &mut src,
                    &mut sink,
                    None,
                    zencodec_io::DriveOptions::default(),
                )
                .unwrap();
            }
            #[cfg(feature = "tokio")]
            "tokio" => {
                let f: std::sync::Arc<[u8]> = file.clone().into();
                sink = rt
                    .block_on(async move {
                        let mut src = zencodec_io::tokio_io::BytesSource(f);
                        zencodec_io::tokio_io::decode_async(
                            HandWritten::new(4),
                            sink,
                            &mut src,
                            zencodec_io::tokio_io::Inline,
                            zencodec_io::DriveOptions::default(),
                            None,
                        )
                        .await
                        .unwrap()
                    })
                    .1;
            }
            _ => {
                let mut d = HandWritten::new(4);
                zencodec_io::drive_from_slice(&mut d, &file, &mut sink, None, 1 << 20).unwrap();
            }
        }
        total += std::hint::black_box(&sink) as *const _ as usize & 1;
    }
    eprintln!(
        "{which}: {:.3} us/iter ({total})",
        t.elapsed().as_secs_f64() * 1e6 / n as f64
    );
}
