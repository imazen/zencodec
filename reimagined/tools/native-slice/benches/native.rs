use reimagined_native_slice::{ARMS, Kind, execute, fixture, qualify};
use zenbench::prelude::*;
fn compare(suite: &mut Suite) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .build()
        .unwrap();
    for kind in [Kind::Qoi, Kind::Bmp, Kind::Jpeg] {
        for size in [8, 256, 1024] {
            let file = fixture(kind, size);
            qualify(kind, &file, &rt);
            suite.group(format!("{}/{size}x{size}", kind.name()), |g| {
                g.config()
                    .max_rounds(30)
                    .max_time(std::time::Duration::from_secs(20));
                for arm in ARMS {
                    let file = file.clone();
                    let rt = tokio::runtime::Builder::new_multi_thread()
                        .worker_threads(2)
                        .build()
                        .unwrap();
                    g.bench(arm, move |b| b.iter(|| execute(arm, kind, &file, &rt)));
                }
            });
        }
    }
}
zenbench::main!(compare);
