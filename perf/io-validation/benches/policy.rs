#![forbid(unsafe_code)]
#[path = "../src/policy/mod.rs"]
mod policy;
#[path = "../src/policy_benchmark.rs"]
mod workload;
use zenbench::prelude::*;
fn resolver(suite: &mut Suite) {
    suite.group("one-operation-three-offers", |g| {
        g.config()
            .max_time(std::time::Duration::from_secs(1))
            .max_rounds(20);
        for (mode, name) in workload::Mode::ALL {
            g.bench(name, move |b| {
                b.iter(|| std::hint::black_box(workload::operation(mode)))
            });
        }
    });
}
zenbench::main!(resolver);
