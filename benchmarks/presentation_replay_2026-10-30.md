# Native presentation replay without a decoded-frame vector

The bounded native witness now offers --replay-source. Its compressed source
packets remain retained. Each media segment creates its own native AV1 decoder;
each decoded frame is projected and sent directly through the shared encoder
pump. Projection shares the picture; the driver stores no decoded-frame vector.
Weak picture references verify all decoded pictures are released after the
native decoder/encoder owners are dropped. The black-gap fixture has its own
explicit owner and is not counted as decoded source data.

The original generated frames and baseline decoded vector are dropped before
execution in this mode. The encoder may retain native storage; compressed output
packets and timing records are accumulated by this bounded test. No total memory
bound or constant-memory production claim follows.

Snapshot 715b50bf qualified complete-source replay: all twelve independent
pixel/timestamp comparisons passed and all twelve containers were byte-identical
to the buffered baseline. Wrapper rc0, 7 seconds, peak RSS 0.39 GiB,
minimum available 13434 MiB, peak load 0.21.

Snapshot 17c2b9c5 additionally stops acquiring packets after finite coverage and
skips native draining when that presentation is already complete. It still
passes the same twelve independent comparisons and byte-identical output checks,
plus all eleven cursor tests and strict Clippy for the example and experiment.
The cursor reports incomplete for open-ended presentations and after terminal
closure. Source metadata obligations are not inferred from frame coverage; this
fixture alone has a known generated source with no remaining inspection duty.
Wrapper rc0, 11 seconds, peak RSS 0.40 GiB, minimum available 13346 MiB,
peak load 0.40. These are test-run resource receipts, not performance measurements.

Both runs use isolated exports, committed locks, explicit harness package clean
and verified compilation paths, 3 GiB/two-job caps and 900-second timeouts.
Full logs on r7900x: /home/lilith/tmp/zencodec-replay-20261030/{run,finite}.log.
Retained artifacts: /mnt/v/output/zencodec/presentation-playback-20261030/
{replay-files,replay-oracle,finite-replay-files,finite-replay-oracle}.
The buffered comparison is pump-files in that same directory. The independent
raw reference and toolchain are those in presentation_execution_2026-10-30.md.
All wrapper processes exited before subsequent work.

Reproduction: copy the two standalone dependencies.lock files to Cargo.lock,
then run `just presentation-replay NEW_FILES EXPECTED_RAW NEW_RESULTS`.
No public source/replay abstraction is promoted by this experiment. Network/file
snapshot identity, admission before effects, keyframe seeking, dynamic metadata,
production cancellation and operation-wide memory accounting remain requirements.
