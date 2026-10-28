# Native contract validation

Executable witnesses of common encoder/decoder contracts against real native
codecs: multi-pass encoding (IMI), video encoder retention and delay (AV1), and
region read planning over large TIFF tiles. These are experiments, not
production entry points. Each area keeps a witness of the current behavior and,
where a shape is proposed, a candidate that passes the same native gates.

## Multi-pass pass plan

IMI's bounded lossy encoder reads its source in several full passes. How many
depends on its plan and on content: with exact blocks on a tier whose DC
stripes fit a work band, the nested pass runs only when blocks are selected.
No output byte is written before every analysis pass has finished, and the
coding pass is always last. The [prototype](../multipass-validation/README.md)
cannot state either fact: `PassCount::{Exact, AtLeast}` has no upper bound, and
its adapter reports `OutputTiming::streaming(height - 1)`, which is false after
the first pass.

The native facts come from `BoundedLossySession::passes()`, a proposed
additive accessor in imazen/zqi (branch `bounded-lossy-passes`, revision
`0e4419a7fe1c76378a836546a7f686ecfc05b70a`, under review): `{ min, max,
band_rows }` from the plan, before any source row. Its oracle test checks every
observed encode against the plan; content reaches both ends of the range.

`tests/baseline_multipass.rs` passes against the unpatched prototype by
observing its gaps (read counts increment on every read attempt):

| Test | Observed |
|---|---|
| `current_budget_admits_an_open_count_by_its_minimum_then_refuses_after_reading` | `AtLeast(2)`, budget 2, three passes needed: `PassBudget { pass: 2 }` after two full passes were read |
| `current_exact_overrun_reads_the_extra_pass_before_refusing` | `Exact(2)`, three passes requested: the third pass is read, then `IncompletePass { pass: 2 }` |
| `current_live_bytes_from_an_analysis_pass_are_published_before_a_budget_refusal` | bytes from passes 0 and 1 are written live, then the budget refusal |

[`candidate/multipass-pass-plan.patch`](candidate/multipass-pass-plan.patch)
changes the prototype driver and IMI adapter:

- `PassCount::Range { min, max }` and `PassCount::max()`. A `Source` route is
  admitted only when the planned maximum is within the pass budget; an
  unbounded count never re-reads under a budget. Staging stays available.
- `PassPlan::output: PassOutput::{FinalPass, AnyPass}` and
  `RowRequest::final_pass`. `FinalPass` forbids pending bytes before the final
  pass; the driver refuses them before publication or staging
  (`Refusal::EarlyOutput`). It does not promise bytes during the final pass:
  `AT_FINISH` final timing, an empty pass and a cancelled pass are legal.
  `OutputTiming::lag_rows` counts rows of the final pass.
- A pass outside the declared plan (beyond its maximum, after the final pass,
  or a final pass below the minimum) is refused before that pass reads
  anything (`Refusal::PassLimit`, `IncompletePass`). Contradictory plans refuse
  at preparation (`Refusal::InvalidPlan`).
- The control stays the prototype's `&dyn zencodec::Pulse`: `check` is
  cancellation only; `advance` counts rows supplied to the encoder (staged rows
  included); the total is `Exact` when the pass count is exact, `Unknown` for a
  range until the final pass begins, then `Exact`.
- IMI's adapter declares `Exact`/`Range` from `passes()`, `FinalPass`, and
  `streaming(band_rows)` (a pull returns right after filling a band and the
  session codes it on the next pull). A single-band image withholds its head
  until finish and declares `AT_FINISH`, which streaming cannot state.
- Custom stop policies in the native tests are passed as
  `StopOnly::borrowed(&stop)`.

Two existing expectations change because refusals now happen earlier:
`more passes than exact` expects `PassLimit { pass: 2 }` instead of
`IncompletePass { pass: 2 }`, and the IMI budget control (renamed
`source_pass_budget_below_the_planned_maximum_refuses_before_reading`) expects
`PassBudget { pass: 0 }` at `Stage::Policy` with no pass read, instead of
`PassBudget { pass: 2 }` after two passes.

`candidate/tests/pass_plan.rs` (scripted encoder) and
`candidate/native-tests/pass_plan_native.rs` (real IMI) cover admission before
any read, refusal of undeclared passes before their read, early output, empty
and cancelled final passes, progress totals, IMI's range reached at both ends,
silence before the coding pass, one-band lag within it, and the single-band
head withheld until finish. Every existing prototype control and native gate
also runs on the candidate. On the 420x330 exact-block fixture IMI planned
`Range { min: 3, max: 4 }` with 64-row bands and requested 990 rows (three
full passes) before its coding pass.

## Video encoder retention and delay

