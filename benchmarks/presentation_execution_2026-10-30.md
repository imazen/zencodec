# Native execution of presentation edits

## Result and scope

All twelve cases pass independent FFmpeg/libdav1d pixel and timestamp comparison:
identity, clip, repeat, explicit black gap, dwell and double rate, each in
progressive and fragmented MP4. This executes edits into the encoded samples;
it does not depend on the player's edit-list execution. The earlier retained-edit
matrix remains unchanged and has seven mismatches; see
[presentation playback](presentation_playback_2026-10-30.md).

The native fixture decodes three AV1 frames, projects their presentation intervals
using the private cursor, re-encodes losslessly and writes TrackTimeline::Unedited.
It verifies unchanged shared picture ownership during projection and exact native
packet PTS/durations before muxing. The output gap policy explicitly inserts black.
No independent software encodes or rewrites the files.

This is a bounded fixture, not a production streaming implementation. It retains
all three decoded frames and projected references. Final source duration is supplied
by the fixture. Reverse playback, unbounded streams, unknown duration, replay
admission, cancellation, audio synchronization and operation-wide resource admission
are not qualified here. A successful witness does not settle those contracts.

## Reproduce

```sh
cp perf/av1-policy-validation/dependencies.lock perf/av1-policy-validation/Cargo.lock
just presentation-baked NEW_FILES EXPECTED_RAW NEW_RESULTS
```

EXPECTED_RAW is the unchanged native fixture documented in the earlier playback
receipt, SHA256 7894797ccaac0d7c4fb904f4e4bd1f182ab09070b0e1adc8975eea57254e8c12.
The oracle's explicit --baked-black-gaps mode adds one zero-valued GBR frame;
all other pixel, PTS and end-time expectations are identical to the retained-edit
comparison. Without the flag, that original comparison remains unchanged.

## Qualification

Native source export 610512dbeecb1abb11ec8afbc7cdb9918510f6aa, outside workspace
patches. Starting from committed dependencies.lock, resolution changed only
zenmp4 to 0070de3f701edbd25c592bdf180f933927cfcdcd; the exact lock is committed.
Native generation and strict example Clippy pass under a 3 GiB/two-job cap:
rc0, 36 seconds, peak RSS 0.97 GiB, minimum available 12624 MiB, peak load 1.22.
Independent FFmpeg 8.0.1 comparison: rc0, 1 second, peak RSS 0.05 GiB,
minimum available 13459 MiB, peak load 1.10. These are validation resource
receipts, not codec performance or memory-budget measurements.

Both wrappers exited; no Cargo/rustc processes remained. Full logs:
/home/lilith/tmp/zencodec-baked-20261030/{run,oracle}.log on r7900x.
Retained files, raw decoder outputs, probe facts, hashes and tool versions:
/mnt/v/output/zencodec/presentation-playback-20261030/{baked-files,baked-oracle}.
