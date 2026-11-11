use std::{
    io::{self, Read, Seek, SeekFrom},
    sync::mpsc,
    thread,
};
use zencodec::{ByteRange, ErrorKind, Input, InputRequest, Result, SharedPulse, check, error_msg};

enum Message<T> {
    Read(u64, usize),
    Len,
    Done(Result<T>),
}
enum Reply {
    Bytes(Vec<u8>),
    Len(u64),
}

pub(super) struct NativeReader<T> {
    tx: mpsc::SyncSender<Message<T>>,
    rx: mpsc::Receiver<Reply>,
    position: u64,
    pulse: SharedPulse,
}
impl<T> NativeReader<T> {
    fn exchange(&mut self, message: Message<T>) -> io::Result<Reply> {
        check(&self.pulse).map_err(io::Error::other)?;
        self.tx
            .send(message)
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "native operation abandoned"))?;
        self.rx
            .recv()
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "native source abandoned"))
    }
}
impl<T> Read for NativeReader<T> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        let Reply::Bytes(bytes) = self.exchange(Message::Read(self.position, out.len()))? else {
            return Err(io::Error::other("wrong native reply"));
        };
        if bytes.len() > out.len() {
            return Err(io::Error::other("oversized native reply"));
        }
        out[..bytes.len()].copy_from_slice(&bytes);
        self.position += bytes.len() as u64;
        Ok(bytes.len())
    }
}
impl<T> Seek for NativeReader<T> {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let next = match from {
            SeekFrom::Start(position) => Some(position),
            SeekFrom::Current(delta) => self.position.checked_add_signed(delta),
            SeekFrom::End(delta) => {
                let Reply::Len(length) = self.exchange(Message::Len)? else {
                    return Err(io::Error::other("wrong length reply"));
                };
                length.checked_add_signed(delta)
            }
        }
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "native seek overflow"))?;
        self.position = next;
        Ok(next)
    }
}

pub(super) enum Progress<T> {
    Need,
    Continue,
    Done(T),
}
pub(super) struct Operation<T> {
    rx: Option<mpsc::Receiver<Message<T>>>,
    tx: Option<mpsc::SyncSender<Reply>>,
    worker: Option<thread::JoinHandle<()>>,
    requested: Vec<InputRequest>,
}
impl<T: Send + 'static> Operation<T> {
    pub(super) fn new(
        pulse: SharedPulse,
        work: impl FnOnce(NativeReader<T>) -> Result<T> + Send + 'static,
    ) -> Self {
        let (send, rx) = mpsc::sync_channel(1);
        let (tx, recv) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            let reader = NativeReader {
                tx: send.clone(),
                rx: recv,
                position: 0,
                pulse,
            };
            let result = work(reader);
            let _ = send.send(Message::Done(result));
        });
        Self {
            rx: Some(rx),
            tx: Some(tx),
            worker: Some(worker),
            requested: Vec::new(),
        }
    }
    pub(super) fn requested(&self) -> &[InputRequest] {
        &self.requested
    }
    pub(super) fn step(&mut self, input: Input<'_>) -> Result<(usize, Progress<T>)> {
        let mut consumed = 0;
        if let Some(request) = self.requested.first() {
            if !request.is_met_by(&input) {
                return Ok((0, Progress::Need));
            }
            let reply = match request {
                InputRequest::Range(range) => {
                    let skip = usize::try_from(range.offset - input.offset)
                        .map_err(|_| error_msg(ErrorKind::Protocol, "native window offset"))?;
                    let available = input
                        .bytes
                        .get(skip..)
                        .ok_or_else(|| error_msg(ErrorKind::Protocol, "native window too short"))?;
                    let len = available.len().min(range.length as usize);
                    let mut bytes = Vec::new();
                    bytes.try_reserve_exact(len).map_err(|_| {
                        error_msg(ErrorKind::OutOfMemory, "native reply allocation")
                    })?;
                    bytes.extend_from_slice(&available[..len]);
                    consumed = skip + len;
                    Reply::Bytes(bytes)
                }
                InputRequest::Len => Reply::Len(input.total_len.ok_or_else(|| {
                    error_msg(ErrorKind::Protocol, "native source has no length")
                })?),
                _ => return Err(error_msg(ErrorKind::Protocol, "unsupported native request")),
            };
            self.tx.as_ref().unwrap().send(reply).map_err(|_| {
                error_msg(ErrorKind::Internal, "native worker stopped before reply")
            })?;
            self.requested.clear();
        }
        match self.rx.as_ref().unwrap().try_recv() {
            Ok(Message::Read(offset, length)) => {
                self.requested
                    .push(InputRequest::Range(ByteRange::new(offset, length as u64)));
                Ok((consumed, Progress::Need))
            }
            Ok(Message::Len) => {
                self.requested.push(InputRequest::Len);
                Ok((consumed, Progress::Need))
            }
            Ok(Message::Done(result)) => {
                let value = result?;
                if self.worker.take().unwrap().join().is_err() {
                    return Err(error_msg(ErrorKind::Internal, "native worker panicked"));
                }
                Ok((consumed, Progress::Done(value)))
            }
            Err(mpsc::TryRecvError::Empty) => Ok((consumed, Progress::Continue)),
            Err(mpsc::TryRecvError::Disconnected) => {
                Err(error_msg(ErrorKind::Internal, "native worker disconnected"))
            }
        }
    }
}
impl<T> Drop for Operation<T> {
    fn drop(&mut self) {
        self.tx.take();
        self.rx.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
