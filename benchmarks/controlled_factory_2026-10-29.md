# Controlled decoder factory admission — 2026-10-29

Base: `feea0ffd15129663fe6dc2c8c1e3e4378c5a2c4d`.

`PolicyDecoder::try_from_factory` checks required native workspace before
invoking the factory, then retains the same policy for decoding. Unknown native
workspace refuses, including an unlimited numeric cap. Without that requirement,
the factory runs once; its original typed failure survives through the wrapper.

Native controls cover zero factory calls and no charged storage on refusal,
original timeout kind/message on factory failure, and metadata requirements
still refusing final adoption after discovery. Baseline/progressive real JPEG
completion now uses the factory entry point with unchanged exact row, metadata,
extras and one-time owner transfer assertions.

`just controlled-workspace-check` passed in an isolated export: 12 native control
tests, 7 native decode-admission tests, all IO feature tests/doctests and strict
native/IO Clippy. Serialized wrapper rc0, 25s, peak RSS 0.71 GiB, minimum available
13821 MiB, peak load 0.69. IO package output was rebuilt for the new export.
These are validation resource receipts, not codec performance measurements.

The first new deferred-metadata fixture used a late profile and reached
Unavailable rather than Final, so the callback-count assertion failed (11/12
controls passed, rc101, 11s, peak RSS 0.72 GiB). The corrected test uses header
metadata to exercise the intended final-adoption path. Existing late-profile
controls remain unchanged; the original failure is retained.

No codec trait or zenpixels change is needed. Captured values evaluated before
the closure runs are outside this gate. Positive evidence for bounded native
factories and general deferred obligations are still unqualified.

Guide qualification at `bd04ce03c383294c674c7e172d52df498557b6e7`:
eight-crate Rustdoc, all 24 source controls and the complete browser suite passed.
`just docs-reference` and `just docs-site-browser-check 3547` ran in an isolated
export under run-heavy 3G/jobs2 and timeout180: rc0, 46s, peak RSS 0.34 GiB,
minimum available 13707 MiB, peak load 1.59. The code-first example exposes the
factory signature alongside the executing native JPEG completion control.
