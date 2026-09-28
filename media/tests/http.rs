#![cfg(feature = "http")]
use std::{
    io::{Read, Seek, SeekFrom, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use zencodec_media::http::HttpRangeReader;

struct Server {
    url: String,
    stop: Arc<AtomicBool>,
    requests: Arc<Mutex<Vec<String>>>,
    worker: Option<JoinHandle<()>>,
}
impl Server {
    fn new(handler: impl Fn(&str, usize) -> Vec<u8> + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let done = stop.clone();
        let observed = requests.clone();
        let worker = thread::spawn(move || {
            while let Ok((mut stream, _)) = listener.accept() {
                if done.load(Ordering::Relaxed) {
                    break;
                }
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                loop {
                    let mut byte = [0];
                    if stream.read(&mut byte).unwrap_or(0) == 0 {
                        break;
                    }
                    request.push(byte[0]);
                    assert!(request.len() < 16384);
                    if request.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                let request = String::from_utf8(request).unwrap();
                let n = {
                    let mut r = observed.lock().unwrap();
                    let n = r.len();
                    r.push(request.clone());
                    n
                };
                let response = handler(&request, n);
                for fragment in response.chunks(7) {
                    // Rejections can close without consuming a body.
                    if stream.write_all(fragment).is_err() {
                        break;
                    }
                }
            }
        });
        Self {
            url: format!("http://{address}/media"),
            stop,
            requests,
            worker: Some(worker),
        }
    }
    fn count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let address = self
            .url
            .strip_prefix("http://")
            .unwrap()
            .split('/')
            .next()
            .unwrap();
        let _ = TcpStream::connect(address);
        self.worker.take().unwrap().join().unwrap();
    }
}
fn response(status: &str, headers: &str, body: &[u8]) -> Vec<u8> {
    let mut out = format!("HTTP/1.1 {status}\r\nConnection: close\r\n{headers}\r\n").into_bytes();
    out.extend_from_slice(body);
    out
}
fn request_range(request: &str) -> (usize, usize) {
    let request = request.to_ascii_lowercase();
    assert!(request.contains("accept-encoding: identity\r\n"));
    let range = request
        .lines()
        .find_map(|s| s.strip_prefix("range: bytes="))
        .unwrap();
    let (start, end) = range.split_once('-').unwrap();
    (start.parse().unwrap(), end.parse().unwrap())
}
fn serve_bytes(bytes: &[u8], request: &str, known: bool) -> Vec<u8> {
    let (start, end) = request_range(request);
    if start >= bytes.len() {
        return response(
            "416 Range Not Satisfiable",
            &format!(
                "ETag: \"v1\"\r\nContent-Range: bytes */{}\r\nContent-Length: 0\r\n",
                bytes.len()
            ),
            &[],
        );
    }
    let end = end.min(bytes.len() - 1);
    let total = if known {
        bytes.len().to_string()
    } else {
        "*".to_owned()
    };
    response(
        "206 Partial Content",
        &format!(
            "ETag: \"v1\"\r\nContent-Range: bytes {start}-{end}/{total}\r\nContent-Length: {}\r\n",
            end - start + 1
        ),
        &bytes[start..=end],
    )
}
fn open(server: &Server, chunk: usize) -> HttpRangeReader {
    HttpRangeReader::open(&server.url, chunk, Duration::from_secs(2)).unwrap()
}

#[test]
fn actual_http_reads_are_bounded_cached_seekable_and_pinned() {
    let original: Vec<_> = (0..251).map(|x| x as u8).collect();
    let bytes = original.clone();
    let server = Server::new(move |request, n| {
        if n != 0 {
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("if-match: \"v1\"\r\n")
            );
        }
        let (start, end) = request_range(request);
        assert!(end - start < 17);
        serve_bytes(&bytes, request, true)
    });
    let mut reader = open(&server, 17);
    assert_eq!(reader.content_length(), Some(251));
    let mut a = [0; 9];
    reader.read_exact(&mut a).unwrap();
    assert_eq!(&a, &original[..9]);
    reader.seek(SeekFrom::Start(1)).unwrap();
    reader.read_exact(&mut a).unwrap();
    assert_eq!(&a, &original[1..10]);
    assert_eq!(server.count(), 1, "seek within chunk must hit cache");
    reader.seek(SeekFrom::End(-3)).unwrap();
    let mut tail = Vec::new();
    reader.read_to_end(&mut tail).unwrap();
    assert_eq!(&tail, &original[248..]);
    reader.seek(SeekFrom::Start(0)).unwrap();
    let mut all = Vec::new();
    reader.read_to_end(&mut all).unwrap();
    assert_eq!(all, original);
    reader.seek(SeekFrom::Start(900)).unwrap();
    assert_eq!(reader.read(&mut a).unwrap(), 0);
    assert!(reader.seek(SeekFrom::Current(-901)).is_err());
    assert_eq!(
        reader.stream_position().unwrap(),
        900,
        "invalid seek leaves position unchanged"
    );
}

#[test]
fn unknown_total_is_preserved_until_an_explicit_unsatisfied_range() {
    let server = Server::new(|request, _| serve_bytes(b"0123456789", request, false));
    let mut reader = open(&server, 4);
    assert_eq!(reader.content_length(), None);
    assert_eq!(
        reader.seek(SeekFrom::End(0)).unwrap_err().kind(),
        std::io::ErrorKind::Unsupported
    );
    let mut all = Vec::new();
    reader.read_to_end(&mut all).unwrap();
    assert_eq!(all, b"0123456789");
    assert_eq!(reader.content_length(), Some(10));
    reader.seek(SeekFrom::End(-1)).unwrap();
    let mut last = [0];
    reader.read_exact(&mut last).unwrap();
    assert_eq!(last, [b'9']);
}

#[test]
fn changed_or_malformed_responses_poison_without_publishing_partial_bytes() {
    let cases = vec![
        response("412 Precondition Failed", "Content-Length: 0\r\n", b""),
        response(
            "206 Partial Content",
            "ETag: \"v2\"\r\nContent-Range: bytes 4-7/10\r\nContent-Length: 4\r\n",
            b"4567",
        ),
        response(
            "206 Partial Content",
            "ETag: \"v1\"\r\nContent-Range: bytes 4-7/11\r\nContent-Length: 4\r\n",
            b"4567",
        ),
        response(
            "206 Partial Content",
            "ETag: \"v1\"\r\nContent-Range: bytes 3-6/10\r\nContent-Length: 4\r\n",
            b"3456",
        ),
        response(
            "206 Partial Content",
            "ETag: \"v1\"\r\nContent-Range: bytes 4-8/10\r\nContent-Length: 5\r\n",
            b"45678",
        ),
        response(
            "206 Partial Content",
            "ETag: \"v1\"\r\nContent-Range: bytes 4-7/10\r\nContent-Length: 4\r\n",
            b"45",
        ),
        response(
            "206 Partial Content",
            "ETag: \"v1\"\r\nContent-Range: bytes 4-7/10\r\nContent-Length: 3\r\n",
            b"456",
        ),
        response(
            "416 Range Not Satisfiable",
            "ETag: \"v1\"\r\nContent-Range: bytes */10\r\nContent-Length: 0\r\n",
            b"",
        ),
        response(
            "206 Partial Content",
            "ETag: \"v1\"\r\nContent-Range: bytes 4-7/10\r\nTransfer-Encoding: chunked\r\n",
            b"5\r\n45678\r\n0\r\n\r\n",
        ),
    ];
    for bad in cases {
        let server = Server::new(move |request, n| {
            if n == 0 {
                serve_bytes(b"0123456789", request, true)
            } else {
                bad.clone()
            }
        });
        let mut reader = open(&server, 4);
        reader.seek(SeekFrom::Start(4)).unwrap();
        let mut output = [99; 4];
        assert!(reader.read(&mut output).is_err());
        assert_eq!(output, [99; 4]);
        assert!(reader.seek(SeekFrom::Start(0)).is_err());
        assert!(reader.read(&mut output).is_err());
        assert_eq!(server.count(), 2, "poisoned source must not retry requests");
    }
}

#[test]
fn unsupported_servers_do_not_trigger_a_full_download() {
    let cases = vec![
        response("200 OK", "Content-Length: 1000000000\r\n", b""),
        response(
            "302 Found",
            "Location: http://127.0.0.1/other\r\nContent-Length: 0\r\n",
            b"",
        ),
        response(
            "206 Partial Content",
            "ETag: W/\"v1\"\r\nContent-Range: bytes 0-3/10\r\nContent-Length: 4\r\n",
            b"0123",
        ),
        response(
            "206 Partial Content",
            "Content-Range: bytes 0-3/10\r\nContent-Length: 4\r\n",
            b"0123",
        ),
        response(
            "206 Partial Content",
            "ETag: \"v1\"\r\nContent-Encoding: gzip\r\nContent-Range: bytes 0-3/10\r\nContent-Length: 4\r\n",
            b"0123",
        ),
        response(
            "206 Partial Content",
            "ETag: \"v1\"\r\nETag: \"v2\"\r\nContent-Range: bytes 0-3/10\r\nContent-Length: 4\r\n",
            b"0123",
        ),
        response(
            "206 Partial Content",
            "ETag: \"v1\"\r\nContent-Range: bytes 0-3/18446744073709551616\r\nContent-Length: 4\r\n",
            b"0123",
        ),
    ];
    for response in cases {
        let server = Server::new(move |_, _| response.clone());
        assert!(HttpRangeReader::open(&server.url, 4, Duration::from_secs(2)).is_err());
        assert_eq!(server.count(), 1);
    }
}

#[cfg(feature = "av1-decode")]
#[test]
fn native_timestamp_extraction_works_over_real_fragmented_http_ranges() {
    use zencodec_media::{
        av1_index::{FrameScope, FrameSelection, IndexedAv1},
        time::{TimeBase, Timestamp},
    };
    let bytes = include_bytes!("../corpus/av1/av1-12-444-narrow-17x13.ivf");
    let server = Server::new(move |request, _| serve_bytes(bytes, request, true));
    let reader = open(&server, 127);
    let mut settings = rav1d_safe::Settings::default();
    settings.all_layers = false;
    let mut indexed = IndexedAv1::build(reader, 1 << 20, 10, 4096, settings, None).unwrap();
    let clock = TimeBase::new(1001, 30000).unwrap();
    for n in [3, 0, 2, 1] {
        let found = indexed
            .extract(
                Timestamp::new(n, clock),
                FrameSelection::Nearest,
                FrameScope::All,
            )
            .unwrap()
            .unwrap();
        assert_eq!(found.frame().timestamp(), Some(Timestamp::new(n, clock)));
        let mapped = found.frame().map();
        let y = mapped.plane(0).unwrap().unwrap();
        assert_eq!(y.code(0, 0).unwrap(), if n % 2 == 0 { 256 } else { 3760 });
    }
}

#[test]
fn total_request_timeout_covers_response_body_reads() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let worker = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            stream.read_exact(&mut byte).unwrap();
            request.push(byte[0]);
        }
        stream.write_all(b"HTTP/1.1 206 Partial Content\r\nETag: \"slow\"\r\nContent-Range: bytes 0-3/4\r\nContent-Length: 4\r\nConnection: close\r\n\r\n").unwrap();
        thread::sleep(Duration::from_millis(250));
        let _ = stream.write_all(b"data");
    });
    let result = HttpRangeReader::open(
        &format!("http://{address}/slow"),
        4,
        Duration::from_millis(50),
    );
    assert_eq!(result.err().unwrap().kind(), std::io::ErrorKind::TimedOut);
    worker.join().unwrap();
}
