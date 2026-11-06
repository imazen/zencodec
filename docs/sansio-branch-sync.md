# Sans-I/O branch synchronization, 2026-11-06

## Open gates

Native PNG at `a26cdc64` cannot resolve the canonical-plane cohort: its stable-only
archmage/magetypes requirements reject the explicit beta.2 requirement of pixels
`fde59b96`. The standalone probe in `perf/sansio-sync-validation` preserves this
failure before compilation. It is not a passing native PNG receipt. PNG ownership
and its gray/pixel changes remain separate from this synchronization.

The recovered JPEG implementation is [draft PR258](https://github.com/imazen/zenjpeg/pull/258).
Its selected native graph passes; the full JPEG workspace still fails resolution
on optional UltraHDR SIMD's stable-only requirement. Default triangle remains
whole-file backed, native resumption is partial, and full features, API snapshots
and platform qualification remain open. Do not infer completion from shared core
checks or promote the draft as a general native incremental decoder.

## Branch inventory and disposition

| Repository / published ref | Disposition |
|---|---|
| zencodec `sansio` (`666be3a8`) | Forward-merge current main, retaining current step/Input/optional-Pulse source; obsolete push/pull definitions are superseded |
| zencodec `feat/sansio-feed-step`, `feat/whole-file-incremental` | Already ancestors of main |
| zencodec `review/sansio-incremental-decoding` | Historical proposal; current Input/request/step contract supersedes it |
| zencodec `feat/sansio-seekable-source` | Historical separate seekable tier; current ReadAt/job source binding and companion I/O crate supersede it |
| zencodec `feat/sansio-coroutine-source` | Historical in-core coroutine and review notes; current companion crate and terminal/input controls supersede it |
| zenjpeg `sansio` (`7c59f19e`) | Forward-merged with main `cae3a9f3`, migrated and qualified in PR258; not merged to main |
| zenpng `sansio` (`11b26dd9`) | Source buffers until EOF before pixel decoding; it is not a native input-progress implementation |
| zenwebp `sansio` (`e62de277`) | Legacy push/pull adapter; no migrated native qualification in this lane |
| zenbitmaps `sansio` (`7e09b6a7`) | Historical QOI work; current main already contains the step-native QOI route; no replay of the old trait surface |
| zenjpeg/zenpng/zenwebp/zengif/zenavif/heic/zenextras/zenjxl `feat/sansio-whole-file` | Already ancestors of their inspected main histories; keep those implementations, not the historical names |

Local cached sibling main refs were stale. Live GitHub branch/PR inspection found
PNG main `a26cdc64`, JPEG main `cae3a9f3`, and core main `1fb997bb`.
Primary JPEG/PNG checkouts contained unrelated WIP; source work used an isolated
JPEG workspace and left those changes untouched. No historical branch was
force-pushed or deleted.

## Graphs qualified separately

| Identity | Main qualification | PR199 compatibility only |
|---|---|---|
| zencodec family | `1fb997bb88e7405e6a315bf1f0b22f71e99be272` | `88fc5871f38f34153d1dfe77379c94e1a2abb35c` |
| pixels/converter | `fde59b96632557592cffa9b7301acd8db96ed078` | `b6b12bbe702e7ca2f93db559858ae1f233a066ec` |
| garb | `605276d7` | `58ce9cad` |

PR199 remains unmerged. Its gray ICC/CICP and source-depth APIs were not put into
main. The selected JPEG consumer passed on each graph using a separate export
and target. Its lock comparison changes exactly the core family, pixel family
and garb; each graph has one shared core/pixel/Pulse/whereat identity. Enough and
almost-enough resolve to 0.4.5, how-far to 0.1.0 and whereat to 0.1.5.

Both shared graphs pass incremental testkit controls, coroutine and all-feature
I/O library/integration suites, strict I/O/coroutine Clippy, WASM and genuine
freestanding no-default checks. `sansio-sync-core-check` records the reproducible
commands. The actual native JPEG receipt and source/consumer limitations live in
[its owning repository](https://github.com/imazen/zenjpeg/blob/sansio/benchmarks/sansio_sync_2026-11-06.md).
The small [UltraHDR clone repair](https://github.com/imazen/ultrahdr/commit/aadf102169086fc232205bdb3b92572cfda07032)
is on remote main with green CI; it preserves the existing infallible signature
across old and fallible pixel-copy cohorts.

Shared main wrapper: rc0, 14s, peak RSS 0.30 GiB; shared PR199 wrapper: rc0, 48s,
peak RSS 0.43 GiB. These are serialized run-heavy validation resources on r7900x,
not codec performance or memory bounds. The initial `--all-targets` invocation
also launched executable benchmark targets; it was terminated (rc143), and its
partial timings are not evidence. The corrected command selects library/tests.

## Caller and copy implications

The ordinary erased factory remains the caller's common path. JPEG's explicit
review method `into_incremental_decoder(&[PixelDescriptor])` returns a concrete
owner implementing `step(Input, sink, Option<&dyn Pulse>)`; it is not promoted
as a second mandatory application route. `None` adds no borrowed control and
never disables an owned SharedPulse. Sink loans yield independently from input
starvation, and successful callbacks are followed by stop checks.

The JPEG candidate copies accepted compressed input into retained parser/scan
storage, reconstructs through a scratch batch and copies into one sink loan per
step. Marker metadata remains retained; progressive input remains buffered.
Neither a row interface nor early output establishes a whole-operation memory
bound. No copy-cost, throughput, preemption or bounded cancellation-latency
claim was measured in this lane.
