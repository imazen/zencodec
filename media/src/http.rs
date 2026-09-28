//! Bounded HTTP range reads over one strongly identified representation.
//!
//! This blocking adapter implements `Read + Seek` for container parsers. It
//! caches at most one configured chunk, sends identity encoding, checks every
//! Content-Range, and pins a strong ETag with If-Match. A failed range fetch
//! poisons the source, so bytes from different representations cannot mix.
use std::{
    io::{self, Read, Seek, SeekFrom},
    time::Duration,
};
use ureq::http::{HeaderMap, HeaderValue};

pub struct HttpRangeReader {
    agent: ureq::Agent,
    url: String,
    chunk_bytes: usize,
    position: u64,
    length: Option<u64>,
    etag: Option<HeaderValue>,
    cache: Vec<u8>,
    cache_start: u64,
    failed: bool,
}
fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn unsupported(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, message)
}
fn transport_error(error: ureq::Error) -> io::Error {
    match error {
        error @ ureq::Error::Timeout(_) => io::Error::new(io::ErrorKind::TimedOut, error),
        ureq::Error::Io(error) => error,
        error => io::Error::other(error),
    }
}
fn body_error(error: io::Error) -> io::Error {
    if matches!(
        error
            .get_ref()
            .and_then(|e| e.downcast_ref::<ureq::Error>()),
        Some(ureq::Error::Timeout(_))
    ) {
        io::Error::new(io::ErrorKind::TimedOut, error)
    } else {
        error
    }
}

impl HttpRangeReader {
    /// Fetch the first chunk and pin the representation. A positive timeout
    /// bounds each complete HTTP request, including response-body reads.
    /// Redirects, ignored Range requests, content coding, and absent/weak ETags
    /// are rejected explicitly. Use a resolved URL (including signed query
    /// credentials if needed). The adapter never silently downloads the file.
    pub fn open(url: &str, chunk_bytes: usize, timeout: Duration) -> io::Result<Self> {
        if chunk_bytes == 0 || timeout.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "chunk size and timeout must be positive",
            ));
        }
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .http_status_as_error(false)
            .max_redirects(0)
            .build()
            .into();
        let mut source = Self {
            agent,
            url: url.to_owned(),
            chunk_bytes,
            position: 0,
            length: None,
            etag: None,
            cache: Vec::new(),
            cache_start: 0,
            failed: false,
        };
        source.fetch()?;
        Ok(source)
    }

    pub fn content_length(&self) -> Option<u64> {
        self.length
    }

    fn fetch(&mut self) -> io::Result<()> {
        let result = self.fetch_inner();
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    fn fetch_inner(&mut self) -> io::Result<()> {
        // A byte at u64::MAX would require an unrepresentable next position.
        if self.position == u64::MAX {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "range position exceeds addressable stream",
            ));
        }
        let end = self
            .position
            .saturating_add(self.chunk_bytes as u64 - 1)
            .min(u64::MAX - 1);
        let mut request = self
            .agent
            .get(&self.url)
            .header("Range", format!("bytes={}-{}", self.position, end))
            .header("Accept-Encoding", "identity");
        if let Some(etag) = &self.etag {
            request = request.header("If-Match", etag.clone());
        }
        let mut response = request.call().map_err(transport_error)?;
        match response.status().as_u16() {
            206 | 416 => {}
            200 => return Err(unsupported("server ignored the byte Range request")),
            412 => return Err(invalid("HTTP representation changed (If-Match failed)")),
            300..=399 => {
                return Err(unsupported(
                    "range source requires a resolved URL without redirects",
                ));
            }
            status => {
                return Err(io::Error::other(format!(
                    "HTTP range request returned status {status}"
                )));
            }
        }
        if let Some(encoding) = one_header(response.headers(), "content-encoding")?
            && !encoding.as_bytes().eq_ignore_ascii_case(b"identity")
        {
            return Err(unsupported("HTTP range response is content-encoded"));
        }
        if let Some(content_type) = one_header(response.headers(), "content-type")? {
            let content_type = content_type
                .to_str()
                .map_err(|_| invalid("invalid Content-Type"))?;
            if content_type
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .eq_ignore_ascii_case("multipart/byteranges")
            {
                return Err(unsupported("multipart response to a single range request"));
            }
        }
        let etag = one_header(response.headers(), "etag")?
            .ok_or_else(|| unsupported("range source requires a strong ETag"))?;
        let bytes = etag.as_bytes();
        if bytes.len() < 2
            || bytes[0] != b'"'
            || bytes[bytes.len() - 1] != b'"'
            || !bytes[1..bytes.len() - 1]
                .iter()
                .all(|&b| b == 0x21 || (0x23..=0x7e).contains(&b) || b >= 0x80)
        {
            return Err(unsupported("range source requires a valid strong ETag"));
        }
        if self.etag.as_ref().is_some_and(|old| old != etag) {
            return Err(invalid("HTTP representation ETag changed"));
        }
        let tag = etag.clone();
        let range = one_header(response.headers(), "content-range")?
            .ok_or_else(|| invalid("missing Content-Range"))?
            .to_str()
            .map_err(|_| invalid("non-ASCII Content-Range"))?;
        let range = ContentRange::parse(range)?;
        if let Some(length) = range.total
            && self.length.is_some_and(|old| old != length)
        {
            return Err(invalid("HTTP representation length changed"));
        }
        if response.status().as_u16() == 416 {
            let total = range
                .total
                .ok_or_else(|| invalid("416 response needs a known complete length"))?;
            if range.interval.is_some() || self.position < total {
                return Err(invalid("unsatisfied range contradicts requested position"));
            }
            self.etag = Some(tag);
            self.length = Some(total);
            self.cache.clear();
            self.cache_start = self.position;
            return Ok(());
        }
        let (start, last) = range
            .interval
            .ok_or_else(|| invalid("206 response needs a satisfied byte range"))?;
        if start != self.position || last > end || last == u64::MAX {
            return Err(invalid("Content-Range does not match requested bytes"));
        }
        let length = usize::try_from(last - start + 1)
            .map_err(|_| invalid("range is too large for this platform"))?;
        if length > self.chunk_bytes {
            return Err(invalid("range exceeds configured chunk size"));
        }
        if self.length.is_some_and(|n| last >= n) {
            return Err(invalid("range exceeds previously known length"));
        }
        if let Some(value) = one_header(response.headers(), "content-length")?
            && decimal(
                value
                    .to_str()
                    .map_err(|_| invalid("invalid Content-Length"))?,
            )? != length as u64
        {
            return Err(invalid("Content-Length differs from Content-Range"));
        }
        // Reuse the one allocation, but never expose partial response bytes.
        self.cache.clear();
        self.cache.try_reserve_exact(length).map_err(|_| {
            io::Error::new(io::ErrorKind::OutOfMemory, "HTTP range allocation failed")
        })?;
        self.cache.resize(length, 0);
        let mut body = response.body_mut().as_reader();
        body.read_exact(&mut self.cache).map_err(body_error)?;
        let mut extra = [0];
        if body.read(&mut extra).map_err(body_error)? != 0 {
            return Err(invalid("HTTP range body exceeds declared interval"));
        }
        self.etag = Some(tag);
        self.length = range.total.or(self.length);
        self.cache_start = start;
        Ok(())
    }
}
impl Read for HttpRangeReader {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if self.failed {
            return Err(invalid(
                "HTTP range source cannot resume after a fetch error",
            ));
        }
        if output.is_empty() || self.length.is_some_and(|n| self.position >= n) {
            return Ok(0);
        }
        let cached = self
            .position
            .checked_sub(self.cache_start)
            .and_then(|n| usize::try_from(n).ok())
            .filter(|&n| n < self.cache.len());
        let offset = if let Some(offset) = cached {
            offset
        } else {
            self.fetch()?;
            if self.cache.is_empty() {
                return Ok(0);
            }
            0
        };
        let len = output.len().min(self.cache.len() - offset);
        output[..len].copy_from_slice(&self.cache[offset..offset + len]);
        self.position += len as u64;
        Ok(len)
    }
}
impl Seek for HttpRangeReader {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        if self.failed {
            return Err(invalid(
                "HTTP range source cannot resume after a fetch error",
            ));
        }
        let position =
            match from {
                SeekFrom::Start(n) => i128::from(n),
                SeekFrom::Current(n) => i128::from(self.position) + i128::from(n),
                SeekFrom::End(n) => {
                    i128::from(self.length.ok_or_else(|| {
                        unsupported("end-relative seeking requires a known length")
                    })?) + i128::from(n)
                }
            };
        let position = u64::try_from(position).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek position is outside u64 domain",
            )
        })?;
        self.position = position;
        Ok(position)
    }
}

