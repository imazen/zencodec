//! Direct source contracts: refuse malformed requests before I/O, retain typed
//! cancellation, and discard file padding with bounded, interruptible reads.
use enough::{Stop, StopReason};
use multipass_validation::{
    PassSource,
    sources::{FileRows, ReaderRows},
};
use std::{
    cell::Cell,
    fs::OpenOptions,
    io::{self, Read, Write},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering::Relaxed},
    },
};
use zencodec::{
    CodecError, ErrorCategory, find_cause,
    how_far::{Child, Execution, NoPulse, PhaseSpec, PlanError, Pulse, Report, Total},
};
use zencodec_io::row_encode::Description;
use zenpixels::{PixelDescriptor, PixelSliceMut};

struct Reads {
    calls: Rc<Cell<usize>>,
}
impl Read for Reads {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.calls.set(self.calls.get() + 1);
        bytes.fill(42);
        Ok(bytes.len())
    }
}

#[test]
fn invalid_reader_requests_do_not_consume_input_or_change_destination() {
    // Short source stride, wrong width, wrong descriptor, and too many rows. Every request has a valid destination view.
    for (stride, width, rows, descriptor, source_height) in [
        (5, 2, 1, PixelDescriptor::RGB8_SRGB, 2),
        (6, 1, 1, PixelDescriptor::RGB8_SRGB, 2),
        (6, 2, 1, PixelDescriptor::GRAY8_SRGB, 2),
        (6, 2, 3, PixelDescriptor::RGB8_SRGB, 2),
    ] {
        let calls = Rc::new(Cell::new(0));
        let mut source = ReaderRows::new(
            Reads {
                calls: calls.clone(),
            },
            stride,
            Description::new(2, source_height, PixelDescriptor::RGB8_SRGB),
        );
        let mut bytes = [77; 36];
        let view = PixelSliceMut::new(&mut bytes, width, rows, 12, descriptor).unwrap();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            source.read_rows(0, view, &NoPulse)
        }));
        assert_eq!(calls.get(), 0, "invalid declaration consumed source bytes");
        assert_eq!(bytes, [77; 36]);
        assert_eq!(
            result
                .expect("malformed stride must not panic")
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
    }
}

#[test]
fn invalid_file_requests_do_not_write_destination() {
    let directory = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join(format!("source-contract-{}", std::process::id()));
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)
        .unwrap();
    file.write_all(&[42; 32]).unwrap();
    for (origin, stride, first, width, rows, descriptor) in [
        (0, 5, 0, 2, 1, PixelDescriptor::RGB8_SRGB),
        (0, 6, 0, 1, 1, PixelDescriptor::RGB8_SRGB),
        (0, 6, 0, 2, 1, PixelDescriptor::GRAY8_SRGB),
        (0, 6, 1, 2, 2, PixelDescriptor::RGB8_SRGB),
        (0, 6, u32::MAX, 2, 2, PixelDescriptor::RGB8_SRGB),
        (u64::MAX - 8, 6, 0, 2, 2, PixelDescriptor::RGB8_SRGB),
    ] {
        let mut source = FileRows::new(
            file.try_clone().unwrap(),
            origin,
            stride,
            Description::new(2, 2, PixelDescriptor::RGB8_SRGB),
        );
        let mut bytes = [77; 24];
        let view = PixelSliceMut::new(&mut bytes, width, rows, 12, descriptor).unwrap();
        let result = source.read_rows(first, view, &NoPulse);
        assert_eq!(bytes, [77; 24], "invalid declaration touched destination");
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidInput);
    }
    drop(file);
    std::fs::remove_file(path).unwrap();
}

