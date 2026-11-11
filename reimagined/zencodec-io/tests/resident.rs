//! Verify the caller's allocation is lent, and fast paths keep driver controls.
mod common;

#[cfg(feature = "tokio")]
use common::Switch;
#[cfg(feature = "tokio")]
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use zencodec::decode::{DecodeEvent, DecodePlan, DecodeStep, Decoder, Sink};
use zencodec::{ByteRange, ImageInfo, Input, InputRequest, Result};
use zencodec_io::{Collect, DriveOptions, decode_read_at, decode_slice};

struct PointerCheck {
    base: usize,
    request: [InputRequest; 1],
    phase: usize,
}
impl PointerCheck {
    fn new(bytes: &[u8]) -> Self {
        Self {
            base: bytes.as_ptr() as usize,
            request: [InputRequest::All],
            phase: 0,
        }
    }
}
impl Decoder for PointerCheck {
    fn requested(&self) -> &[InputRequest] {
        &self.request
    }
    fn info(&self) -> Option<&ImageInfo> {
        None
    }
    fn plan(&self) -> Option<&DecodePlan> {
        None
    }
    fn step(&mut self, input: Input<'_>, _: &mut dyn Sink) -> Result<DecodeStep> {
        let (offset, len) = [(0, 32), (3, 5), (11, 21)][self.phase];
        assert_eq!((input.offset, input.bytes.len()), (offset, len));
        assert_eq!(input.bytes.as_ptr() as usize, self.base + offset as usize);
        assert_eq!(input.total_len, Some(32));
        self.phase += 1;
        self.request = [match self.phase {
            1 => InputRequest::Range(ByteRange::new(3, 5)),
            _ => InputRequest::AtLeast {
                offset: 11,
                min_len: 3,
            },
        }];
        Ok(DecodeStep::new(
            0,
            if self.phase == 3 {
                DecodeEvent::Done
            } else {
                DecodeEvent::Info
            },
        ))
    }
}

#[test]
fn slice_and_positioned_resident_paths_lend_the_original_allocation() {
    let bytes = [0x95; 32];
    decode_slice(
        Box::new(PointerCheck::new(&bytes)),
        &bytes,
        &mut Collect::new(),
    )
    .unwrap();
    // The cap is for retained driver input, not a borrow of caller-owned bytes.
    decode_read_at(
        Box::new(PointerCheck::new(&bytes)),
        &mut &bytes[..],
        &mut Collect::new(),
        &DriveOptions::new().with_max_input_bytes(0),
    )
    .unwrap();
}

#[cfg(feature = "tokio")]
#[tokio::test]
async fn inline_resident_paths_lend_the_original_allocation_across_yields() {
    use zencodec_io::tokio_io::{self, InMemory, Offload};
    let bytes = [0x95; 32];
    tokio_io::decode(
        Box::new(PointerCheck::new(&bytes)),
        &mut InMemory(&bytes[..]),
        Collect::new(),
        Offload::Inline,
        DriveOptions::new()
            .with_max_input_bytes(0)
            .with_steps_per_batch(1),
        zencodec::SharedPulse::default(),
    )
    .await
    .unwrap();
}

#[cfg(feature = "tokio")]
struct Stops {
    switch: Switch,
    calls: Arc<AtomicUsize>,
    done: bool,
}
#[cfg(feature = "tokio")]
impl Decoder for Stops {
    fn requested(&self) -> &[InputRequest] {
        &[]
    }
    fn info(&self) -> Option<&ImageInfo> {
        None
    }
    fn plan(&self) -> Option<&DecodePlan> {
        None
    }
    fn step(&mut self, _: Input<'_>, _: &mut dyn Sink) -> Result<DecodeStep> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.switch.stop();
        Ok(DecodeStep::new(
            0,
            if self.done {
                DecodeEvent::Done
            } else {
                self.done = true;
                DecodeEvent::Info
            },
        ))
    }
}

#[cfg(feature = "tokio")]
#[tokio::test]
async fn resident_decode_keeps_per_step_and_completion_cancellation() {
    use zencodec_io::tokio_io::{self, InMemory, Offload};
    for done in [false, true] {
        let switch = Switch::default();
        let calls = Arc::new(AtomicUsize::new(0));
        let result = tokio_io::decode(
            Box::new(Stops {
                switch: switch.clone(),
                calls: calls.clone(),
                done,
            }),
            &mut InMemory(&b"bytes"[..]),
            Collect::new(),
            Offload::Inline,
            DriveOptions::new(),
            switch.pulse(),
        )
        .await;
        let err = result
            .err()
            .expect("stop must prevent another step and successful completion");
        assert_eq!(
            err.error().kind(),
            zencodec::ErrorKind::Stopped(zencodec::StopReason::Cancelled)
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}

#[cfg(feature = "tokio")]
#[tokio::test]
async fn a_directly_assigned_zero_quota_still_steps() {
    use zencodec_io::tokio_io::{self, InMemory, Offload};
    let bytes = [0x95; 32];
    let mut opts = DriveOptions::new();
    opts.steps_per_batch = 0;
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        tokio_io::decode(
            Box::new(PointerCheck::new(&bytes)),
            &mut InMemory(&bytes[..]),
            Collect::new(),
            Offload::Inline,
            opts,
            zencodec::SharedPulse::default(),
        ),
    )
    .await
    .expect("no endless yields")
    .unwrap();
}