`VideoEncoder` lets an accepted frame be cloned (`SendStatus::Accepted`: "the
frame handle was cloned"), which keeps the caller's accounted picture charged
inside the encoder, or copied. It declares neither that choice nor how many
frames it accepts before its first packet.

- `tests/video_retention.rs`: two conforming toy encoders with identical
  packets, copying versus cloning with a three-frame delay, need one versus
  four caller frames. With a two-frame accounted pool the cloning encoder fails
  the third allocation before any packet exists.
- `video-native/tests/encoder_retention.rs`: zenavif's media encoder
  (`feadc4c9`) copies planes (a one-frame accounted pool never runs out) and
  produced its first packet after 19 accepted 64x48 4:2:0 frames at speed 10
  and after 27 at speed 6. The delay depends on configuration and is only
  observable by sending frames.

- `video-native/tests/late_metadata.rs`: an HDR content-light metadata OBU
  spliced into the fourth of six temporal units. rav1d-safe reports
  `[None, None, None, Some((1000, 400)), Some((1000, 400)), Some((1000, 400))]`
  per picture; zenavif's media decoder returns frames identical (interpretation,
  presentation, timing, samples) to those of the stream without the OBU. Media
  frames, tracks and codec configurations have no carrier for HDR static
  metadata, so a transcode through them drops it without any signal.

`video-native` pins the AV1 harness's native encoder and decoder (zenavif
`b25f7604`, which adopts the Pulse controls, and rav1d-safe `b19aec65`).
Pulse changes neither result: on the Pulse tree the encoder still copies
frames with the same first-packet delays, and the late content light level
still never reaches a media frame.

## Region read planning (large TIFF tiles)

The region candidate in [`perf/spatial-validation`](../spatial-validation/README.md)
admits each coding unit lazily: its decoded size before decoding it, its
compressed size through the parser's intermediate limit when it reaches that
unit. A refusal at a later unit therefore follows the earlier units' reads.

`spatial-native/` includes that candidate unchanged and adds `plan()`: from
tags alone, the units a region needs, every plane's stored compressed range,
decoded sizes, output and compressed totals. A batched fetch then reads every
admitted range in one transport call, and a planned reader serves the native
parser and refuses any unplanned byte.

On a 512x512 RGB8 file with four uncompressed 256x256 tiles:

- The last tile declares one byte more than the unit allowance. The current
  region read delivers tiles 0-2 in full (3 x 196,608 bytes) and then refuses.
  The plan refuses with no tile byte read.
- A total compressed cap one byte below the four tiles refuses before any tile
  read.
- An honest region crossing all four tiles: the direct read made 60 transport
  calls (4 in tile data) for 786,628 bytes; planned, 59 metadata calls plus one
  batch for 786,676 bytes, then decoding from planned bytes with no transport
  call and identical samples. The parser's small metadata reads remain; a range
  transport would also need the directory region planned or buffered.

A tags-only plan cannot see streams shared outside chunk ranges (old-JPEG
tables); the planned reader then fails explicitly rather than reading them.

## Results

Run on r7900x, Rust 1.99.1, in exports outside the checkout. Receipts include
compilation; they are not codec memory or speed measurements, and no memory
bound is claimed for any route.

| Gate (zencodec base) | Result | run-heavy receipt |
|---|---|---|
| baseline + video retention + strict clippy (f73a7e0e) | 4 passed | `rc=0 7s \| peak-RSS 0.33GiB` |
| candidate prototype + strict clippy (f73a7e0e) | 23 passed (8 new) | `rc=0 2s \| peak-RSS 0.21GiB` |
| candidate native IMI + strict clippy (f73a7e0e, imij `0e4419a7` from git) | 17 passed (4 new) | `rc=0 50s \| peak-RSS 1.19GiB` |
| spatial-native + strict clippy (f73a7e0e) | 3 passed | `rc=0 3s \| peak-RSS 0.23GiB` |
| video-native incl. late metadata + strict clippy (0fa2f60e tree) | 2 passed | `rc=0 1s \| peak-RSS 0.52GiB` |
| zqi `bounded_lossy_passes` (release, LTO off, 16 codegen units) | 1 passed | `rc=0 38s \| peak-RSS 1.09GiB` |
| zqi existing `bounded_lossy_session` + the new test | 3 + 1 passed | `rc=0 2s \| peak-RSS 0.22GiB` |
| zqi `cargo clippy -p imij --all-targets -D warnings` | clean | `rc=0 15s \| peak-RSS 0.86GiB` |
| baseline + video retention + strict clippy (0fa2f60e) | 4 passed | `rc=0 7s \| peak-RSS 0.33GiB` |
| candidate prototype + strict clippy (0fa2f60e) | 23 passed (8 new) | `rc=0 3s \| peak-RSS 0.20GiB` |
| candidate native IMI + strict clippy (0fa2f60e) | 17 passed (4 new) | `rc=0 57s \| peak-RSS 1.18GiB` |
| spatial-native + strict clippy (0fa2f60e) | 3 passed | `rc=0 6s \| peak-RSS 0.40GiB` |
| video-native (zenavif build) + strict clippy (0fa2f60e) | 1 passed | `rc=0 96s \| peak-RSS 1.15GiB` |
| video-native incl. late metadata + strict clippy (f73a7e0e, Pulse zenavif) | 2 passed | `rc=0 19s \| peak-RSS 0.53GiB` |

## Run

Inside a heavy slot, from this directory (exports go outside any checkout):

```sh
just export <new-export-dir> <zqi-revision>    # or export-local <dir> <imij-path>
just baseline <export-dir>
just candidate <export-dir>
just candidate-native <export-dir>             # private zqi access
just spatial <export-dir>
just video <export-dir>
```

No zenpixels change was needed.
