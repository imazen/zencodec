# Native replay source identity and snapshot admission

## Contract evidence

The private witness uses existing ReadAt, StorageBudget, SharedStorage and
Packet::from_accounted. No source-version method or public trait was added.
Known total length permits allocation admission before body reads. The source
must keep its representation coherent while copying; freezing makes subsequent
replay independent of external changes. Unknown length currently refuses.

Native AV1 packets are concatenated into a binary fixture; known packet spans
become shared accounted views after acquisition. This is not a container parser.
The same native configuration decodes the admitted snapshot twice, matching every
source sample. File acquisition uses FileReadAt while the fixture remains stable;
the original file is then overwritten after closing that source, and replay still
matches. There is no claim of filesystem snapshot isolation during copying.

The existing spatial HTTP source and simulator are reused unchanged in their
validation logic. The simulator now also admits .bin fixture names and accepts an
explicit scope label; TIFF remains the default. The passing run records 21 real
local HTTP requests across five routes, 20 responses with status 206 and one 412.
Every request after opening carries If-Match. No WAN/TLS/performance claim.

Controls preserve these assertions:
- Insufficient snapshot bytes refuse before another HTTP body request.
- Dropping the first clone retains the complete allocation charge; dropping the
  last clone returns byte and object reservations to zero.
- Replacement returns Status(412); a changed tag returns Validator; changed total
  returns ContentRange; truncated body returns Body. All retain typed failures.
- Failed HTTP sources do not silently reopen or make another network request.
- A stop raised by the first successful read returns typed TimedOut, releases the
  partial allocation, and permits no next read. Pre-cancelled entry does no read.

## Qualification

Frozen source 9e4978e8683527b3065d946217bd359ac69ac7a8: both complete test controls
and strict target Clippy pass. The two simulator supervision tests also pass.
3 GiB/two-job cap, 900-second outer timeout, simulator's existing 240-second
child timeout retained. Wrapper rc0, 7 seconds, peak RSS 0.39 GiB,
minimum available 13317 MiB, peak load 0.32. This is test resource usage,
not a memory-bound or throughput measurement. All processes exited.

Two preceding fixture compilation failures are retained: StopReason does not
implement Error, so the fix uses the existing typed SessionError::Cancelled;
borrowed slices need SliceReadAt rather than implementing ReadAt directly.
Neither failure ran native assertions. No established assertions were relaxed.

The isolated locked graph adds ureq 3.5.0 and eight transitive package identities
(base64, bytes, http, httparse, itoa, percent-encoding, ureq-proto, utf8-zero).
Only the standalone harness's existing dependency list changes; no prior package
identity/version is removed or changed. No production dependency gains HTTP.
The final committed lock is the qualified one. Harness compilation was forced
with a package clean and its source path verified.

Logs on r7900x: /home/lilith/tmp/zencodec-replay-source-20261030/
{run,fixed,slice}.log, simulator-tests.log, and graph.json.
HTTP request evidence and native fixture:
/mnt/v/output/zencodec/presentation-playback-20261030/replay-source-http-slice/.
Reproduce with the standalone dependencies.lock copied to Cargo.lock and
`just presentation-replay-sources NEW_ARTIFACTS 3586`.

Still open: bounded unknown-length acquisition, conditional reacquisition
without a full compressed snapshot, keyframe seeking, whole-operation memory
admission, and late-metadata obligations during early timeline completion.
The private snapshot helper is evidence for composing existing abstractions,
not a promoted production convenience API.
