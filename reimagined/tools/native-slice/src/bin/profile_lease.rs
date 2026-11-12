//! Keep the published zenbench exclusive lease around an external profiler.
#![forbid(unsafe_code)]
use std::{
    process::{Command, ExitCode},
    time::Duration,
};
use zenbench::exclusive::{AcquireConfig, Lock};
fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let Some(program) = args.next() else {
        eprintln!("profile_lease PROGRAM ARGS...");
        return ExitCode::FAILURE;
    };
    let config = AcquireConfig {
        timeout: Some(Duration::from_secs(300)),
        project: "zencodec".into(),
        binary: "resident-cpu-accounting".into(),
        benchmark: "external perf process accounting".into(),
        activity: "diagnostic CPU/wall counters; no latency acceptance".into(),
        ..AcquireConfig::default()
    };
    let _lease = match Lock::acquire(config) {
        Ok(lock) => lock,
        Err(e) => {
            eprintln!("zenbench lease: {e}");
            return ExitCode::FAILURE;
        }
    };
    match Command::new(program).args(args).status() {
        Ok(s) if s.success() => ExitCode::SUCCESS,
        Ok(s) => {
            eprintln!("profile command: {s}");
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("profile command: {e}");
            ExitCode::FAILURE
        }
    }
}
