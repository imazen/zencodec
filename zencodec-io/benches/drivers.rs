//! Framework overhead per strip: the same memcpy-per-strip decode through
//! every driver shape, against direct sink writes.
//! `cargo bench -p zencodec-io --features tokio --bench drivers`

mod fixture;
use fixture::*;
use zenbench::prelude::*;

// Per-iteration allocation counts in the report. zenbench counts per
// thread: every arm runs on the bench thread except the spawn_blocking one,
// whose batches allocate on pool threads that are not counted.
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
            g.config()
                .max_time(std::time::Duration::from_secs(3))
                .max_rounds(40);
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
                        16 << 20,
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
                g.bench("hand-written via tokio decode_async, inline", move |b| {
                    let f = f.clone();
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .build()
                        .unwrap();
                    b.iter(move || {
                        rt.block_on(async {
                            let mut src = zencodec_io::tokio_io::BytesSource(f.clone());
                            zencodec_io::tokio_io::decode_async(
                                HandWritten::new(strip_rows),
                                zencodec_io::CollectSink::new(),
                                &mut src,
                                zencodec_io::tokio_io::Inline,
                                zencodec_io::DriveOptions::default(),
                                None,
                            )
                            .await
                            .unwrap()
                        })
                    })
                });
                let f: std::sync::Arc<[u8]> = file.clone().into();
                g.bench(
                    "hand-written via tokio decode_async, spawn_blocking",
                    move |b| {
                        let f = f.clone();
                        let rt = tokio::runtime::Builder::new_multi_thread()
                            .worker_threads(2)
                            .build()
                            .unwrap();
                        b.iter(move || {
                            rt.block_on(async {
                                let mut src = zencodec_io::tokio_io::BytesSource(f.clone());
                                zencodec_io::tokio_io::decode_async(
                                    HandWritten::new(strip_rows),
                                    zencodec_io::CollectSink::new(),
                                    &mut src,
                                    |job: zencodec_io::tokio_io::Job| {
                                        tokio::task::spawn_blocking(job);
                                    },
                                    zencodec_io::DriveOptions::default(),
                                    None,
                                )
                                .await
                                .unwrap()
                            })
                        })
                    },
                );
            }
        });
    }
}

zenbench::main!(bench_drivers);
