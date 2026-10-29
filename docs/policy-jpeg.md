# Native JPEG policy evidence

Measured 2026-10-25 against zenjpeg
[`a2a128ab`](https://github.com/imazen/zenjpeg/tree/a2a128ab0bc36f01338225f61c285f462ae67ca6),
zencodec production source `4e3a6465`. Reproduce with
[`just jpeg-policy-check`](../perf/jpeg-policy-validation/README.md).
This qualifies specific configurations and generated fixtures, not all JPEGs,
ICC privacy, native video reconfiguration or whole-operation memory bounds.

## Current optional-control qualification

The 2026-10-29 paired consumer update passes all 48 native policy cases and five
preflight scenarios, plus strict Clippy, with the independent `djpeg` oracle and
all existing assertions retained. The exact sources, reviewed six-source lock
transition and disk-floor interruption are recorded in the
[optional-control receipt](../benchmarks/optional_pulse_2026-10-29.json).
The dated measurements below retain their original sources and scope.

## Why JPEG is the primary integration

The actual codec supplies progressive scans, split ICC, opaque XMP, orientation,
metadata-only rewrite and configuration-dependent output timing. Keep QOI as a
small exact-pixel control. Use JPEG to test whether proposed contracts survive
real interpretation and publication obligations.

The generated fixture is 97 × 65 RGB with a 312-byte stride (21 padding bytes),
deterministic noise, quality 87, a synthesized 440-byte Display P3 ICC profile,
orientation 6, and private EXIF artist/XMP/comment bytes. This is an adversarial
contract fixture, not a photographic quality benchmark. Metadata is placed in
the header, after the first scan, before EOI, or split across the first scan.
After-scan EXIF is not the Exif-specified header layout; these cases test tolerant
input handling and complete disclosure inventory, not recommended authoring.
The [Exif specification, §4.5.4](https://www.cipa.jp/std/documents/e/DC-008-2012_E.pdf)
places its APP1 immediately after SOI.

## Executed observations

| Path | Observation |
|---|---|
| Fixed-table sequential encode | `begin().streams()` is true. Pending output includes a header before rows. |
| Optimized-table sequential and progressive encode | `begin().streams()` is false; no bytes are emitted until finish in these fixtures. |
| Row submission | Strips of 1, 7 and 16 rows produce exactly the one-shot encoded bytes in all three configurations. Strided input is used throughout. |
| Decode paths | Whole-file incremental, row reader and buffered decode produce identical pixels for tested baseline/progressive and orientation cases. This does not establish ICC-managed color equivalence. |
| Incremental timing | The known-length slice driver supplies the whole 6,695-byte baseline or 6,950-byte progressive fixture before HeaderReady. Rows are produced only with full input/EOF. |
| Metadata snapshot | Header placement yields ICC 440 bytes and XMP 55 bytes. After-scan/before-EOI placement yields neither in the header snapshot; full decode returns both and late orientation. Split placement yields only 220 ICC bytes in the snapshot, versus 440 after full decode. One metadata event is observed. |
| Native metadata rewrite | With a 28-byte private trailer, inputs of 6,723/6,978 bytes become 6,533/6,788 bytes. All 1/15 scan spans remain byte-identical; private EXIF artist, XMP, comment and trailer fixtures are absent; orientation 6 remains. |
| Privacy admission | The rewrite reports retained ICC as `OpaqueIcc`. A strict scoped request requiring inspection refuses before copying any plan bytes. `ColorAndRotation` alone is not a privacy certificate. |
| Malformed/unavailable service | Incomplete/invalid ICC sequence counts refuse rewrite. Requesting XMP editing without a service refuses. PreserveExact demonstrably retains the private marker. |

The incremental implementation uses `WholeFileIncremental`; it is not a native
MCU-input streaming decoder. Its `info()` remains the probe snapshot. The native
row reader accepts the complete encoded input. Source:
[decode adapter](https://github.com/imazen/zenjpeg/blob/a2a128ab0bc36f01338225f61c285f462ae67ca6/zenjpeg/src/codec/decode.rs),
[whole-file helper](../src/helpers/whole_file.rs).

Do not infer that all recovered after-scan metadata should govern rendering.
zenjpeg's container inspection accessors intentionally use header metadata,
while its full decoder can recover later fields. A complete carrier inventory
and a final rendering interpretation are separate contracts. A partial ICC
payload must never be certified as a complete profile.

## Independent checks and transport

`djpeg` from libjpeg-turbo 2.1.5 (build 20260406) decodes the original and rewritten
files to identical PPM bytes. It also verifies equality between the native
row/one-shot encoder outputs. This checks same-decoder pixel preservation across
a rewrite, not bit identity between different JPEG implementations or ICC-aware
rendering. `kamadak-exif 0.6.1` independently reads orientation 6 and confirms
Artist is absent from rewritten output.

The unchanged [IMI transport model](../perf/io-validation/src/transport_model.rs)
serves actual JPEG driver requests. With 160 ms latency, 12,500 bytes/ms,
concurrency 8 and cache caps of either 0 or 131,072 bytes, both fixtures need two
rounds. Baseline transfers 6,695 bytes in **320.53560 simulated ms**; progressive
transfers 6,950 in **320.55600 simulated ms**. Both cache settings produce the same
result. Decoded pixels agree with the native buffered path. These are model
results, not network or CPU measurements, and do not qualify gigapixel ROI access.

## Validation and remaining gates

The native tests and harness Clippy with warnings denied pass. The pinned
future zenpixels bridge produces existing deprecation warnings in zencodec;
those dependency warnings were not suppressed. The run-heavy process receipt:
`rc=0 4s | peak-RSS 0.55GiB | min-avail 18298MiB | peak-load 0.20`.
This measures the build/test process, not a JPEG encoder retention bound.

Additional native controls now distinguish these scopes:

| Placement | Header/full probe ICC bytes | Full decode | Container header accessor | `djpeg -icc`, baseline / progressive |
|---|---:|---:|---|---|
| Both parts before scans | 440 | 440 | 440 | 440 / 440 |
| Both after first scan | 0 | 440 | None | None / 440 |
| Both before EOI | 0 | 440 | None | None / 440 |
| Part 1 before, part 2 after first scan | 220 | 440 | Invalid sequence | Invalid sequence / 440 |

A complete reversed-order pair before scans reassembles correctly. With part 2
missing entirely, the container accessor and libjpeg reject the sequence, while
`probe_full_metadata()` succeeds with the 220-byte fragment. That is a concrete
reason not to treat successful probing as final color qualification. The default
[`DecodeJob::probe_full`](../src/traits/decoding.rs) delegates to `probe`; zenjpeg
inherits it. Complete-metadata wording is insufficient to establish the effect.

The independent CLI is itself timing-sensitive: in
[`djpeg 2.1.5`](https://github.com/libjpeg-turbo/libjpeg-turbo/blob/2.1.5/djpeg.c#L809),
ICC extraction runs after scanline reads and before `jpeg_finish_decompress`.
Progressive input has consumed later markers at that point. Its different
baseline/progressive answers are observations, not a format specification or a
portable profile-selection rule. The
[ICC embedding note](https://www.color.org/technotes/ICC-Technote-ProfileEmbedding.pdf)
defines APP2 identification and chunk numbering/count consistency; it does not
supply the missing operation-finality contract.

Unknown-length input with actual reads capped at 1, 7 or 4,096 bytes reports
HeaderReady after the 4,096-byte prefix and still emits rows only with the full
6,695/6,950 bytes and EOF. The IMI transport path also reports its header at 4,096;
its initial request does not yet carry the source length. Known-length slices
instead ask for the entire source on the first failed probe. These differences
must not change the eventual interpretation guarantee.

[All native observation lines](../benchmarks/jpeg_policy_discovery_2026-10-25.txt)
include the command, source revisions and process receipt. Still unqualified:
ICC-managed color equivalence, native encoder retention while a writer stalls,
automatic replay and native video reconfiguration. No public interface changed.


## Guide integration

The [compiled JPEG examples](examples/jpeg_policy.rs) first execute the native
metadata rewrite, then add admission for retained opaque ICC. Both the no-ICC
success and ICC refusal paths run in the native harness. The refusal retains its
typed `At<Failure<Infallible>>` envelope and writes zero bytes; no callback is
invoked when no alternative can meet the requirement. This extra example checks
ICC admission, not all possible retained carriers or sample-level PII.

Tracked in [zenjpeg #238](https://github.com/imazen/zenjpeg/issues/238) and
[zencodec #166](https://github.com/imazen/zencodec/issues/166).

The guide's native example run passed with `rc=0 2s | peak-RSS 0.24GiB |
min-avail 18558MiB | peak-load 0.10`. Browser checks cover both JPEG examples'
full-source copying and inline expansion at desktop and mobile widths. The
CI-equivalent browser run passed (`rc=0 6s | peak-RSS 0.18GiB | min-avail
18661MiB | peak-load 0.43`). Optional full-page screenshot capture failed on one
local run; the lifecycle cards and earlier expanded views were captured and
inspected. No functional check was skipped.
