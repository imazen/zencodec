#![forbid(unsafe_code)]
#[path = "../perf/frame-policy-cost.rs"]
mod workload;
use zenbench::prelude::*;

fn frame_policy(suite: &mut Suite) {
    for count in [1, 16, 256] {
        suite.group(format!("frame-policy/{count}-presentations"), |g| {
            g.throughput(Throughput::Elements(count as u64));
            g.config()
                .max_time(std::time::Duration::from_secs(1))
                .max_rounds(20);
            for mode in ["record", "resolve", "combined"] {
                let workload = workload::Workload::new();
                workload.verify(mode, count);
                g.bench(mode, move |b| {
                    b.iter(|| std::hint::black_box(workload.run(mode, count)))
                });
            }
        });
    }
}
zenbench::main!(frame_policy);
