//! Compare real container events across chunked async random and forward I/O.
#![forbid(unsafe_code)]
use zencodec::how_far::NoPulse;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use zencodec::decode::BoxedError;
use zencodec_io::tokio_io::{AsyncCursor, AsyncReadSource, AsyncSource};
use zencodec_io::{DriveOptions, SliceCursor};
use zencodec_media::container::{DemuxEvent, DemuxStage, Demuxer};
use zencodec_media::limits::MediaLimits;

struct Ranges {
    data: Arc<[u8]>,
    chunk: usize,
    reads: usize,
    bytes: usize,
}
impl AsyncSource for Ranges {
    async fn len(&mut self) -> Result<u64, BoxedError> {
        Ok(self.data.len() as u64)
    }
    async fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<usize, BoxedError> {
        tokio::task::yield_now().await;
        self.reads += 1;
        let at = usize::try_from(offset)?;
        let n = out
            .len()
            .min(self.chunk)
            .min(self.data.len().saturating_sub(at));
        if n > 0 {
            out[..n].copy_from_slice(&self.data[at..at + n]);
        }
        self.bytes += n;
        Ok(n)
    }
}
struct Fragments {
    data: Arc<[u8]>,
    at: usize,
    chunk: usize,
    pending: bool,
}
impl tokio::io::AsyncRead for Fragments {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        out: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if !self.pending {
            self.pending = true;
            cx.waker().wake_by_ref();
            return Poll::Pending;
        }
        self.pending = false;
        let n = out
            .remaining()
            .min(self.chunk)
            .min(self.data.len() - self.at);
        out.put_slice(&self.data[self.at..self.at + n]);
        self.at += n;
        Poll::Ready(Ok(()))
    }
}
fn record<D: Demuxer>(d: &D, e: DemuxEvent, out: &mut Vec<String>) {
    match e {
        DemuxEvent::Headers => out.push(format!("tracks {:?}", d.tracks())),
        DemuxEvent::Packet(p) => out.push(format!("{p:?}")),
        other => out.push(format!("{other:?}")),
    }
}
async fn from_async<D: Demuxer, S: AsyncSource>(
    mut dem: D,
    src: &mut S,
    size: usize,
) -> Result<Vec<String>, String> {
    let mut cur = AsyncCursor::new(DriveOptions::new(0, size));
    let mut events = Vec::new();
    loop {
        let y = cur
            .next(src, &mut DemuxStage(&mut dem), &NoPulse)
            .await
            .map_err(|e| format!("{e:?}"))?;
        let done = y.is_finished();
        record(&dem, y.into_inner(), &mut events);
        if done {
            return Ok(events);
        }
    }
}
async fn check<D: Demuxer>(name: &str, data: Arc<[u8]>, make: impl Fn() -> D) {
    let mut d = make();
    let mut cur = SliceCursor::new(65536);
    let mut expected = Vec::new();
    loop {
        let y = cur
            .next(&data, &mut DemuxStage(&mut d), &NoPulse)
            .unwrap();
        let done = y.is_finished();
        record(&d, y.into_inner(), &mut expected);
        if done {
            break;
        }
    }
    for size in [1, 7, 1024, 65536] {
        let mut source = Ranges {
            data: data.clone(),
            chunk: size,
            reads: 0,
            bytes: 0,
        };
        let got = from_async(make(), &mut source, size).await.unwrap();
        assert_eq!(got, expected, "{name} random chunk={size}");
        println!(
            "{name} random chunk={size} reads={} bytes={} events={} parity=true",
            source.reads,
            source.bytes,
            got.len()
        );
        let stream = Fragments {
            data: data.clone(),
            at: 0,
            chunk: size,
            pending: false,
        };
        let mut source = AsyncReadSource::new(stream);
        match from_async(make(), &mut source, size).await {
            Ok(got) => {
                assert_eq!(got, expected, "{name} forward chunk={size}");
                println!("{name} forward chunk={size} parity=true");
            }
            Err(e) => println!("{name} forward chunk={size} refused={e}"),
        }
    }
}
#[tokio::main(flavor = "current_thread")]
async fn main() {
    for name in std::env::args().skip(1) {
        let data: Arc<[u8]> = std::fs::read(&name).unwrap().into();
        let limits = MediaLimits::default();
        match std::path::Path::new(&name)
            .extension()
            .and_then(|s| s.to_str())
            .unwrap()
        {
            "mp4" => check(&name, data, || zenmp4::Mp4Demuxer::new(limits)).await,
            "webm" | "mkv" => check(&name, data, || zenwebm::WebmDemuxer::new(limits)).await,
            "ivf" => check(&name, data, || zenivf::IvfDemuxer::new(limits)).await,
            other => panic!("unsupported extension: {other}"),
        }
    }
}