struct CancelAt {
    consumed: Arc<AtomicUsize>,
    threshold: usize,
}
impl Stop for CancelAt {
    fn check(&self) -> Result<(), StopReason> {
        if self.consumed.load(Relaxed) >= self.threshold {
            Err(StopReason::TimedOut)
        } else {
            Ok(())
        }
    }
}
impl Report for CancelAt {
    fn advance(&self, _: u64) {}
}
impl Pulse for CancelAt {
    fn split(&self, _: Execution, _: &[PhaseSpec<'_>]) -> Result<Vec<Child<'_>>, PlanError> {
        Err(PlanError::Unsupported)
    }
    fn set_total(&self, _: Total) -> Result<(), PlanError> {
        Ok(())
    }
}
struct PaddingReader {
    consumed: Arc<AtomicUsize>,
    largest: Rc<Cell<usize>>,
}
impl Read for PaddingReader {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.largest.set(self.largest.get().max(bytes.len()));
        self.consumed.fetch_add(bytes.len(), Relaxed);
        bytes.fill(0);
        Ok(bytes.len())
    }
}

#[test]
fn padding_is_bounded_and_cancellable_with_typed_reason() {
    let consumed = Arc::new(AtomicUsize::new(0));
    let largest = Rc::new(Cell::new(0));
    let reader = PaddingReader {
        consumed: consumed.clone(),
        largest: largest.clone(),
    };
    let mut source = ReaderRows::new(
        reader,
        1 << 20,
        Description::new(2, 1, PixelDescriptor::RGB8_SRGB),
    );
    let pulse = CancelAt {
        consumed: consumed.clone(),
        threshold: 4096,
    };
    let mut bytes = [77; 6];
    let result = source.read_rows(
        0,
        PixelSliceMut::new(&mut bytes, 2, 1, 6, PixelDescriptor::RGB8_SRGB).unwrap(),
        &pulse,
    );
    assert!(
        largest.get() <= 4096,
        "padding requested {} bytes at once",
        largest.get()
    );
    assert!(
        consumed.load(Relaxed) < 1 << 20,
        "cancellation failed to interrupt padding"
    );
    let error = result.unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert_eq!(
        find_cause::<CodecError>(&error)
            .expect("typed stop cause")
            .category(),
        ErrorCategory::Stopped(StopReason::TimedOut)
    );
}

#[test]
fn cancellation_before_read_keeps_typed_timeout() {
    let calls = Rc::new(Cell::new(0));
    let mut source = ReaderRows::new(
        Reads {
            calls: calls.clone(),
        },
        6,
        Description::new(2, 1, PixelDescriptor::RGB8_SRGB),
    );
    let mut bytes = [77; 6];
    let pulse = CancelAt {
        consumed: Arc::new(AtomicUsize::new(0)),
        threshold: 0,
    };
    let error = source
        .read_rows(
            0,
            PixelSliceMut::new(&mut bytes, 2, 1, 6, PixelDescriptor::RGB8_SRGB).unwrap(),
            &pulse,
        )
        .unwrap_err();
    assert_eq!(calls.get(), 0);
    assert_eq!(bytes, [77; 6]);
    assert_eq!(
        find_cause::<CodecError>(&error)
            .expect("typed stop cause")
            .category(),
        ErrorCategory::Stopped(StopReason::TimedOut)
    );
}

#[test]
fn unaligned_source_pitch_and_destination_padding_preserve_samples() {
    let raw = [1, 2, 3, 4, 5, 6, 99, 7, 8, 9, 10, 11, 12, 99];
    let mut source = ReaderRows::new(
        io::Cursor::new(raw),
        7,
        Description::new(2, 2, PixelDescriptor::RGB8_SRGB),
    );
    let mut bytes = [77; 18];
    source
        .read_rows(
            0,
            PixelSliceMut::new(&mut bytes, 2, 2, 9, PixelDescriptor::RGB8_SRGB).unwrap(),
            &NoPulse,
        )
        .unwrap();
    assert_eq!(
        bytes,
        [
            1, 2, 3, 4, 5, 6, 77, 77, 77, 7, 8, 9, 10, 11, 12, 77, 77, 77
        ]
    );
}
