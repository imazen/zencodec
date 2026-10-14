//! Framework overhead per strip: the same memcpy-per-strip decode through
//! every driver shape, against direct sink writes.
//! `cargo bench -p zencodec-io --features tokio --bench drivers`

mod fixture;
use fixture::*;
use zenbench::prelude::*;

// Per-iteration allocation counts in the report. Thread-local: the tokio
// arm's decode thread is not counted here; `examples/alloc_audit.rs`
// counts across threads.
#[global_allocator]
static ALLOC: zenbench::AllocProfiler = zenbench::AllocProfiler::system();

fn bench_drivers(suite: &mut Suite) {
    for (label, w, h, strip_rows) in [
        ("1024x1024/16-row strips", 1024u32, 1024u32, 16u32),
        ("1024x1024/1-row strips", 1024, 1024, 1),
        ("256x256/4-row strips", 256, 256, 4),
    ] {
        let file = synthetic_file(w, h);
        let strips = (h / strip_rows) as u64;
        suite.group(label, |g| {
            g.throughput(Throughput::Elements(strips));
            let f = file.clone();
            g.bench("direct sink writes", move |b| {
                let f = f.clone();
                b.iter(move || {
                    let mut sink = zencodec_io::CollectSink::new();
                    direct_decode(&f, strip_rows, &mut sink);
                    sink
                })
            });
            let f = file.clone();
            g.bench("hand-written via drive_from_slice", move |b| {
                let f = f.clone();
                b.iter(move || {
                    let mut dec = HandWritten::new(strip_rows);
                    let mut sink = zencodec_io::CollectSink::new();
                    zencodec_io::drive_from_slice(&mut dec, &f, &mut sink, None, 1 << 20).unwrap();
                    sink
                })
            });
            let f = file.clone();
            g.bench("hand-written via pump_read_to_sink 64K", move |b| {
                let f = f.clone();
                b.iter(move || {
                    let mut dec = HandWritten::new(strip_rows);
                    let mut sink = zencodec_io::CollectSink::new();
                    zencodec_io::pump_read_to_sink(
                        std::io::Cursor::new(&f),
                        &mut dec,
                        &mut sink,
                        None,
                        64 << 10,
                    )
                    .unwrap();
                    sink
                })
            });
            let f = file.clone();
            g.bench("coroutine sequential via drive_from_slice", move |b| {
                let f = f.clone();
                b.iter(move || {
                    let mut dec = coroutine_sequential(strip_rows);
                    let mut sink = zencodec_io::CollectSink::new();
                    zencodec_io::drive_from_slice(&mut dec, &f, &mut sink, None, 1 << 20).unwrap();
                    sink
                })
            });
            let f = file.clone();
            g.bench("coroutine ranges via drive_from_read_at", move |b| {
                let f = f.clone();
                b.iter(move || {
                    let mut dec = coroutine_ranges(strip_rows);
                    let mut sink = zencodec_io::CollectSink::new();
                    let mut src = zencodec::source::SliceReadAt::new(&f);
                    zencodec_io::drive_from_read_at(
                        &mut dec,
                        &mut src,
                        &mut sink,
                        None,
                        zencodec_io::DriveOptions::default(),
                    )
                    .unwrap();
                    sink
                })
            });
            #[cfg(feature = "tokio")]
            {
                let f: std::sync::Arc<[u8]> = file.clone().into();
                g.bench("hand-written via tokio decode_on_thread", move |b| {
                    let f = f.clone();
                    b.iter(move || {
                        let rt = tokio::runtime::Builder::new_current_thread()
                            .build()
                            .unwrap();
                        rt.block_on(async {
                            zencodec_io::thread::decode_on_thread(
                                HandWritten::new(strip_rows),
                                zencodec_io::CollectSink::new(),
                                None,
                                zencodec_io::thread::ThreadOptions::default(),
                                zencodec_io::thread::fetch_from_bytes(f.clone(), 1 << 20),
                            )
                            .await
                            .unwrap()
                        })
                    })
                });
            }
        });
    }
}

zenbench::main!(bench_drivers);
