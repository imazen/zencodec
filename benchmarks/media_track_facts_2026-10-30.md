# Demux track declaration conformance — 2026-10-30

The shared demux conformance helper previously compared track counts at headers
and identifiers at configuration changes. It now compares every current track
field at both boundaries, across slice and random-access input drivers.
This is consistency evidence, not independent proof of container interpretation.

## Controls and frozen sources

- Test-only source `20517abb`: the regression fails because the old helper accepts
  differing facts. Ten fields are varied at headers and configuration changes
  while retaining identical packet events.
- Corrected source `e5347a9d`: all 77 media-testkit tests pass, with no ignored or
  filtered tests; strict all-target Clippy passes.
- Native source `67858ddd5be9e18a9927573be390812d4e71bff30`: the focused native
  MP4 test passes (1 selected, 49 filtered). It encodes three actual AV1 frames
  and checks progressive and fragmented mux output through the shared helper.
  Strict all-target Clippy for the native harness passes. Other native runtime
  tests were not rerun. The locked graph audit reports 12 interface consumers
  with zero mismatches; the lock adds only the path testkit and its harness edge.

The native control uses the same native mux/demux family. It does not prove
edit-list completeness, edited playback, independent player parity or VP8.

## Reproduction and resource receipts

Run `just media-track-facts-check`. Frozen runs used isolated exported sources,
separate before/after targets, locked dependencies, 3 GiB memory caps and two
build jobs. The native command had a 900-second timeout.

| Run | Result | run-heavy elapsed | Peak RSS | Minimum available | Peak load |
| --- | --- | --- | --- | --- | --- |
| Before | expected rc101 | 12s | 0.32 GiB | 14477 MiB | 0.25 |
| Corrected testkit + Clippy | rc0 | 21s | 0.32 GiB | 14506 MiB | 0.60 |
| Native + Clippy | rc0 | 100s | 0.78 GiB | 13828 MiB | 1.57 |

These are build/test resource receipts, not codec performance measurements.
An initial offline metadata resolution lacked cached generator 0.8.11; online
locked metadata fetched it without changing package versions. That failure is
retained separately from the passing build.

Archive SHA256 values:

- Before: `80e967bd4a69fb7d1a77a6c1aabd93c5b690a98a85e81481038556a771d77dc4`
- Corrected: `fdbc1cde950be1ae0fab2bcad0df1fc2c90aa9c6c82f6980d179f987df346f81`
- Native: `7e7812910cb1ac0e95f34e6b25cd65977b8c2eb0fd1aa1a98f527bd58e9d82a0`
- Native lock: `fa71c1320aadff87484e03af917c1f01f80a80c2dd6b836c7975edcdd0ceaecd`

All eight compiler catalogs were regenerated (rc0, 51s, peak RSS 0.35 GiB,
minimum available 14251 MiB, peak load 0.86). Only media-testkit documentation
and source locations changed; its public surface hash is unchanged. All 26 site
source checks pass (rc0, 4s, peak RSS 0.23 GiB, minimum available 13634 MiB,
peak load 0.18). Browser interaction tests were not rerun for this source-only
catalog update. Native and catalog processes were verified exited.
