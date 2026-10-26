//! Strict single-object HTTP source for native-codec contract tests.
//! Adapted from imazen/zqi f7652c97119c936b53d739098e2eab789a42592a,
//! tools/codec-speed/src/bin/iqa_http_access.rs (Imazen-owned code).
//! Protocol validation stays outside the codec and the public ReadAt trait.
use std::{cell::Cell, io::Read, rc::Rc, sync::Arc, time::Duration};
use zencodec::source::ReadAt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailureKind {
    Bounds,
    RangeLimit,
    Status(u16),
    ContentRange,
    ContentLength,
    Encoding,
    Validator,
    Transport,
    Body,
}
#[derive(Clone, Debug)]
pub struct Failure {
    pub kind: FailureKind,
    cause: Option<Arc<dyn std::error::Error + Send + Sync>>,
}
impl Failure {
    fn new(kind: FailureKind) -> Self {
        Self { kind, cause: None }
    }
    fn caused(kind: FailureKind, cause: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self {
            kind,
            cause: Some(Arc::new(cause)),
        }
    }
}
impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HTTP snapshot {:?}", self.kind)
    }
}
impl std::error::Error for Failure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.cause.as_ref().map(|e| e.as_ref() as _)
    }
}

pub struct HttpSource {
    agent: ureq::Agent,
    url: String,
    length: Option<u64>,
    etag: Option<String>,
    max_range: usize,
    failed: Option<Failure>,
    pub attempts: Rc<Cell<u64>>,
}
impl HttpSource {
    pub fn open(url: String, max_range: usize) -> Result<Self, Failure> {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(5)))
            .max_redirects(0)
            .http_status_as_error(false)
            .proxy(None)
            .build()
            .into();
        let mut source = Self {
            agent,
            url,
            length: None,
            etag: None,
            max_range,
            failed: None,
            attempts: Rc::new(Cell::new(0)),
        };
        source.read_exact_at(0, &mut [0])?;
        Ok(source)
    }

    fn fetch(&mut self, offset: u64, out: &mut [u8]) -> Result<(), Failure> {
        if out.is_empty() {
            return Ok(());
        }
        if out.len() > self.max_range {
            return Err(Failure::new(FailureKind::RangeLimit));
        }
        let end = offset
            .checked_add(out.len() as u64 - 1)
            .filter(|&end| self.length.is_none_or(|len| end < len))
            .ok_or_else(|| Failure::new(FailureKind::Bounds))?;
        let range = format!("bytes={offset}-{end}");
        let mut request = self
            .agent
            .get(&self.url)
            .header("Range", &range)
            .header("Accept-Encoding", "identity");
        if let Some(tag) = &self.etag {
            request = request.header("If-Match", tag);
        }
        self.attempts.set(self.attempts.get() + 1);
        let mut response = request
            .call()
            .map_err(|e| Failure::caused(FailureKind::Transport, e))?;
        let status = response.status().as_u16();
        if status != 206 {
            return Err(Failure::new(FailureKind::Status(status)));
        }
        let headers = response.headers();
        let field = |name: &str, kind| -> Result<&str, Failure> {
            if headers.get_all(name).iter().count() != 1 {
                return Err(Failure::new(kind));
            }
            headers
                .get(name)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| Failure::new(kind))
        };
        if headers.contains_key("Content-Encoding") {
            return Err(Failure::new(FailureKind::Encoding));
        }
        let count = field("Content-Length", FailureKind::ContentLength)?
            .parse::<usize>()
            .ok();
        if count != Some(out.len()) {
            return Err(Failure::new(FailureKind::ContentLength));
        }
        let prefix = format!("bytes {offset}-{end}/");
        let total = field("Content-Range", FailureKind::ContentRange)?
            .strip_prefix(&prefix)
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|&total| total > end && self.length.is_none_or(|old| old == total))
            .ok_or_else(|| Failure::new(FailureKind::ContentRange))?;
        let tag = field("ETag", FailureKind::Validator)?;
        // One strong entity tag; opaque content may be empty. No wildcard/list.
        if tag.len() < 2
            || !tag.starts_with('"')
            || !tag.ends_with('"')
            || !tag.as_bytes()[1..tag.len() - 1]
                .iter()
                .all(|&c| c == 0x21 || (0x23..=0x7e).contains(&c))
            || self.etag.as_ref().is_some_and(|prior| prior != tag)
        {
            return Err(Failure::new(FailureKind::Validator));
        }
        let tag = tag.to_owned();
        let mut body = response.body_mut().as_reader();
        body.read_exact(out)
            .map_err(|e| Failure::caused(FailureKind::Body, e))?;
        if body
            .read(&mut [0])
            .map_err(|e| Failure::caused(FailureKind::Body, e))?
            != 0
        {
            return Err(Failure::new(FailureKind::ContentLength));
        }
        // Commit representation facts only after the complete response succeeds.
        self.length = Some(total);
        self.etag = Some(tag);
        Ok(())
    }
}
impl ReadAt for HttpSource {
    type Error = Failure;
    fn len(&mut self) -> Result<u64, Failure> {
        if let Some(error) = &self.failed {
            return Err(error.clone());
        }
        self.length.ok_or_else(|| Failure::new(FailureKind::Bounds))
    }
    fn len_hint(&self) -> Option<u64> {
        self.failed.is_none().then_some(self.length).flatten()
    }
    fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), Failure> {
        if let Some(error) = &self.failed {
            return Err(error.clone());
        }
        let result = self.fetch(offset, out);
        if let Err(error) = &result {
            // No implicit reopen or adoption of a new validator after a failure.
            self.failed = Some(error.clone());
        }
        result
    }
}
