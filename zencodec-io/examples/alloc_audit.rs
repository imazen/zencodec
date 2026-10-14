//! Counts heap allocations per decode for each driver shape, with a
//! decoder whose own work is a memcpy per strip, so the numbers are the
//! framework's overhead. Run: `cargo run --release -p zencodec-io --features tokio --example alloc_audit`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Counting;
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(new_size, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn measure(label: &str, strips: u64, f: impl FnOnce()) {
    let a0 = ALLOCS.load(Ordering::Relaxed);
    let b0 = BYTES.load(Ordering::Relaxed);
    f();
    let a = ALLOCS.load(Ordering::Relaxed) - a0;
    let b = BYTES.load(Ordering::Relaxed) - b0;
    println!(
        "{label:<44} allocs={a:>7}  per strip={:>8.3}  bytes={b:>12}",
        a as f64 / strips as f64
    );
}

#[path = "../benches/fixture.rs"]
mod fixture;
use fixture::*;

fn main() {
    let (w, h, strip_rows) = (1024u32, 1024u32, 16u32);
    let file = synthetic_file(w, h);
    let strips = (h / strip_rows) as u64;
    println!(
        "image {w}x{h} RGBA8 ({} MiB), {strips} strips of {strip_rows} rows\n",
        (w as usize * h as usize * 4) >> 20
    );

    measure("baseline: direct sink writes, no driver", strips, || {
        let mut sink = zencodec_io::CollectSink::new();
        direct_decode(&file, strip_rows, &mut sink);
    });
    measure(
        "hand-written feed/step via drive_from_slice",
        strips,
        || {
            let mut dec = HandWritten::new(strip_rows);
            let mut sink = zencodec_io::CollectSink::new();
            zencodec_io::drive_from_slice(&mut dec, &file, &mut sink, None, 1 << 20).unwrap();
        },
    );
    measure(
        "hand-written via pump_read_to_sink (64 KiB)",
        strips,
        || {
            let mut dec = HandWritten::new(strip_rows);
            let mut sink = zencodec_io::CollectSink::new();
            zencodec_io::pump_read_to_sink(
                std::io::Cursor::new(&file),
                &mut dec,
                &mut sink,
                None,
                64 << 10,
            )
            .unwrap();
        },
    );
    measure(
        "coroutine (sequential) via drive_from_slice",
        strips,
        || {
            let mut dec = coroutine_sequential(strip_rows);
            let mut sink = zencodec_io::CollectSink::new();
            zencodec_io::drive_from_slice(&mut dec, &file, &mut sink, None, 1 << 20).unwrap();
        },
    );
    measure("coroutine (ranges) via drive_from_read_at", strips, || {
        let mut dec = coroutine_ranges(strip_rows);
        let mut sink = zencodec_io::CollectSink::new();
        let mut src = zencodec::source::SliceReadAt::new(&file);
        zencodec_io::drive_from_read_at(
            &mut dec,
            &mut src,
            &mut sink,
            None,
            zencodec_io::DriveOptions::default(),
        )
        .unwrap();
    });
    #[cfg(feature = "tokio")]
    {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let shared: std::sync::Arc<[u8]> = file.clone().into();
        measure("hand-written via tokio decode_on_thread", strips, || {
            rt.block_on(async {
                zencodec_io::thread::decode_on_thread(
                    HandWritten::new(strip_rows),
                    zencodec_io::CollectSink::new(),
                    None,
                    zencodec_io::thread::ThreadOptions::default(),
                    zencodec_io::thread::fetch_from_bytes(shared.clone(), 1 << 20),
                )
                .await
                .unwrap()
            });
        });
        measure(
            "same, strips streamed back via ChannelRowSink",
            strips,
            || {
                rt.block_on(async {
                    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
                    let decode = zencodec_io::thread::decode_on_thread(
                        HandWritten::new(strip_rows),
                        zencodec_io::thread::ChannelRowSink::new(tx, false),
                        None,
                        zencodec_io::thread::ThreadOptions::default(),
                        zencodec_io::thread::fetch_from_bytes(shared.clone(), 1 << 20),
                    );
                    let consume = async move {
                        let mut n = 0u64;
                        while let Some(ev) = rx.recv().await {
                            if matches!(ev, zencodec_io::thread::StripEvent::Finish) {
                                break;
                            }
                            n += 1;
                        }
                        n
                    };
                    let (r, _) = tokio::join!(decode, consume);
                    r.unwrap();
                });
            },
        );
    }
}