fn one_header<'a>(headers: &'a HeaderMap, name: &str) -> io::Result<Option<&'a HeaderValue>> {
    let mut values = headers.get_all(name).iter();
    let first = values.next();
    if values.next().is_some() {
        return Err(invalid("duplicate range response metadata"));
    }
    Ok(first)
}
fn decimal(value: &str) -> io::Result<u64> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid("invalid decimal range value"));
    }
    value.parse().map_err(|_| invalid("range integer overflow"))
}
struct ContentRange {
    interval: Option<(u64, u64)>,
    total: Option<u64>,
}
impl ContentRange {
    fn parse(value: &str) -> io::Result<Self> {
        let (unit, value) = value
            .split_once(' ')
            .ok_or_else(|| invalid("invalid Content-Range syntax"))?;
        if !unit.eq_ignore_ascii_case("bytes") {
            return Err(unsupported("unsupported range unit"));
        }
        let (interval, total) = value
            .split_once('/')
            .ok_or_else(|| invalid("invalid Content-Range syntax"))?;
        let total = if total == "*" {
            None
        } else {
            Some(decimal(total)?)
        };
        let interval = if interval == "*" {
            None
        } else {
            let (first, last) = interval
                .split_once('-')
                .ok_or_else(|| invalid("invalid byte interval"))?;
            let (first, last) = (decimal(first)?, decimal(last)?);
            if first > last || total.is_some_and(|n| last >= n) {
                return Err(invalid("invalid byte interval bounds"));
            }
            Some((first, last))
        };
        if interval.is_none() && total.is_none() {
            return Err(invalid("unsatisfied range lacks total length"));
        }
        Ok(Self { interval, total })
    }
}
