//! Mechanical cancellation shared by the private native collection adapters.
//! The existing SharedPulse is configured once when opening an operation.
use super::tiff_region::Failure;
use std::io::{self, Read, Seek, SeekFrom};
use zencodec::{SharedPulse, enough::Stop};

#[derive(Clone, Default)]
pub struct Control {
    stop: Option<SharedPulse>,
}
impl Control {
    pub fn new(stop: SharedPulse) -> Self {
        Self { stop: Some(stop) }
    }
    pub fn token(&self) -> Option<&SharedPulse> {
        self.stop.as_ref()
    }
    pub fn check(&self) -> Result<(), Failure> {
        self.check_io().map_err(Into::into)
    }
    fn check_io(&self) -> io::Result<()> {
        self.stop
            .as_ref()
            .map_or(Ok(()), Stop::check)
            .map_err(|reason| io::Error::other(zencodec::CodecError::new(None, reason.into())))
    }
    pub fn reader<R>(&self, input: R) -> CheckedReader<R> {
        CheckedReader {
            input,
            control: self.clone(),
        }
    }
}

/// Checks both sides of a native parser's I/O call. A successful last read
/// cannot publish a directory or raster after the source cancels the operation.
/// This cannot interrupt a blocking source call; transport deadlines remain
/// the source's obligation. Native TIFF compute checks are at coding-unit seams.
pub struct CheckedReader<R> {
    input: R,
    control: Control,
}
impl<R: Read> Read for CheckedReader<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.control.check_io()?;
        let count = self.input.read(out)?;
        self.control.check_io()?;
        Ok(count)
    }
}
impl<R: Seek> Seek for CheckedReader<R> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.control.check_io()?;
        let position = self.input.seek(position)?;
        self.control.check_io()?;
        Ok(position)
    }
}

/// Deterministic cancellation injection for adapter conformance, not profiling.
#[derive(Clone, Default)]
pub struct CheckStop(std::sync::Arc<CheckState>);
struct CheckState {
    calls: std::sync::atomic::AtomicUsize,
    fail_at: std::sync::atomic::AtomicUsize,
    reason: zencodec::enough::StopReason,
}
impl Default for CheckState {
    fn default() -> Self {
        Self {
            calls: std::sync::atomic::AtomicUsize::new(0),
            fail_at: std::sync::atomic::AtomicUsize::new(usize::MAX),
            reason: zencodec::enough::StopReason::Cancelled,
        }
    }
}
impl CheckStop {
    pub fn with_reason(reason: zencodec::enough::StopReason) -> Self {
        Self(std::sync::Arc::new(CheckState {
            reason,
            ..CheckState::default()
        }))
    }
    pub fn calls(&self) -> usize {
        self.0.calls.load(std::sync::atomic::Ordering::SeqCst)
    }
    pub fn after(&self, checks: usize) {
        self.0.fail_at.store(
            self.calls().checked_add(checks).unwrap(),
            std::sync::atomic::Ordering::SeqCst,
        );
    }
    pub fn token(&self) -> SharedPulse {
        SharedPulse::new(zencodec::how_far::StopOnly::new(self.clone()))
    }
}
impl Stop for CheckStop {
    fn check(&self) -> Result<(), zencodec::enough::StopReason> {
        use std::sync::atomic::Ordering;
        let call = self.0.calls.fetch_add(1, Ordering::SeqCst);
        if call >= self.0.fail_at.load(Ordering::SeqCst) {
            Err(self.0.reason)
        } else {
            Ok(())
        }
    }
}
