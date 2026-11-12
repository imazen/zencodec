# Synthetic driver mechanism trial

This reuses the existing SYN1 RGBA8 fixture and older decoder/coroutine driver
with a translated decoder for the reimagined contract. Source adapted from
zencodec `05b73f5f4e26a45c4bb4f584b590831117bb36a2`. It compares the same row-copy
kernel across every arm in `ARMS`, with optional Pulse disabled. This is a driver
mechanism diagnostic, not evidence about native entropy decoding or complete
codec semantics.

`qualify_file` checks exact active samples, all final rows, completion and
untouched output padding for tightly packed and 16-byte-padded rows. Unit
controls retain the earlier non-square, small, one-row-strip and quota cases.

From `reimagined/`, use `probe-check` and `probe-build` with `PROBE_TARGET`
pointing to an isolated output directory. `allocations fixture SIZE FILE` saves
one fixture; `allocations qualify 0 @FILE STRIP QUOTA` checks every route;
`allocations ARM COUNT @FILE STRIP QUOTA` warms once and performs COUNT extra
decodes. Fixture generation and warm correctness checks are outside the repeated
operation. The large-study controller lives in `../large-decode/`.
