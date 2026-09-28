//! Bounded, sequential AV1 IVF I/O over blocking `Read` / `Write`.
//!
//! Network reads may be arbitrarily short. A complete packet is assembled before
//! being passed to the AV1 decoder; arbitrary transport fragments are not OBUs.
//! Writers use IVF's zero (unknown) frame count and never seek back to the header.
//! IVF carries no audio, loop count, per-frame duration, or container color record.

use crate::time::{TimeBase, Timestamp};
use std::io::{self, Read, Write};
#[cfg(feature = "av1-decode")]
use std::io::{Seek, SeekFrom};

#[derive(Clone, Copy, Debug)]
pub struct IvfInfo {
    width: u16,
    height: u16,
    time_base: TimeBase,
    declared_frames: Option<u32>,
}

#[cfg(feature = "av1-decode")]
impl<R: Read + Seek> IvfReader<R> {
    /// Restore an index position belonging to this same, unchanged source.
    pub(crate) fn resume(
        mut reader: R,
        info: IvfInfo,
        payload_offset: u64,
        ordinal: u64,
        max_packet_bytes: usize,
    ) -> io::Result<Self> {
        let offset = payload_offset
            .checked_sub(12)
            .ok_or_else(|| invalid("invalid indexed offset"))?;
        reader.seek(SeekFrom::Start(offset))?;
        Ok(Self {
            reader,
            info,
            max_packet_bytes,
            offset,
            packets: ordinal,
            ended: false,
            failed: false,
        })
    }
}

impl IvfInfo {
    pub fn width(self) -> u16 {
        self.width
    }
    pub fn height(self) -> u16 {
        self.height
    }
    pub fn time_base(self) -> TimeBase {
        self.time_base
    }
    pub fn declared_frames(self) -> Option<u32> {
        self.declared_frames
    }
}

/// One compressed temporal unit. Its timestamp is presentation time, not DTS.
#[derive(Debug)]
#[non_exhaustive]
pub struct IvfPacket {
    pub data: Vec<u8>,
    pub timestamp: Timestamp,
    pub byte_offset: u64,
    pub ordinal: u64,
}

