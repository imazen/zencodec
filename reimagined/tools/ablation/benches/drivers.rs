//! End-to-end synthetic mechanisms; construction, copying and drop are timed.
#![forbid(unsafe_code)]
use reimagined_driver_ablation::{ARMS, execute, fixture, qualify};
use zenbench::prelude::*;

fn compare(suite: &mut Suite) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    qualify(&rt);
    for (name, w, h, strip) in [
        ("256x256/4-row", 256, 256, 4),
        ("1024x1024/16-row", 1024, 1024, 16),
        ("1024x1024/1-row", 1024, 1024, 1),
    ] {
        let file = fixture(w, h);
        suite.group(name, |g| {
            g.throughput(Throughput::Bytes(w as u64 * h as u64 * 4));
            g.config()
                .max_rounds(30)
                .max_time(std::time::Duration::from_secs(20));
            for arm in ARMS {
                let f = file.clone();
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .build()
                    .unwrap();
                g.bench(arm, move |b| {
                    b.iter(|| execute(arm, &f, strip, 0, 64, &runtime))
                });
            }
        });
    }
    let file = fixture(256, 256);
    for quota in [1usize, 64, usize::MAX] {
        suite.group(format!("quota/256x256/1-row/{quota}"), |g| {
            g.config()
                .max_rounds(30)
                .max_time(std::time::Duration::from_secs(20));
            for arm in [
                "older_tokio_inline",
                "next_tokio_inline",
                "older_tokio_pool",
                "next_tokio_pool",
                "next_buffered_one_hop",
            ] {
                let f = file.clone();
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .build()
                    .unwrap();
                g.bench(arm, move |b| {
                    b.iter(|| execute(arm, &f, 1, 0, quota, &runtime))
                });
            }
        });
    }
}
zenbench::main!(compare);
