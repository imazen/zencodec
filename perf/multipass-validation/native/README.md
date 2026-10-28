# Retained native jobs through the public row driver

`tests/public_driver.rs` exercises real IMI retained jobs through
`zencodec_io::row_encode::{encode_job, encode_dyn_job}` and their destination
variants. It does not call the experimental `MultiPassEncoder` bridge. Existing
native harness tests and the committed `dependencies.lock` remain unchanged.

Root executed candidate `50fbb2ff` against core `d7b76287`, native `d3e0eac5`
and paired pixels `283655c5`: all 19 prior controls passed, as did the new real
parity matrix, typed fault/cleanup control and unconfigured registry refusal.
Three new tests failed on diagnostic expectations, so the overall gate failed
and Clippy did not run. These results do not establish complete native
qualification or independent cross-implementation correctness.

The source correction distinguishes early workspace admission's direct
`UnmetRequirements` from route refusal's typed `PolicyFailure` and retained
unmet-evidence record, matching the root public replay fixture. The missing
scratch-directory control now expects the native `InvalidInput("scratch directory")`
returned by `Store::open`. Every behavioral/no-effect assertion and prior control
remains. **The corrected tests have not been executed.** Root owns integration
onto qualified core `62b92eff` and the next qualification slot; no replacement
full export is prepared. The existing preparation scripts describe the original
frozen graph, not a new qualified revision.
Native production code at this revision retains the complete compressed output
until finish. This harness cannot qualify a live-output route for that backend.

The new controls require:

- Identical bounded-native bytes for padded RGB8 and RGB16, checksums on/off,
  immutable raster, forward private snapshot, positional reads and a distinct
  raw replay file. Typed and erased jobs run against Forward and SeekBack;
  SeekBack preserves an existing destination prefix. Raw replay and native
  index/checksum scratch have separate paths and owners.
- Identical decoded samples from the native whole-file decoder and its separate
  reader path, including XMP readback. The ordinary unbounded native encoder
  supplies a further decoded-sample comparison. These are **same-native**
  controls; no third-party IMI oracle is available in this harness.
- Actual native pass ordinals, complete traversals, one final pass, retained
  plan equality, no pending output before finish and no external patches.
  Counters distinguish external source traversals from native replay passes.
- Source caps 0/1/2, exact replay, raw RAM exclusion, live-output requirements,
  and unqualified workspace/metadata/profile obligations refusing before source
  reads or output. Hash-verified disk replay does not claim exact replay.
- Original typed source, cancellation, exact-depth, replay-mutation and native
  errors; no output on these failures; released driver reservations and native
  scratch. The writer checks that source, original job metadata and native
  scratch are gone before destination publication.

## Configuration gap and migration cost

The actual native `ImiEncoderConfig::job()` does not select scratch. The retained
path requires `.with_scratch(...)` on `EncodeRequest`. Public config erasure can
create a job, but cannot set native scratch on an already erased job or erase an
arbitrarily configured typed job. The direct registry-job control therefore
expects the native typed `EncodePlanning` refusal before output.

For the positive erased controls, a private `NativeConfig` implements only
`EncoderConfig`: its `job()` returns the real configured `EncodeRequest`. The
existing blanket `DynEncoderConfig` then supplies standard job erasure. No test
implements `DynEncodeJob`, replaces planning facts, or forwards native pulls.
This is consumer setup glue, not a new production API or a claim that the direct
registry path already supports retained scratch selection. A future production
consumer needs a native scratch selection mechanism before erasure or a supported
way to erase a configured job; neither is implemented by these tests.

Typed migration is one native job construction with scratch, metadata and limits,
then the public driver call. Erased migration currently adds that private config
wrapper and a transient configuration clone by standard erasure. Shared metadata
uses Arc clones there; native preparation still copies XMP to its own Vec once.
The shared job metadata survives until native completion/drop. The output-writer
check verifies release of that original Arc, not absence of every encoded copy.

The borrowed route copies active samples into each native loan. Positional replay
reads each pass and keeps row hashes. Forward snapshot copies the first traversal
into private full-raster RAM and copies subsequent passes from it. Disk replay
writes one raw raster, re-reads it and retains hashes; aliases and hash collisions
remain reasons it cannot establish exact replay. Neither disk path nor the native
index/checksum files make native compressed output stream: the native full output
and driver FinishFirst staging can coexist during final transfer. Reservations
here cover driver storage, not all native allocations or process memory. No cost
or memory measurement has been run.

## Frozen qualification export

`prepare_public_driver.py DESTINATION --core-git-dir CORE_GIT_DIR
--native-git-dir NATIVE_GIT_DIR` creates a **new** export outside ambient workspace
patches. It reads only immutable git archives, overlays these new tests and tools,
patches active registry core-family edges to one canonical exported root, and uses
the actual frozen native export as a path dependency. It preserves the committed
lockfile as both the starting Cargo.lock and `run/original.Cargo.lock`. It never
runs Cargo, fetches, builds or executes Rust tests.

The generated `run/COMMAND.sh` is the single serialized qualification command:
`run-heavy --mem 3G --jobs 2 -- timeout --signal=TERM --kill-after=30s 900 bash ...`,
with full stdout/stderr through `tee` and `pipefail`. The generated script first
checks source identities, runs ordinary incremental `cargo metadata`, and records
`LOCK-DELTA.json`, `LOCK-DELTA.patch` and `GRAPH.json`. It refuses unexplained
transitive changes, duplicate core/pixel identities and ambient path overrides
before executing the unchanged full native harness plus the new tests, then
strict all-target Clippy, all locked. No broad update or fresh lock generation is
permitted. Unexplained resolution changes require source review, not an expanded
allowlist to make the gate pass. Keep failed exports and logs intact; prepare a
new export for another attempt. Root owns qualification and any landing.

## Configured-job erasure follow-through (source candidate)

The corrected first-run controls now also have a third `ConfiguredErased` dispatch:
`job(config, scratch, metadata, limits).into_dyn_job()`. This uses the proposed
provided core method to move the real native job directly, preserving its native
scratch selection without the private configuration wrapper. The original typed,
configuration-erased and unconfigured-registry-refusal controls remain unchanged.
No new dispatch is qualified until its frozen run passes. This does not supply
unknown metadata/workspace evidence or enable live native output.