pub struct IvfReader<R> {
    reader: R,
    info: IvfInfo,
    max_packet_bytes: usize,
    offset: u64,
    packets: u64,
    ended: bool,
    failed: bool,
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

impl<R: Read> IvfReader<R> {
    /// Parse the header, accepting extended headers but only AV1 and version 0.
    /// `max_packet_bytes` limits each allocation before any payload is read.
    pub fn new(mut reader: R, max_packet_bytes: usize) -> io::Result<Self> {
        if max_packet_bytes == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "packet limit must be positive",
            ));
        }
        let mut header = [0; 32];
        reader.read_exact(&mut header)?;
        if &header[..4] != b"DKIF" || u16::from_le_bytes(header[4..6].try_into().unwrap()) != 0 {
            return Err(invalid("unsupported IVF signature or version"));
        }
        if &header[8..12] != b"AV01" {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "IVF codec is not AV1",
            ));
        }
        let header_len = u16::from_le_bytes(header[6..8].try_into().unwrap());
        if header_len < 32 {
            return Err(invalid("IVF header length is shorter than 32 bytes"));
        }
        let width = u16::from_le_bytes(header[12..14].try_into().unwrap());
        let height = u16::from_le_bytes(header[14..16].try_into().unwrap());
        if width == 0 || height == 0 {
            return Err(invalid("zero IVF dimensions"));
        }
        let rate = u32::from_le_bytes(header[16..20].try_into().unwrap());
        let scale = u32::from_le_bytes(header[20..24].try_into().unwrap());
        let time_base = TimeBase::new(scale, rate).map_err(|_| invalid("zero IVF time base"))?;
        let frames = u32::from_le_bytes(header[24..28].try_into().unwrap());
        // The u16 header length bounds this work. Skip without an allocation.
        let mut scratch = [0; 512];
        let mut remaining = usize::from(header_len) - 32;
        while remaining != 0 {
            let take = remaining.min(scratch.len());
            reader.read_exact(&mut scratch[..take])?;
            remaining -= take;
        }
        Ok(Self {
            reader,
            info: IvfInfo {
                width,
                height,
                time_base,
                // FFmpeg uses all-ones when nonseekable output prevents its
                // trailer from patching the count. Other writers use zero.
                declared_frames: (frames != 0 && frames != u32::MAX).then_some(frames),
            },
            max_packet_bytes,
            offset: u64::from(header_len),
            packets: 0,
            ended: false,
            failed: false,
        })
    }

    pub fn info(&self) -> IvfInfo {
        self.info
    }
    pub fn into_inner(self) -> R {
        self.reader
    }

    /// Read one packet, returning `None` only at a clean packet boundary.
    /// Truncated headers/payloads, mismatched counts, and over-limit packets are
    /// errors. After any error the reader is poisoned; retrying cannot reinterpret
    /// a partly consumed payload as a new packet header.
    pub fn next_packet(&mut self) -> io::Result<Option<IvfPacket>> {
        if self.failed {
            return Err(invalid("IVF reader cannot resume after an error"));
        }
        let result = self.read_packet();
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    fn read_packet(&mut self) -> io::Result<Option<IvfPacket>> {
        if self.ended {
            return Ok(None);
        }
        let mut header = [0; 12];
        loop {
            match self.reader.read(&mut header[..1]) {
                Ok(0) => {
                    self.ended = true;
                    if self
                        .info
                        .declared_frames
                        .is_some_and(|n| u64::from(n) != self.packets)
                    {
                        return Err(invalid("IVF frame count does not match packets"));
                    }
                    return Ok(None);
                }
                Ok(_) => break,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
        self.reader.read_exact(&mut header[1..])?;
        if self
            .info
            .declared_frames
            .is_some_and(|n| self.packets >= u64::from(n))
        {
            return Err(invalid("more IVF packets than declared frames"));
        }
        let len = usize::try_from(u32::from_le_bytes(header[..4].try_into().unwrap()))
            .map_err(|_| invalid("IVF packet size exceeds addressable memory"))?;
        if len == 0 || len > self.max_packet_bytes {
            return Err(invalid("IVF packet is empty or exceeds configured limit"));
        }
        // Match the signed 64-bit timestamp API used by libvpx and FFmpeg.
        let ticks = i64::from_le_bytes(header[4..].try_into().unwrap());
        let next_offset = self
            .offset
            .checked_add(12)
            .and_then(|n| n.checked_add(len as u64))
            .ok_or_else(|| invalid("IVF byte offset overflow"))?;
        let mut data = Vec::new();
        data.try_reserve_exact(len).map_err(|_| {
            io::Error::new(io::ErrorKind::OutOfMemory, "IVF packet allocation failed")
        })?;
        data.resize(len, 0);
        self.reader.read_exact(&mut data)?;
        let packet = IvfPacket {
            data,
            timestamp: Timestamp::new(ticks, self.info.time_base),
            byte_offset: self.offset + 12,
            ordinal: self.packets,
        };
        self.offset = next_offset;
        self.packets += 1; // Each packet has at least 13 bytes: offset bounds this.
        Ok(Some(packet))
    }
}

/// Sequential output with an unknown frame count; no whole-file output buffer.
pub struct IvfWriter<W> {
    writer: W,
    time_base: TimeBase,
    packets: u64,
    failed: bool,
}

impl<W: Write> IvfWriter<W> {
    pub fn new(mut writer: W, width: u16, height: u16, time_base: TimeBase) -> io::Result<Self> {
        if width == 0 || height == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "zero IVF dimensions",
            ));
        }
        let mut header = [0; 32];
        header[..4].copy_from_slice(b"DKIF");
        header[6..8].copy_from_slice(&32_u16.to_le_bytes());
        header[8..12].copy_from_slice(b"AV01");
        header[12..14].copy_from_slice(&width.to_le_bytes());
        header[14..16].copy_from_slice(&height.to_le_bytes());
        header[16..20].copy_from_slice(&time_base.denominator().to_le_bytes());
        header[20..24].copy_from_slice(&time_base.numerator().to_le_bytes());
        writer.write_all(&header)?;
        Ok(Self {
            writer,
            time_base,
            packets: 0,
            failed: false,
        })
    }

    pub fn packets_written(&self) -> u64 {
        self.packets
    }

    /// Emit a complete packet with a signed 64-bit presentation timestamp.
    /// Rescaling must be explicit at the call site; rounding is never implicit.
    pub fn write_packet(&mut self, data: &[u8], timestamp: Timestamp) -> io::Result<()> {
        if self.failed {
            return Err(invalid("IVF writer cannot resume after an I/O failure"));
        }
        if data.is_empty() || timestamp.time_base() != self.time_base {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "empty IVF packet or mismatched time base",
            ));
        }
        let len = u32::try_from(data.len()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "IVF packet exceeds 32-bit length",
            )
        })?;
        let ticks = timestamp.ticks();
        let next = self
            .packets
            .checked_add(1)
            .ok_or_else(|| invalid("IVF packet counter overflow"))?;
        let result = (|| {
            self.writer.write_all(&len.to_le_bytes())?;
            self.writer.write_all(&ticks.to_le_bytes())?;
            self.writer.write_all(data)
        })();
        if result.is_err() {
            self.failed = true;
        } else {
            self.packets = next;
        }
        result
    }

    /// Flush and return the sink. Errors from finalization are observable.
    /// The header retains an unknown frame count, which is valid for streaming IVF.
    pub fn finish(mut self) -> io::Result<W> {
        if self.failed {
            return Err(invalid(
                "cannot finalize an IVF stream after an I/O failure",
            ));
        }
        self.writer.flush()?;
        Ok(self.writer)
    }
}
