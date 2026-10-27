# Native JPEG policy qualification

Private experiments against zenjpeg main `a2a128ab0bc36f01338225f61c285f462ae67ca6`.
These do not add a public policy API. QOI remains a simple control in the I/O
harness; JPEG exercises metadata placement, progressive scans, output timing,
orientation, rewriting and refusal before publication.

From a checkout outside the zen workspace path overrides, install `djpeg`
(`libjpeg-turbo-progs` on Ubuntu), then run:

```sh
just jpeg-policy-setup
just jpeg-policy-check
```

Use `just jpeg-policy-sync <export>` to update an existing source export.
The standalone manifest pins zenjpeg's unpublished dependency bridges. Its
`dependencies.lock` is copied to Cargo.lock by setup; no source in the local
zenjpeg checkout is modified. Native JPEG is a separate workspace so its newer
pixel dependencies do not alter the original I/O experiments.

[Evidence and limits](../../docs/policy-jpeg.md). Tests fail when the independent
`djpeg` oracle is missing. Fixtures are generated with the actual encoder and
metadata serializers; there are no committed image binaries. The recipe supplies
`JPEG_POLICY_ARTIFACTS` for independent ICC extraction; direct Cargo invocations
must set it explicitly to a disk directory.

### Native output scheduling qualification

Run `just jpeg-output-scheduling-check`. At pinned zenjpeg revision
`4535ec499d88b04f58882df03e61761526c35756`, the matrix covers sequential and
progressive coding, fixed and optimized Huffman tables, and heights 8 and 65
at width 97 with strided RGB8 input. It compares finish-first, per-row draining,
seven-byte partial consumption and the shared three-byte short-write driver.
Every schedule produces identical encoded bytes for the same configuration;
libjpeg-turbo independently reads every comparison output. Repeated pending
inspection does not advance or change output. Only the taller sequential
fixed-table case reports streaming.

This verifies byte scheduling, not memory retention. The input fixture remains
owned by the caller throughout: even a strip-based native encoder cannot release
a caller's complete raster. The shared `encode_to_writer(PixelSlice, ...)`
therefore cannot establish a bounded end-to-end sequential source route.
A qualified live route also needs pausable source ownership, bounded retained
codec/intermediate state, and an explicit publication decision. Finite staging
must remain a separate executable alternative, not a fallback chosen after a
streaming requirement has already been accepted.

Source inspection identifies different native mechanisms:
`zenjpeg/src/encode/streaming.rs` selects strip storage for sequential,
non-optimized, non-XYB/non-RGB operation; other configurations retain blocks.
That inspection is not an allocation bound and does not qualify every metadata,
quality-search or source configuration.

Validation receipt (2026-10-27): native matrix and strict lint passed;
`rc=0 6s | peak-RSS 0.57GiB | min-avail 12723MiB | peak-load 0.36`.
The wrapper covers compilation and testing; it is not a codec RSS comparison.

The prepared scheduling witness also calls the shared
`resolve_output_schedule` before requesting any generated row or writing the
retained destination. It executes the selected route without reconstructing the
encoder. Fixed-table JPEG offers live and finish-first; optimized JPEG offers
finish-first only. Required unavailable schedules and required zero wait-memory
bounds refuse before source/destination effects. Both native routes retain
unknown total memory evidence; a pull source is never treated as proof of a
bound. This witness now executes the shared row_encode driver; the private scheduling loop has been removed. Native total-retention bounds remain unqualified.
