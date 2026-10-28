#![forbid(unsafe_code)]
#[path = "../perf/frame-policy-cost.rs"]
mod workload;

fn main() {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).is_some_and(|mode| mode == "layout") {
        assert_eq!(args.len(), 2, "layout takes no count");
        // Rust value layout only; this is not allocation, RSS or stack-peak data.
        println!(
            "TranscodeOffer\t{}",
            core::mem::size_of::<zencodec::operation::TranscodeOffer>()
        );
        println!(
            "OutputScheduleOffer\t{}",
            core::mem::size_of::<zencodec::operation::OutputScheduleOffer>()
        );
        println!(
            "FrameRouteOffer\t{}",
            core::mem::size_of::<zencodec::operation::FrameRouteOffer>()
        );
        return;
    }
    assert_eq!(
        args.len(),
        3,
        "MODE COUNT (record, resolve, combined), or layout"
    );
    assert!(["record", "resolve", "combined"].contains(&args[1].as_str()));
    let count: usize = args[2].parse().unwrap();
    assert!(count <= 10_000_000);
    workload::Workload::new().verify(&args[1], count);
}
