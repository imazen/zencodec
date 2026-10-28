# zencodec-media

Shared packets, tracks, frame ownership, codec sessions and sans-I/O container
traits. `no_std + alloc` by default; this review crate is not yet published.

With `std`, `staged::StagedEncode::new(encoder, muxer, destination, resolver,
storage, stop)` provides finite video encoding: push frames, then finish. It
holds conversion through policy and stages compressed payloads, releasing the
encoder before the first final-destination write. It retains decision history on success
and failure. The managed payload budget excludes native workspace and
caller-owned pictures; this is not a live streaming operation.
Explicit `with_staging(..., Staging::disk(file, limits, replay_budget)?, stop)`
uses caller-owned private scratch with disk/index caps and admitted replay.
Spool writes can block while the encoder is alive; policy sees the chosen store.
The caller controls scratch permissions and deletion. Replay failures retain
confirmed destination appends, including when some output already escaped.

For existing encoded packets, `output` provides checked synchronous output:

- `Output::new(muxer, Forward(writer), tracks, stop)` prepares and publishes,
  requiring exact packet timing. Send packets, then consume it with `finish`.
- `PreparedOutput::new(...)` retains preparation without writing; `publish`
  consumes it after the caller's policy decision.
- `SeekBack::new(writer)` captures the current file position as the output
  origin. Earlier file contents remain outside relative finish patches.
- `Output::with_container_timing(...)` explicitly opts into ordinary native
  timing instead of requiring exact supplied timestamps and durations.

Errors retain the original cause, stage and confirmed append progress. Packet
errors are terminal. Final patches are preflighted, but an I/O failure can leave
partial effects. Dropping output never implicitly publishes or finishes it.
These drivers do not establish safe live encoder backpressure or whole-operation
memory bounds.

[Contract](../docs/media-contract.md) ·
[Native validation](../perf/av1-policy-validation/README.md#shared-public-output-driver)
