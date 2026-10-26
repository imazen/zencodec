//! Shared workload for wall-clock and fixed-work profiling of the real policy API.
use std::hint::black_box;
use zencodec::{
    frame::{
        ChromaSiting, FrameGeometry, FrameInterpretation, FrameTiming, PlaneRole, SampleEncoding,
        Subsampling,
    },
    operation::{
        DecisionContext, DecisionRecord, OperationReport, PolicySet, PublicationOffer, Selection,
        resolve_frame,
    },
    time::{TimeBase, Timestamp},
};

pub struct Workload {
    interpretation: FrameInterpretation,
    seed: DecisionRecord,
}
impl Workload {
    pub fn new() -> Self {
        let interpretation = FrameInterpretation::new(
            FrameGeometry::full(1920, 1080),
            Subsampling::Yuv420,
            ChromaSiting::Unknown,
            SampleEncoding::new(zenpixels::ChannelType::U16, 10, 0).unwrap(),
            vec![PlaneRole::Luma, PlaneRole::Cb, PlaneRole::Cr],
        );
        let seed = resolve_frame(
            &interpretation,
            Self::timing(0),
            &mut PolicySet::new(),
            PolicySet::new(),
            &enough::Unstoppable,
        )
        .unwrap();
        Self {
            interpretation,
            seed,
        }
    }

    fn timing(i: usize) -> FrameTiming {
        FrameTiming::at(Timestamp::new(
            i as i64,
            TimeBase::new(1001, 30000).unwrap(),
        ))
        .with_packet(Some(i as u64), Some(i as u64 * 100))
    }

    pub fn record(&self, i: usize) -> DecisionRecord {
        let mut record = self.seed;
        record.question = i as u64 + 1;
        if let DecisionContext::Frame(ref mut facts) = record.context {
            facts.timing = Self::timing(i);
        }
        record
    }

    pub fn resolve(&self, i: usize) -> DecisionRecord {
        resolve_frame(
            black_box(&self.interpretation),
            Self::timing(i),
            &mut PolicySet::new(),
            PolicySet::new(),
            &enough::Unstoppable,
        )
        .unwrap()
    }

    pub fn run(&self, mode: &str, count: usize) -> OperationReport {
        let mut report = OperationReport::default();
        for i in 0..count {
            let record = if mode == "record" {
                self.record(i)
            } else {
                self.resolve(i)
            };
            if mode != "resolve" {
                report.record(black_box(record));
            } else {
                black_box(record);
            }
        }
        report
    }

    pub fn verify(&self, mode: &str, count: usize) {
        let report = self.run(mode, count);
        if mode == "resolve" {
            assert_eq!(report.decisions().count(), 0);
            return;
        }
        assert_eq!(report.decisions().count(), count.min(16));
        assert_eq!(report.dropped_decisions(), count.saturating_sub(16) as u64);
        for (i, record) in (count.saturating_sub(16)..count).zip(report.decisions()) {
            let DecisionContext::Frame(facts) = record.context else {
                panic!("lost frame context")
            };
            assert_eq!(facts.timing, Self::timing(i));
            assert!(matches!(
                record.selected,
                Some(Selection::Frame(PublicationOffer::PreserveNative))
            ));
        }
    }
}
