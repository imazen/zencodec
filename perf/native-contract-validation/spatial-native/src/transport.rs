//! An in-memory object read through a counting transport, a one-call fetch of
//! planned ranges (one multi-range request), and a planned reader that serves
//! the native parser from kept bytes and refuses any unplanned byte.
use std::{
    cell::RefCell,
    collections::BTreeMap,
    io::{self, Read, Seek, SeekFrom},
    rc::Rc,
};

/// Transport counters for one object.
#[derive(Clone, Debug, Default)]
pub struct Counts {
    /// Transport calls (a parser read, or one batched fetch).
    pub calls: u64,
    /// Bytes delivered by the transport, rereads included.
    pub bytes: u64,
    /// Every delivered `(offset, length)`.
    pub reads: Vec<(u64, u64)>,
}

/// Bytes delivered so far, by offset; shared by the readers of one object.
pub type Kept = Rc<RefCell<BTreeMap<u64, Vec<u8>>>>;

fn resolve(to: SeekFrom, pos: u64, len: u64) -> io::Result<u64> {
    match to {
        SeekFrom::Start(n) => Some(n),
        SeekFrom::End(d) => len.checked_add_signed(d),
        SeekFrom::Current(d) => pos.checked_add_signed(d),
    }
    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "seek outside the object"))
}

/// A parser's direct reader: each `read` is one transport call. Delivered
/// bytes are kept so a planned reader can serve them again without a call.
pub struct Recording<'a> {
    data: &'a [u8],
    pos: u64,
    counts: Rc<RefCell<Counts>>,
    kept: Kept,
}
impl<'a> Recording<'a> {
    /// Reads `data`, counting into `counts` and keeping into `kept`.
    pub fn new(data: &'a [u8], counts: Rc<RefCell<Counts>>, kept: Kept) -> Self {
        Self {
            data,
            pos: 0,
            counts,
            kept,
        }
    }
}
impl Read for Recording<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let start = usize::try_from(self.pos)
            .unwrap_or(usize::MAX)
            .min(self.data.len());
        let n = buf.len().min(self.data.len() - start);
        if n > 0 {
            buf[..n].copy_from_slice(&self.data[start..start + n]);
            let mut counts = self.counts.borrow_mut();
            counts.calls += 1;
            counts.bytes += n as u64;
            counts.reads.push((self.pos, n as u64));
            self.kept.borrow_mut().insert(self.pos, buf[..n].to_vec());
        }
        self.pos += n as u64;
        Ok(n)
    }
}
impl Seek for Recording<'_> {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        self.pos = resolve(to, self.pos, self.data.len() as u64)?;
        Ok(self.pos)
    }
}

/// Fetches every planned range in one transport call.
pub fn fetch(
    data: &[u8],
    ranges: &[(u64, u64)],
    counts: &RefCell<Counts>,
    kept: &Kept,
) -> io::Result<()> {
    let mut counts = counts.borrow_mut();
    counts.calls += 1;
    for &(at, len) in ranges {
        let start = usize::try_from(at)
            .ok()
            .filter(|&s| s <= data.len())
            .ok_or(io::ErrorKind::UnexpectedEof)?;
        let end = usize::try_from(len)
            .ok()
            .and_then(|n| start.checked_add(n))
            .filter(|&e| e <= data.len())
            .ok_or(io::ErrorKind::UnexpectedEof)?;
        counts.bytes += len;
        counts.reads.push((at, len));
        kept.borrow_mut().insert(at, data[start..end].to_vec());
    }
    Ok(())
}

/// Serves reads only from kept bytes; any other byte is an unplanned read.
pub struct Planned {
    kept: Kept,
    len: u64,
    pos: u64,
}
impl Planned {
    /// A reader of an object of `len` bytes over `kept`.
    pub fn new(kept: Kept, len: u64) -> Self {
        Self { kept, len, pos: 0 }
    }
}
impl Read for Planned {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() || self.pos >= self.len {
            return Ok(0);
        }
        let kept = self.kept.borrow();
        let covering = kept
            .range(..=self.pos)
            .rev()
            .find(|(at, bytes)| self.pos < **at + bytes.len() as u64);
        let Some((at, bytes)) = covering else {
            return Err(io::Error::other(format!("unplanned read at {}", self.pos)));
        };
        let offset = (self.pos - at) as usize;
        let n = buf.len().min(bytes.len() - offset);
        buf[..n].copy_from_slice(&bytes[offset..offset + n]);
        self.pos += n as u64;
        Ok(n)
    }
}
impl Seek for Planned {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        self.pos = resolve(to, self.pos, self.len)?;
        Ok(self.pos)
    }
}
