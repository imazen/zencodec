#!/usr/bin/env python3
"""Run native selection tests against deliberately faulty HTTP responses.

Adapted from imazen/zqi f7652c97119c936b53d739098e2eab789a42592a,
tools/codec-speed/http_range_study.py (Imazen-owned code).
Protocol validation, not an HTTP performance benchmark.
"""
import argparse
import hashlib
import json
import os
import re
import subprocess
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlsplit


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("artifacts", type=Path)
    parser.add_argument("port", type=int)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if not 3000 <= args.port < 4000:
        parser.error("test port must be in 3000..3999")
    args.artifacts.mkdir(parents=True, exist_ok=False)
    root = args.artifacts.resolve()
    guard = threading.Lock()
    requests = []
    counts = {}
    modes = {"stable", "change", "changed_tag", "weak", "missing_tag", "bad_tag",
             "wrong_range", "wrong_total", "encoding", "full_body", "short_body",
             "wrong_length", "missing_length", "duplicate_tag", "empty_tag", "redirect"}

    class Handler(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, *_args):
            pass

        def do_GET(self):
            parsed = urlsplit(self.path)
            parts = parsed.path.strip("/").split("/")
            if len(parts) != 2 or parts[0] not in modes or not re.fullmatch(r"[a-zA-Z0-9_-]+\.tif", parts[1]):
                self.send_error(404)
                return
            mode, filename = parts
            path = root / filename
            if not path.is_file() or path.stat().st_size > 2 * 1024 * 1024:
                self.send_error(404)
                return
            data = path.read_bytes()
            etag = '"' + hashlib.sha256(data).hexdigest() + '"'
            match = re.fullmatch(r"bytes=(\d+)-(\d+)", self.headers.get("Range", ""))
            if match is None or self.headers.get("Accept-Encoding") != "identity":
                self.send_error(400)
                return
            start, end = map(int, match.groups())
            if not 0 <= start <= end < len(data):
                self.send_error(416)
                return
            with guard:
                call = counts.get(self.path, 0) + 1
                counts[self.path] = call
            fault_at = int(parse_qs(parsed.query).get("at", ["2"])[0])
            active = mode != "stable" and call >= fault_at
            if mode == "empty_tag":
                etag = '""'
            if active and mode in {"change", "changed_tag"}:
                data = bytes(b ^ 0x5a for b in data)
                etag = '"' + hashlib.sha256(data).hexdigest() + '"'
            status = 206
            if self.headers.get("If-Match") not in (None, etag) and mode != "changed_tag":
                status = 412
            if active and mode == "full_body":
                status = 200
            if active and mode == "redirect":
                status = 302
            body = data[start:end + 1] if status == 206 else b""
            length = len(body)
            content_range = f"bytes {start}-{end}/{len(data)}"
            if active:
                if mode == "wrong_range":
                    content_range = f"bytes {start + 1}-{end + 1}/{len(data)}"
                if mode == "wrong_total":
                    content_range = f"bytes {start}-{end}/{len(data) + 1}"
                if mode == "wrong_length":
                    length += 1
                if mode == "short_body":
                    body = body[:-1]
                if mode == "weak":
                    etag = "W/" + etag
                if mode == "bad_tag":
                    etag = '"bad"tag"'
            with guard:
                requests.append({"path": self.path, "call": call, "status": status,
                                 "range": [start, end], "if_match": self.headers.get("If-Match"),
                                 "response_tag": etag, "body_bytes": len(body)})
            self.send_response(status)
            self.send_header("Content-Range", content_range)
            if not (active and mode == "missing_length"):
                self.send_header("Content-Length", str(length))
            if not (active and mode == "missing_tag"):
                self.send_header("ETag", etag)
            if active and mode == "duplicate_tag":
                self.send_header("ETag", '"second"')
            if active and mode == "encoding":
                self.send_header("Content-Encoding", "gzip")
            if status == 302:
                self.send_header("Location", "/stable/redirect.tif")
            self.send_header("Connection", "close")
            self.end_headers()
            try:
                self.wfile.write(body)
            except (BrokenPipeError, ConnectionResetError):
                # Expected when malformed headers are rejected before body I/O.
                pass

    server = ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    worker = threading.Thread(target=server.serve_forever, daemon=True)
    worker.start()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    environment = dict(os.environ, SPATIAL_HTTP_URL=f"http://127.0.0.1:{args.port}",
                       SPATIAL_HTTP_ARTIFACTS=str(root))
    try:
        completed = subprocess.run(command, env=environment, timeout=240, check=False)
    finally:
        server.shutdown()
        worker.join(timeout=5)
        server.server_close()
        (root / "requests.json").write_text(json.dumps(requests, indent=2) + "\n")
    if completed.returncode != 0:
        raise SystemExit(completed.returncode)
    assert requests, "native tests performed no HTTP requests"
    assert all((r["if_match"] is None) == (r["call"] == 1) for r in requests), "conditional request omitted"
    summary = {"scope": "Native TIFF selection over local HTTP/1.1; no WAN/TLS/performance claim",
               "requests": len(requests), "cases": len(counts),
               "statuses": {str(s): sum(r["status"] == s for r in requests)
                            for s in sorted({r["status"] for r in requests})},
               "conditional_after_open": True}
    (root / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary))


if __name__ == "__main__":
    main()
