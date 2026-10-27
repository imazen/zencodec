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

### Unknown opaque-profile evidence

The harness now pins zencodecs `77d316ba`, whose retained transcode offers
explicitly supply known opaque-profile evidence to zencodec `f384e4c7`.
An unfilled offer remains unknown and cannot satisfy strict exclusion. The
adapter also compiles against its older pinned boolean evidence shape; the
native retain/discard assertions remain equivalent across that migration.

Validation (2026-10-27): all 35 JPEG qualification cases and strict lint passed,
including native rewrite privacy, late metadata refusal and checked transcodes.
The separate adapter suite passed all 22 cases against both its original pins
and a graph verified to contain exactly one local current zencodec. No zenpixels
change was needed. This establishes these routes, not whole-container PII
inspection of arbitrary XML, nested objects or reconstruction carriers.

Receipt: `just jpeg-policy-check <artifacts>`; log
`~/tmp/zencodec-profile-native-20261027.log`, artifacts
`/mnt/v/output/zencodec/platform-regions-20261026-run/profile-evidence-native`.
Resource wrapper: `rc=0 17s | peak-RSS 0.69GiB | min-avail 10109MiB | peak-load 0.82`.
These are build/test process measurements, not codec performance bounds.

### Final interpretation preset enforcement

A native baseline/progressive JPEG regression exercises seven required preset
constraints, both directly and through `UsePreset` delegation (28 combinations).
Discovery may continue, but final interpretation adoption refuses when the
question cannot establish the configured constraint. No sink begin, row or finish
occurs, and managed storage returns to zero. Before the fix the first case
accepted an unproven descriptive-metadata requirement.

Validation (2026-10-27): `just jpeg-policy-check <artifacts>`,
`just operation-policy-check`, coroutine integration tests and strict core lint
passed. The JPEG suite contains 36 tests. Combined wrapper receipt:
`rc=0 21s | peak-RSS 0.69GiB | min-avail 9659MiB | peak-load 1.45`.
This is build/test resource usage, not a codec memory bound. Logs:
`~/tmp/zencodec-boundary-preset-before-20261027.log` and
`~/tmp/zencodec-boundary-preset-after-20261027.log`.

Scope: this closes enforcement inside the built-in resolver. A custom callback
selecting a route directly still bypasses preset constraints in the current
contract. Mandatory requirements shared by callbacks and presets need a separate
admission rule; this regression does not claim to provide it.
