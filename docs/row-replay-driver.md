# Retained-job row and replay driver

`zencodec_io::row_encode::{encode_job, encode_dyn_job}` now dispatch the retained
job's `Rows` or `Replay` plan through the shared `Encoder`/`DynEncoder` traits.
The raw-encoder entry points keep their existing push-only ordering. Whole-raster
jobs refuse with the original typed `EncodePlanError::InputProtocol`; there is
no allocating fallback. This implementation is source-only and unqualified until
the serialized controls and native integration are completed.

## Admission and execution

The driver checks both bound and legacy workspace requirements before calling
`plan_encode`. Generic job hooks supply no workspace bound, so every configured
workspace limit refuses at that point. The exact retained job plans once;
`begin_declared` must reproduce that complete plan after the joint policy choice.
No reconstruction or fallback follows a refusal or a source/native/write error.

One `Question::EncodeRoute` carries common configured-job evidence and complete
acquisition × output choices. Generic jobs do not expose qualified metadata,
profile, fidelity, extended-range or wait-retention evidence. Those values stay
unknown, and strict requirements refuse. The driver never derives them from
setter acceptance, a color carrier, a source mechanism label or streaming timing.

The driver probes candidate managed reservations independently, releases those
probes, then reserves and allocates only the chosen acquisition before native
construction or source reads. This avoids charging mutually exclusive stores
simultaneously. A callback or another budget user can exhaust the pool between
probe and selection: allocation then fails before construction/reads, preserving
the selected decision. Push-row storage is also allocated after policy, keeping
the existing zero-reservation callback contract.

| Acquisition | Actual storage and replay validation |
|---|---|
| Retained raster | `Source::borrowed_pixels()` returns the actual held `PixelSlice`; native requests copy its active rows and honor both strides |
| Direct rows, native maximum one | One source traversal, with no replay hash or snapshot |
| Positional replay | `supports_positional_replay()` permits restarts; row digests and an optional `replay_identity()` detect changes |
| Private RAM snapshot | One source traversal copies active rows into admitted private storage; later native requests read that storage |
| Raw disk replay | One traversal writes an independently supplied file; later reads verify row digests; this never uses the compressed-output spool |

The first raster probe chooses the borrowing branch; the second returned view
is the retained view used throughout execution. Neither a boolean capability
nor `OutputSource::BorrowedRaster` establishes immutable evidence. A positional
capability does not establish sample identity. The rolling digest detects
changes but is not cryptographic or a proof of exact samples. Disk aliases can
still exist; exact-replay policy excludes this route. A snapshot freezes the
acquired values, not an atomic historical state of a concurrently mutable source.

Raw disk and compressed staging have separate owned files and byte caps. Callers
own paths, permissions, deletion and non-aliasing with each other and the final
destination. Raw disk staging retains RAM digests under the managed pool. A
forward source may use raw disk when RAM staging is forbidden; unknown/finite
native replay counts still require valid final-pass completion. External staged
acquisition counts as one source pass; native passes remain independently checked.

Native requests must cover each complete image in order, beginning at pass zero,
with consecutive pass numbers and consistent final-pass status. Labels are
opaque diagnostics and never determine finality. Each successful pull performs
exactly one fill; `Done` performs none and requires a complete final traversal
within the native minimum/maximum. Diagnostic state is constant-size, not a
history vector. Exact-depth validation covers every active row before a fill
succeeds or a push sees the strip. Failed fills retain their first typed cause,
even if the codec swallows the marker; failed pulls never drain new output.

Finish-first drops acquisition storage, source, push buffer and encoder before
publishing to the destination. The final patch set is validated before appending
the tail. Cancellation is checked before patch seeks, before restoring the end
cursor and before flushing, retaining confirmed append/patch counts. Earlier
live output and failed external writes cannot be rolled back. Arbitrary callback,
Pulse, destination and native allocations have no inferred bound.

## Unknown generic-job evidence

The driver isolates one core dependency in private `unknown_job_evidence()`:

```rust
let evidence = zencodec::operation::EncodeEvidence::unknown();
```

`EncodeJob` and `DynEncodeJob` expose neither the destination format nor fidelity.
The existing non-exhaustive `EncodeOffer` cannot be constructed outside core;
`EncodeEvidence::new` requires one. The constructor uses
`ImageFormat::Unknown`, `fidelity: None`, `preference_index: 0`,
`resolution: None`, `metadata: None`, and `retains_opaque_profile: None`.
This preserves the current `EncodeOffer` representation and avoids pretending to
know the codec or introducing a parallel public job hierarchy. This source
candidate now includes that core constructor. There is no
permissive policy bypass or invented encoder configuration.

## Migration and costs

Planned-job callbacks now receive `Question::EncodeRoute`; their records contain
`Selection::EncodeRoute`. Raw-encoder callbacks still receive `OutputSchedule`.
Existing defaults and source implementations continue to work: the new source
methods conservatively return no borrow, no positional replay and no identity.
`with_batch_rows` bounds push strips only; a replay codec owns its strip size.
The historical `row_encode` module name now covers both protocols.

The driver uses a fixed six-slot offer array. Borrowed replay allocates no raw
snapshot or digest table. Push mode owns one managed strip; replay uses the native
loan directly. Snapshot mode reserves one raw array, disk/direct replay reserve
one row-digest array, and compressed finish-first storage retains its existing
chunk/file costs. Each borrowed/snapshot/raw-disk fill copies active samples;
direct source acquisition fills the native loan without an intermediate strip.
ICC-bearing plans remain borrowed by policy; bounded decision records carry no
ICC payload. Source/context checks can clone reference-counted handles; the
driver does not clone a full metadata owner for each candidate. No throughput,
heap/RSS, copy-count instrument or compile-time measurement has been run.

## Qualification still required

The source controls in `zencodec-io/tests/planned_row_encode/replay.rs` exercise
the real public typed and erased driver entry points with scripted shared-trait
implementers. They cover padded borrows, forward snapshots, positional and disk
mutation, finite/unknown native counts, caps zero/one/two, exact-replay refusal,
raw-RAM exclusion, reservation failure, depth verification, invalid pull progress,
swallowed errors, owner release, partial writes, patches and cancellation.
Existing planned-row controls retain their stages, byte/sample expectations,
pre-plan workspace deadline and pre-construction storage refusal. Only their
question/selection variants migrate to the joint route.

No Cargo command, build, test, native codec or performance tool was run for this
source implementation. Formatting and source review do not establish compilation
or passing behavior. Run these **serially**
in the root's authorized build slot; retain complete logs:

```sh
cargo test -p zencodec-io --features std --locked --test planned_row_encode -- --test-threads=1
just planned-row-check
just encoding-route-check
```

`planned-row-check` already includes std tests/Clippy followed by all-feature
tests/Clippy. Run the commands through the repository's required resource wrapper;
then regenerate the public source catalogs/API snapshots under the root's slot.
No new CI or core file is changed by this driver candidate.

Native qualification must use the actual retained IMI job and shared
`pull_rows`, with no experimental `MultiPassEncoder` adapter. Use identical
canonical active samples from a padded immutable raster, forward source plus
private snapshot, positional source and raw disk. Require byte equality against
the native in-memory encode and an independent readback where available. Check
exactly one plan/construction, actual pass labels/finality, refusal-before-read
for source cap zero and forbidden staging, source/stop failures, and output
publication order. Keep color and depth declarations identical across routes.
The scripted tests cannot establish native equivalence, Send behavior, metadata
completeness or a native workspace bound.
