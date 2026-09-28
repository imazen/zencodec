# Timestamp extraction and network input

`IndexedAv1<R: Read + Seek>` builds an IVF presentation index by decoding the
source once. IVF has no built-in seek table. Construction retains packet
locations, exact PTS, presentation order, keyframe provenance and sequence
headers; decoded pictures are released. Packet/presentation counts, retained
sequence bytes, packet size and decoder frame area have separate limits.

Queries select nearest, at-or-before or at-or-after frames, optionally among
visible keyframes only. Comparison uses integer rational arithmetic, including
negative PTS and mixed clocks. Nearest ties choose the earlier PTS, then the
first presentation among duplicates. Nonmonotonic timestamp sequences are
sorted only for selection; decoding still follows their original order.
Directional queries outside the stream return no frame. Nearest selects an
endpoint. The result includes actual PTS and original presentation ordinal.

Extraction creates fresh decoder state, provides the sequence header and starts
from a preceding visible key picture's packet. A picture flag alone does not
prove that every coded picture in its temporal unit is independent. If starting
there produces an invalid-data codec error, extraction retries from the beginning.
Cancellation and allocation failures are not converted into retries. The returned
packet count includes an unsuccessful attempt. This adapter rejects multiple
visible presentations in one IVF packet and multilayer/invisible output settings.

The source must remain unchanged after indexing. The index owns its reader and
checks packet locations, lengths, times and selected presentation metadata on
reuse. Those checks are not a content hash for an externally modified local file.
HTTP input pins representation identity as described below. For one sequential
pass, `Av1IvfDecoder<R: Read>` avoids index construction and needs no seeking.

## HTTP range source

With the `http` feature, `HttpRangeReader::open(url, chunk_bytes, timeout)` supplies
`Read + Seek` over blocking HTTP(S). It requests one byte range at a time, caches
one chunk, and uses a positive total timeout for each request including its body.
It sends `Accept-Encoding: identity`, requires a strong ETag, and sends `If-Match`
on subsequent requests. It validates range boundaries, complete length, body
length and validator before returning response bytes. Partial fetches, changed
representations and malformed responses poison the source.

Unknown total length remains unknown until the server supplies it. Reading past
an unknown end requires a valid 416 response with the complete length; seeking
relative to the end is unavailable while length is unknown. Short valid 206
ranges are consumed and the remainder requested later. Seeking beyond a known
end is allowed and reads return EOF, matching ordinary seekable-file behavior.

Resolved URLs with range support and strong ETags are required. Redirects,
ignored Range requests, weak/missing validators, multipart responses and content
coding are explicit unsupported errors. This implementation does not silently
buffer a whole download as a substitute for seeking. Applications needing that
fallback must choose and budget a separate spool operation. A CPU cancellation
token cannot interrupt arbitrary blocking I/O; the transport timeout bounds it.

The contract follows [RFC 9110 sections 13–15](https://www.rfc-editor.org/rfc/rfc9110.html#section-13.1.1):
If-Match prevents a changed representation from replacing the indexed bytes;
If-Range would permit a whole new representation, which this adapter does not
want. The implementation was checked against ureq 3.4.2 source, upstream commit
`8fcd72a7881354400c432157e1a60222d61efc5c` (`config.rs`, `response.rs`, `body/mod.rs`,
`error.rs`), and the version locked in Cargo.lock.

Tests use an actual local HTTP server with fragmented writes, changed ETags,
incorrect ranges/lengths, truncated and excessive bodies, unknown length, rejected
servers, and stalled response bodies. The complete native 12-bit decoding/index/
extraction path also runs through that range reader. File-backed extraction
compares every code across all 48 native AV1 cases, covers reordered show-existing
presentations, and tests sequence initialization missing from a later key packet.
