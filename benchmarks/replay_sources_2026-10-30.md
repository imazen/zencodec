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

## Unknown-length bounded spool follow-through

Snapshot 52e661a6164b8ba4bf969036c4b41b29baf45d78 adds the existing
BufferedReadAt + BufferPolicy::Spool to the native acquisition controls. The
reader supplies seven-byte short reads. Zero and one-byte-short caps refuse;
exact and larger caps confirm EOF and replay native AV1 samples twice unchanged.
The cap failure consumes the documented one-byte EOF probe and then remains
terminal without further reads. Snapshotting after EOF makes no additional
underlying reads. A typed timeout from the reader survives the spool error chain;
no replay snapshot is created from that failed acquisition.

The passing controls observe both source.retained_bytes() and
budget.reserved_bytes() equal to the input payload length during snapshot copy.
They then drop the spool and prove the admitted snapshot still decodes correctly.
This is a measured logical-retention overlap, not an RSS or allocator-capacity
claim. The spool's private Vec cannot currently transfer directly into accounted
SharedStorage. Removing that overlap needs an ownership/admission design, not a
claim that two independently capped buffers share one operation allowance.

Both full source tests and strict target Clippy pass with unchanged lock and
HTTP cases. Wrapper rc0, 8 seconds, peak RSS 0.39 GiB, minimum available 13246 MiB,
peak load 0.55. Original test assertions remain. Log on r7900x:
/home/lilith/tmp/zencodec-replay-source-20261030/unknown.log.
HTTP evidence: replay-source-http-unknown under the same artifact root.
This supersedes the earlier unknown-length qualification gap only for the
bounded full-spool-plus-copy route. It does not qualify streaming seek/reopen,
zero-copy transfer, stalled-read preemption or whole-operation RAM admission.

## Single-payload alternative

Snapshot b27ceff8daea2f10988f4dee6e490e8bf7e26385 adds a private alternative:
reserve the chosen cap with StorageBudget before the first read, fill its
OwnedStorage directly, freeze it and return only the initialized input prefix.
The payload pointer is asserted unchanged across freezing; one managed object
remains charged. The small shared-owner header is outside that payload allowance.
This is not a claim of one allocation for the entire operation.

Native AV1 replay passes for exact and overprovisioned caps. The returned view
has the actual input length, while charged_bytes and budget reservations retain
the full chosen capacity. A one-byte-short cap refuses after its EOF probe and
releases the reservation. Byte and object admission failures precede all input
consumption. Empty input with zero byte allowance succeeds with one object slot.
A reader returning Interrupted while raising cancellation is polled before retry:
only one native read occurs, the typed timeout survives and storage is released.
Malformed read counts refuse rather than indexing outside admitted storage.

All three complete source tests and strict target Clippy pass. Original HTTP,
file and spool controls remain enabled. Isolated export, same committed lock,
explicit package clean. Wrapper rc0, 7 seconds, peak RSS 0.39 GiB,
minimum available 13441 MiB, peak load 0.33. Log:
/home/lilith/tmp/zencodec-replay-source-20261030/reserved.log on r7900x.
HTTP evidence: replay-source-http-reserved under the same artifact root.

This avoids the spool/snapshot payload overlap by choosing eager reservation,
not by transferring or retrospectively charging an existing Vec. It reserves
and initializes unused capacity and keeps that capacity charged until the last
view drops. The source may have consumed a prefix on failure and must not be
silently retried as a new object. A blocking read still needs transport/native
cancellation support. No production API or whole-operation memory claim is added.
