# zencodec

Shared traits and types for zen* image codecs.

## Precision integration gates (2026-10-28)

- The paired zenpixels verifier at `283655c5` makes `DecodeOutput` 368 bytes
  on x86_64; the existing exact 360-byte guard fails. A compiler array-size
  diagnostic independently reports 368. The user approved updating the exact
  guard to 368. Focused, workspace and native JPEG controls pass; native lower-depth
  selection remains unqualified. This is object layout,
  not a measured heap or process-memory increase. See
  [precision admission](docs/precision-admission.md).
- Native GIF fuzz qualification exposed a dependency floor mismatch:
  `src/image/owner.rs` calls `PixelSlice::clone`, absent from zenpixels 0.2.14
  even though the pre-precision manifest admits it. The precision draft raises
  the floor to the paired 0.2.17 and pins the unpublished verifier; release
  still requires published dependency contents that provide those APIs.

## Workspace Layout

This repo is a Cargo workspace:
- **`zencodec`** (root package) — the published traits/types crate.
- **`zencodec-testkit/`** (member, unpublished) — conformance harness codec
  crates run against their own `EncoderConfig`/`DecoderConfig`. Ships
  `check_metadata_no_leak` (privacy), `check_cross_path_pixel_equivalence`,
  `check_orientation_roundtrip`, and a comprehensive bidirectional
  `check_capability_honesty`. Two in-crate codecs validate the harness: a faithful
  `reference` (honors every capability) and a `minimal` one (declares every
  optional capability false) for the false-direction branches, plus EXIF fixtures.
  Build/test the whole workspace with `cargo test --workspace`; the testkit must
  stay green and is the place to add cross-codec correctness checks.
- **`zencodec-io/`** (member, unpublished) — drivers that run a decoder over
  real input: `drive_from_slice` / `drive_from_read_at`, `pump_read_to_sink`,
  `BufferedReadAt`, `FileReadAt`, `CollectSink`, `EncoderSink`, and (feature
  `tokio`) `tokio_io::{decode_async, transcode_async, pull_decode_on_thread,
  ChannelRowSink}`. `no_std` by default; `std` and `tokio` features.
- **`zencodec-coroutine/`** (member, unpublished) — write an `IncrementalDecode`
  as straight-line `async` code (`CoroutineContext` / `CoroutineDecoder`);
  `no_std`, no runtime. Interface stays in zencodec; drivers and authoring aids
  stay out of it. Contract: `docs/range-decoding-contract.md`.

## API Specification

**[spec.md](docs/spec.md)** — canonical reference for the full public API surface.
Read this before modifying any traits.

**[correctness-model.md](docs/correctness-model.md)** — how color emission,
orientation, and metadata retention are resolved by the framework before a codec
runs (the "pit of success" contract), and how `zencodec-testkit` verifies a codec
honors it. Read before changing metadata/color/orientation flow.

## Purpose

Tiny, stable crate defining the common interface that all zen* codecs implement:

- **Encode**: `EncoderConfig` → `EncodeJob` → `Encoder` (type-erased, accepts any `PixelSlice`)
- **Encode animation**: `EncodeJob` → `AnimationFrameEncoder` (push frames one at a time)
- **Decode**: `DecoderConfig` → `DecodeJob` → `Decode` (one-shot), `StreamingDecode` (scanline batches), or `AnimationFrameDecoder` (animation)
- **Dyn dispatch**: `DynEncoderConfig` / `DynDecoderConfig` for codec-agnostic pipelines
- **Metadata**: `ImageInfo`, `Metadata`, `OutputInfo`, `Orientation`
- **Format detection**: `ImageFormat::from_magic()`, `ImageFormatRegistry`
- **Capabilities**: `EncodeCapabilities` / `DecodeCapabilities` (const-constructible flag structs)
- **Errors**: `UnsupportedOperation`, `CodecErrorExt` (error chain inspection)
- **Re-exports**: `enough` (cooperative cancellation), `Cicp`/`ContentLightLevel`/`MasteringDisplay` (from zenpixels)

## Design Rules

- `#![no_std]` + `alloc` — must build on wasm32
- `#![forbid(unsafe_code)]`
- Codec feature gates the trait hierarchy; pixel/metadata types always available
- No codec-specific types here (those live in codec crates)
- No `CodecError` here — each codec has its own error type (associated type on trait)
- Traits use GATs for lifetime-parameterized Job types
- `EncodeJob::Enc`/`AnimationFrameEnc` have NO trait bounds — codecs implement whichever
  encode approach they support (type-erased `Encoder`, animation, or both)
- **zenpixels pixel types: use but NEVER re-export.** `PixelDescriptor`, `PixelSlice`,
  `PixelSliceMut`, `PixelBuffer`, `PixelFormat`, `ChannelLayout`, `ChannelType`,
  etc. are defined in `zenpixels` and used as the cross-crate interchange format.
  All zen crates depend on `zenpixels` directly. zencodec uses these types
  in trait signatures but must not re-export them — callers import from `zenpixels`.
- **zenpixels color metadata types: re-export is OK.** `Cicp`,
  `ContentLightLevel`, and `MasteringDisplay` appear in zencodec's public
  API types. Re-exporting avoids forcing callers to add zenpixels as a
  direct dependency just for these types.

## Key Design Decisions

- **Type-erased encode**: `Encoder` accepts `PixelSlice<'_>` (type-erased, any format). Codecs do runtime dispatch internally. No per-format encode traits.
- **`StreamingDecode`**: Pull-based scanline iterator. `impl StreamingDecode for ()` is the rejection stub for codecs that don't support streaming.
- **Decode format negotiation**: Caller provides ranked `&[PixelDescriptor]` preference list. Decoder picks best match without lossy conversion.
- **Sans-I/O**: `IncrementalDecode::step(input, sink, stop)` with driver-owned input
  (the decoder takes a prefix and names its next `Request`); drivers live in
  `zencodec-io`, never here. Final rows always go through `provide_next_buffer`.
  Encoders stream through `begin`/`pending_output`. Contract and decisions:
  `docs/range-decoding-contract.md`.

## Consumer version requirements: always current-plus-next minor (STANDING RULE)

**Every consumer's requirement on `zencodec`, `zencodec-testkit`, `zenpixels`, and
`zenpixels-convert` must span the published minor AND the next one**, so a minor
bump does not require a coordinated re-pin wave across every repo.

For a `0.x` crate Cargo treats the minor as the major: `"0.1.26"` means `^0.1.26`
= `>=0.1.26, <0.2.0`, so a `0.2.0` release is invisible until every manifest is
hand-edited. Instead, given published `0.x.z`, write:

```toml
zencodec  = ">=0.x.z, <0.(x+2).0"     # accepts all of 0.x.* and all of 0.(x+1).*
```

As of 2026-08-29 (published versions verified with `cargo search`):

| crate | published | requirement to write |
|---|---|---|
| zencodec | 0.1.26 | `">=0.1.26, <0.3.0"` |
| zencodec-testkit | 0.1.0 | `">=0.1.0, <0.3.0"` |
| zenpixels | 0.2.16 | `">=0.2.16, <0.4.0"` |
| zenpixels-convert | 0.2.16 | `">=0.2.16, <0.4.0"` |

**The rule is current-plus-next, always — it moves with each release.** When
zencodec publishes `0.2.0` the requirement becomes `">=0.2.0, <0.4.0"`; when it
publishes `0.3.0`, `">=0.3.0, <0.5.0"`. Re-derive the ceiling at each release
rather than leaving a stale one in place.

**The floor stays at the version the consumer actually needs.** Only the ceiling
moves. Never truncate to `"0.1"` or `"0.2"` — the standing never-truncate-versions
rule still applies, because Cargo resolution is not "largest compatible patch"
when a sibling constrains the graph.

**Why this exists — the two-copies trap.** If some consumers widen and others do
not, and both feed one dependency graph, cargo resolves **two copies** of the
crate (e.g. `0.1.26` and `0.2.0`) and the types do not unify across the boundary:
a `zencodec::CodecError` from one copy is a different type from the other, so
trait impls silently fail to apply and errors surface as inscrutable mismatches.
This workspace has hit that failure twice recently (zenanalyze and `cubecl-ir`).
So the widening must be **uniform** — a partial sweep is worse than none.

The concrete motivation: the `zencodec 0.1.26` rollout
(`zen/ZENCODEC_026_ROLLOUT.md`) required touching every consumer repo by hand,
across weeks. The range makes the next bump automatic — consumers ride over
`0.2.0` with no edit at all.

**`[patch.crates-io]` entries are unaffected.** A patch replaces the source
regardless of the requirement, so bridges carrying unpublished versions stay. The
range removes the need for the future *edit*, not the present patch; delete a
patch when its crate actually publishes.

### ⚠ BLOCKER: `zencodec-testkit` must republish BEFORE `zencodec 0.2.0`

Widening the consumers is necessary but **not sufficient**, because one link in
the graph is a *published artifact* nobody can edit in place. Read from
`cargo metadata` (not assumed), the published `zencodec-testkit 0.1.0` declares:

```
zencodec  ^0.1.26      = >=0.1.26, <0.2.0
zenpixels ^0.2.10      = >=0.2.10, <0.3.0
```

Those carets will **not** admit `zencodec 0.2.0` / `zenpixels 0.3.0`. So the day
`zencodec 0.2.0` publishes, any repo that takes `zencodec-testkit` from the
registry resolves **two `zencodec` copies** — the consumer's `0.2.0` and
testkit's `0.1.26` — and a `check_*` helper handed a `DecoderConfig` from the
other copy fails to typecheck (E0277: the trait bound names a *different*
`DecoderConfig`). Affected today: `heic`, `zenbitmaps`, `zenraw`, `zenwebp`,
`zenjpeg`, `zengif`, and all four `zenextras` members.

The fix is already committed here — `zencodec-testkit/Cargo.toml` carries
`zencodec ">=0.1.26, <0.3.0"` and `zenpixels ">=0.2.10, <0.4.0"` — but a
manifest is not a release. **`zencodec-testkit` must be published with those
ranges before, or in the same batch as, `zencodec 0.2.0`.** Treat it as a
release-gate item, not a follow-up.

Two consumers currently take testkit (and zencodec) from **git** rather than the
registry — `zenjxl` and `zenpng`, whose comments still say testkit is
unpublished. It published as `0.1.0`; those git deps and the tag-pinned
`[patch.crates-io]` entries that exist only to unify with them can become plain
registry deps whenever someone verifies the swap.

### Git pins cascade through git-pinned intermediaries

While a zencodec version is unpublished, consumers pin it by git rev and
`[patch.crates-io]`. That patch reaches only **registry** edges. A dependency
taken from git with its own `zencodec = { git, rev }` (ultrahdr-core, which
heic depends on and zenavif patches in; zenpng and zenjpeg revs used by
ultrahdr and zenavif) brings its own zencodec copy, and cargo refuses a
`[patch."https://github.com/imazen/zencodec"]` that points back at the same
URL. So moving a consumer to a new zencodec rev means first moving every
git-pinned intermediary in its graph to that same rev (2026-10-14: heic
needed ultrahdr PR 40, which needed zenjpeg's matching branch). Put every
codec in one rollout on one zencodec rev, and check a consumer's lock with
`grep -c 'name = "zencodec"' Cargo.lock` in an export outside `~/work/zen`
(the workspace `.cargo/config.toml` path patches hide the split). Publishing
the version ends the cascade: registry edges unify through the range rule
above.

Two more traps in the same rollouts (2026-10-14):

- A local build under `~/work/zen` rewrites the consumer's `Cargo.lock` to
  the path sources of the workspace patches. Regenerate the lock in the export
  starting from the committed lock (`git show <rev>:Cargo.lock`), and copy it
  back after the last local build, just before committing.
- A codec PR stacked on another codec PR (base = a feature branch) gets no CI
  where the workflow triggers only on `pull_request: branches: [main]`
  (zenjpeg). Dispatch it on the branch: `gh workflow run ci.yml --ref <branch>`.

## Release Requirements

**CI MUST pass before any crates.io release.** This includes:
- All tests pass on Linux, Windows, macOS
- WASM build succeeds (wasm32-wasip1)
- Clippy clean (no warnings)
- Format check passes
- MSRV 1.88 check passes (`rust-version` in Cargo.toml — keep this line in sync)
- `cargo-semver-checks` passes (no unintended breaking changes)

**Before publishing:**
1. Verify README.md reflects current API
2. Run `cargo semver-checks check-release` locally
3. Bump version in Cargo.toml
4. Get explicit user approval
5. `cargo publish`

## Known Issues

One open cross-repo finding from the color/metadata scenario-matrix research
(2026-06-01). Resolved findings from that research (the double-rotation hazard,
fixed in this crate's `Metadata::filtered`; the zenavif descriptor-CICP
override, fixed in zenavif `b3be82a`) are recorded in the respective
CHANGELOGs. Full design context:
[`docs/color-emit-model.md`](docs/color-emit-model.md).

1. **Missing signal-range conversion kernels (zenpixels-convert) — HARDENED
   (refuse-fast, 2026-06-11, zenpixels main `54aca62e`), kernels still
   unbuilt.** `ConvertPlan::new` now refuses any `Narrow <-> Full` crossing
   with `NoPath` (Display names the range crossing), closing the
   relabel-without-rescaling hole — previously the allocating path emitted
   range-coded values under the wrong label. `SignalRange` docs now pin the
   ITU definition (anchors ×2^(N−8), studio-swing RGB/gray, excursion
   clamp-vs-preserve decision, CICP/AV1 mapping, the no-relabel rule).
   Narrow remains verbatim-passthrough-only (sole consumer: zenavif accepts
   Narrow PQ/HLG BT.2020 → AV1 limited range, no conversion). Build
   `ConvertStep::{ExpandNarrowToFull,ContractFullToNarrow}` only when a
   consumer actually needs to cross the boundary; known design points are
   recorded in the `SignalRange` docs.

## Known Bugs

- Fixed container final-position cancellation (2026-10-29): output could flush
  after its final seek signalled a stop. The driver now checks before flush;
  testkit controls retain confirmed append counts and native seek error priority.
  See `just media-publication-check` and `docs/tokio-execution.md`.

- Fixed async publication cancellation (2026-10-29): a writer could signal
  cancellation after accepting the final batch and the driver would still
  finalize, publish the tail and return success. Shared-Pulse checks now cover
  publication, queued finalization execution, tail and flush boundaries. Seven
  effect controls preserve already-written prefixes and original native causes.
  These checks do not interrupt a stalled writer or preempt a native call.
  See `docs/tokio-execution.md` and `just tokio-guide-check`.

- Controlled decode workspace deadline (2026-10-29): PolicyDecoder previously
  polled native discovery before rejecting an unqualified workspace requirement.
  Admission now precedes the first native step and retains a typed named cause.
  Use PolicyDecoder::try_new to admit requirements before construction;
  the already-owned new path cannot undo prior allocations. Positive native
  workspace bounds remain unqualified. Native failing-first control and receipts:
  `benchmarks/controlled_workspace_2026-10-29.md`.

- Guide browser CI on `3739a49a` failed an immediate URL assertion after the
  focused skip link received Enter. The check now waits for the exact
  `#journeys` fragment before retaining the original destination assertion.
  Matching Rustdoc, all 24 guide source checks and the complete browser suite
  pass on `7a26c718`; the original CI failure remains recorded.

- Fixed in the multipass experiment (2026-10-28): a short source stride could
  consume input before subtraction panicked; malformed file requests could
  change destination samples before refusal. Source validation now precedes
  these effects. Padding discard uses at most 4096 retained bytes and checks
  Pulse between chunks; stop causes remain typed. Four failing-first controls,
  strided parity and unchanged native gates qualify this candidate fix; see
  `benchmarks/multipass_sources_2026-10-28.json`. Production promotion remains open.

- Fixed in the multipass experiment (2026-10-28): source `exact_depth`
  declarations were carried but never checked. Incompatible claims now refuse
  before native startup; every active row is verified before successful fill,
  with the original typed cause retained even if a codec swallows interruption.
  The native IMI adapter also refused non-sRGB contexts but omitted descriptor
  primaries/range checks, permitting descriptor-only relabeling. Both failing-first
  regressions and native byte-parity controls are recorded in
  `benchmarks/multipass_declarations_2026-10-28.json`. The protocol remains an
  experiment; native planning/Send and source/diagnostic gates remain open.

- Fixed during independent policy-probe integration (2026-10-28): two
  `planned-row-check` recipes made the whole justfile fail to parse. The
  consolidated recipe retains both std and all-feature test/Clippy commands.
  `just --show planned-row-check` and the runner-control recipe verify parsing.

- Fixed during the zenpipe integration loop (2026-10-28): coroutine delivery
  split a native batch into small sink loans but filled all loans before
  returning, exhausting a bounded pull adapter's object allowance. It now
  retains the batch and yields one loan per event. The one-slot regression
  covers striding, provisional passes, final-poll rows and cancellation; native
  PNG requalification is separate. No queue cap or prior assertion was relaxed.

- Fixed during native integration supervision (2026-10-28): the code-first
  catalog fingerprint omitted crate READMEs, so zenmeta's `include_str!` crate
  documentation could change without invalidating its generated entries.
  Generation and site checks now include each crate README and verify exported
  inputs against the checkout. A regression checks README creation and edits.

- Fixed during standalone documentation qualification (2026-10-28): the guide
  vendor setup still fetched pre-Pulse bitmap/container revisions, while private
  resolver cancellation fixtures passed `Stop` directly to Pulse methods. The
  setup now pins the migrated native revisions, and the fixtures use explicit
  `StopOnly` adapters without changing their assertions. Stale imports also broke
  strict Clippy in the standalone renderer. The guide test/Clippy command,
  unchanged transport-model CSV comparison, and `just render-check` pass in an
  export outside workspace overrides. The JPEG harness's separate migration is
  still pending.

- Fixed on the API review branch (2026-10-27): the new row driver could publish
  a live header after policy changed source dimensions/context/acquisition.
  It now revalidates the retained declaration before output and before/after
  source reads, preserving the selected record and confirmed output count.
  Native regression: `public_row_driver_revalidates_source_across_policy_and_writer_callbacks`.

- Native render integration (2026-10-27) exposed a same-profile U8 sample
  change in zenpixels-convert when TIFF supplied an authoritative ICC and
  unknown descriptor color fields. Fixed by zenpixels `02ccdfb0`; the spatial
  harness pins it and checks exact TIFF samples and independent CMS output.
  The combined renderer remains private; collection-level integration,
  planar video and full workspace/privacy admission are still open.

- Fixed during destination-contract validation (2026-10-27): the former
  `apply_finish` appended its tail and applied earlier patches before rejecting
  a later invalid range. A native-output regression observed changed bytes on
  error. `MuxFinish::validate` checks the complete result before effects;
  `apply_to` reserves fallibly before mutation. The old helper is deprecated
  and delegates to the corrected implementation. External I/O can still fail
  after partial publication; validation is not a rollback guarantee.

- TIFF parity fork integration (2026-10-27): shared inventory now follows
  external JPEG tables 519–521 conservatively, but zentiff's codec-specific
  old-JPEG and predictor consumption rules still need qualification against
  image-tiff `10a3b6b2`. The spatial gate's lzw/deflate configuration does not
  exercise old JPEG. Track decoder read traces, EOF/shared-stream costs and
  consumer rollout in https://github.com/imazen/zencodec/issues/171.

- Fixed during the output-owner guide update (2026-10-26): four-stage flow
  diagrams used a three-stage SVG viewport and omitted the final connector.
  Viewport and connectors now follow the stage count. Browser checks require
  every stage to fit and retain the full four-stage conversion/publication route.

- Guide regression (2026-10-26, fixed): adding the operation-storage migration diagram
  exposed a fixed six-case browser assertion. Keep the original six case/role
  checks and verify the added diagram separately; do not weaken the existing
  rendering or filtering gates. The checked API page similarly retains its
  twelve original card checks while checking the added thirteenth card.

- Fixed during packed-owner qualification (2026-10-26): direct `OutputError`
  mapping diagnostics printed arbitrary backend text. Its default Display and
  Debug are now bounded while `source()` retains the original typed cause and
  trace. The control uses a payload in both backend formatting paths.

- Native spatial CI (2026-10-26): packaged JXL comparisons made the full
  unoptimized suite exceed its existing ten-minute command timeout. The
  standalone harness now uses test optimization level 1 with debug assertions
  and overflow checks enabled. `just spatial-ci-check` keeps the complete suite
  and timeout; `benchmarks/native_ci_2026-10-26.json` records qualification.

- Packaged-JXL qualification (2026-10-26): the first harness promotion omitted
  the regenerated tracked `perf/spatial-validation/dependencies.lock`; CI copies
  it to the ignored `Cargo.lock` before its locked build. Updating only
  `Cargo.lock` cannot repair CI. Keep the tested `dependencies.lock` with
  dependency-pin changes, including new packages.

- Fixed source-guide regression (2026-10-26): a duplicate `collectionOpen`
  key silently replaced the TIFF opening signatures with `OpenRequest`.
  Keep separate `collectionOpen` / `collectionOpenRequest` entries. The source
  gate now checks declared catalog keys against evaluated keys and rejects
  duplicates; browser coverage retains both native and consumer context.

- Independent raw-layer CI oracle (2026-10-26): libjxl 0.7 disagreed on the
  noise/spline fixture. A source-controlled experiment isolated the difference
  to two existing upstream noise fixes, without changing native output or the
  pixel gates. CI now uses checksum-pinned unmodified 0.11.1 and preserves
  comparison/profile evidence. See `benchmarks/layer_oracle_2026-10-26.json`.

- Fixed in the shared mapping review (2026-10-26): a zero-width `PlaneView`
  with nonzero stride accepted empty storage but `row(1)` indexed beyond it.
  Empty rows now avoid unused offset arithmetic. The regression covers extreme
  U8/U16 heights and strides without weakening non-empty geometry checks.

- Fixed during native TIFF spatial qualification (2026-10-25): `find_cause`
  skipped a transport error embedded in `std::io::Error`, whose `source()`
  delegates past its payload. With `std`, lookup now follows `get_ref()` before
  the next source link. Native partial-read/version failures and nested traced
  payloads exercise the regression; no error type or signature changes.

- Found during native PNG transcode qualification (2026-10-25): a native batch
  taller than the checked adapter's admitted strip failed with `StripGeometry`
  (97x65 guide fixture, default 64-row adapter). The added sink capacity method
  and shared copying path split delivery while retaining the native batch;
  tests preserve strided samples, row coordinates and provisional passes.

- Controlled-session retention regression (2026-10-25, ideal-api PR): native JPEG qualification found that a failed `PolicyDecoder` kept its suspended input allocation until the adapter was dropped. The adapter now drops the native session on error/unwind while retaining its report; decline, unavailable interpretation, native failure and callback unwind release the admitted input.

- Controlled decode review (2026-10-25): the initial fused wrapper held pending suspensions but trusted a backend not to call the sink before its first boundary. An adversarial backend reproduced an early `begin`. The wrapper now guards begin/final rows/provisional rows/finish independently and refuses swallowed guard errors. Coroutine-level adoption checks remain; native computation and final-fact correctness still require qualification. See `docs/controlled-decoding.md`.

- Fixed on the ideal API PR (2026-10-25): async image drivers copied every
  borrowed request before input admission, and transcode drained newly pending
  output even after a batch failed. Request copies are now bounded and failed
  batches do not publish new pending bytes. Earlier successful writes remain
  irrevocable. Work-quota controls preserve scheduling independently of writes.

- Fixed on the ideal API PR (2026-10-25): the sequential pump limited the
  requested minimum but could prefetch/skip into a larger allocation than its
  `max_window`. The existing cap now covers these paths; random-access and async
  drivers share explicit retained-input admission. `tests/input_admission.rs`
  verifies byte parity, capacity, alternative selection, batches and refusal
  before source reads. This does not bound source/codec/output allocations.
  Completed range batches are published atomically; partial errors and async
  cancellation leave no partial cache that a later call could consume.

- Fixed on the ideal API PR (2026-10-25): buffered exact spool requests formerly
  consumed input before discovering they exceeded the cap, and a large windowed
  read retained its entire requested span. Admission now precedes exact reads;
  forward output is filled directly while only the configured history survives.
  Tests cover zero/small caps, exact EOF, terminal excess, overflow, interrupted
  reads and fallible capacity reservation. The cap is retained payload, not whole
  process memory; length discovery admits one stack-byte EOF probe.

- Ideal API PR integration (2026-10-25): changing the standard source error
  type exposed fixed `ReadAt<Error = SourceError>` assumptions in the disk
  comparison harness. The controls now use a common lossless error adapter;
  their read algorithms and parity assertions remain unchanged. The guide gate
  and `just io-disk-compile-check` cover these consumers.

- Fixed during independent API review (2026-10-25): generic `find_cause::<E>`
  skipped the native error inside `At<E>` because the wrapper's source link
  delegates past it. Lookup now inspects `At<E>` at each link. The regression
  covers native and nested causes, trace access and erased native leaf errors;
  `just error-cause-check` retains the existing category/envelope checks.

- Fixed during independent API review (2026-10-25): handwritten `DynEncoder`
  implementations inherited a successful `begin` despite unsupported row input.
  The default now returns `RowLevelEncode`, matching the typed default. The
  ordinary `EncoderShim` was already correct. `just encoder-begin-check` covers
  typed, shimmed and direct dyn admission plus one-shot fallback after refusal.

- Fixed native source cause loss (requalified 2026-10-29): `FileReadAt` and
  `BufferedReadAt` retain `std::io::Error` in `StdSourceError`; source/driver
  erasure preserves its original kind, OS code and custom payload. The scoped
  `just io-source-errors-check` covers partial-input permission/transport errors
  and structural refusals. See `docs/io-contract-proposal.md` for scope.

- Fixed during API comparison (2026-10-25): the private policy resolver checked
  accounting scope only when a RAM cap existed, allowing driver-only evidence
  under an operation-wide disk-only cap. The regression fails before the fix;
  scope rejection now precedes the callback for either capped resource. Evidence:
  `docs/policy-scenarios.md#accounting-scope-regression-2026-10-25`. No released API changed.

- Fixed in guide qualification (2026-10-25): six driver examples converted
  errors to strings, losing their typed causes. They now propagate with `?`.
  `positioned_guide_keeps_the_original_source_error` reproduces the former
  failure and requires the original timeout kind and message in the error chain.

- Native JPEG policy qualification (2026-10-25): `DecodeJob::probe_full` defaults
  to the header probe despite its complete-metadata wording. Current zenjpeg
  inherits this; a missing ICC chunk can appear as a successful 220-byte profile,
  and whole-file incremental `info()` remains the early snapshot after decoding.
  The native container inspector rejects that incomplete sequence. Informational
  metadata events are not a suspension barrier before sink effects. Evidence and
  scope: `docs/policy-jpeg.md`, `docs/policy-transcoding.md`. Public API and codec
  behavior remain unchanged; these need qualified completeness and barrier contracts.

- Fixed freestanding policy integration (2026-10-29): implicit zenpixels std
  edges are disabled; private gain-map helpers use libm without std and preserve
  platform math with explicit std. Five runtime packages pass the genuine
  freestanding target, separately from WASM. Existing gain-map assertions remain
  unchanged. Numeric scope and reproducible gates: [no-std](docs/no-std.md).

- Fixed during IO contract qualification (2026-10-24): cursors refetched ranges
  already held in their active window and discarded batched ranges on sequential
  requests. Forward-only regressions cover full/partial overlap and suffix reuse;
  the real container matrix retains exact event parity. Genuine fragmented MP4
  backtracking remains tracked in zenavif issue 72.

- Fixed during IO contract qualification (2026-10-24): default `Encoder::begin`
  accepted row encoding although the default `push_rows` rejects it. It now
  refuses at `begin`, allowing the writer driver to use one-shot encoding.
  `tests/encode_write.rs` in zencodec-io covers inherited defaults with strided input.
  The writer also recognizes the shared error category: actual BMP exposed a
  second refusal path whose native detail has no typed source chain.

- I/O validation (2026-10-24): `just io-contract-probes` reproduces output
  batching tied to input starvation (#160), spool rejection after consumption
  and unsatisfied lookback admission (#161), and the default-begin one-shot
  writer failure (#162). Async range requests also fail to reuse prefetched
  window bytes (#163), making fast-start MP4 fail on a forward source at the
  default read size. Evidence: `benchmarks/io_validation_2026-10-24.md`.

- PR138 audit update (2026-10-13): the historical contract gaps below are fixed
  on the PR for timing capability, editor-free XMP preservation and report bounds.
  Adversarial review also fixed partial child-plan mutation, mixed-input plans,
  replaced-input admission, expanded EXIF budgets and false timing overflow
  (`eddbdd51`, `4ec35983`). The color realizability gap remains. Legacy EXIF
  allocation is still not fully fallible. See the PR138 section of
  `docs/zencodec-contract-audit-2026-10-13.md` for scope and remaining gates.

- Open contract gaps verified by `just contract-probes` (2026-10-13):
  rewrite `PreserveExact` rejects a request with no XMP editor before inspecting
  whether XMP is present. The color resolver can request ICC synthesis when the
  target cannot carry ICC, so callers still need a realizability check.
  `Plan::from_parts` trusts codec validation and does not check report ranges
  against the input length. See `docs/zencodec-contract-audit-2026-10-13.md` for
  the trust boundaries, inherited versus unpublished issues, and proposed fixes.

- Fixed during metadata/timing integration (2026-10-12): the existing timing
  proposal inserted `AnimationTiming` in the middle of `UnsupportedOperation`,
  changing numeric casts of six released variants. Append new variants after
  the released ones. `just metadata-semver` compares crates.io 0.1.26 and catches
  this even though the enum is non-exhaustive.

## Metadata conformance regression (2026-10-18)

The testkit compares color metadata against a clean same-policy encode before
reporting a leak: formats can require native color, and decoders can synthesize
an equivalent ICC profile. The exception requires exact ICC bytes/CICP equality;
unexpected color and discarded descriptive metadata remain failures.

## Known Bugs: checked guide mobile navigation (2026-10-25)

Fixed on the checked API review branch: adding a navigation entry overflowed the
390 px CI viewport. The header wraps its links and grows vertically. Existing
browser no-overflow assertions remain unchanged; evidence is in docs/ideal-api.md.

## Known Bugs: media error-chain gaps (2026-10-25, fixed)

The unpublished media pump retained upstream/downstream failures but omitted
`Error::source`, and mapping failures accepted only strings. Pump sources now
forward their original errors; `MapError::Backend` owns the backend error.
`just media-errors-check` verifies concrete causes and location traces through
typed and erased pumps, mapping adapters and fused terminal states.

## Known Bugs: F32 CMYK selected-output carriage (2026-10-26)

The raw-layer integration reproduced a descriptor-construction panic for native
F32 CMYK, then exposed a second hazard: native JPEG XL reverses ink polarity.
The follow-through uses zenpixels PR 93's conventional/inverted formats, fallible
adapter lookup, and explicit polarity on separate black. The selected-layer gate
passes without changing sample bits or the native conformance threshold; the
receipt retains the initial failure (`benchmarks/raw_collection_2026-10-26.json`).
F32 CMYK color conversion is still unqualified and refuses at runtime. Source ICC
bytes alone must never imply conventional polarity for inverted samples.

## Fixed frame-route preference regression (2026-10-27)

The initial coupled conversion offer left `prefer_lossless` ranking only encoder
fidelity. A sample-changing conversion could precede a fully preserving route.
The resolver now ranks complete conversion/encoder preservation first, retaining
required workspace gates and the legacy encoder-only fallback. The frame-policy
regression reproduces the old selection and checks both required-bound refusal
and explicit request-order behavior.

## Transcode profile evidence regression (2026-10-27)

The review API originally defaulted `TranscodeOffer::retains_opaque_profile` to
`false`, allowing a strict preset to accept a route without inspection evidence.
It now defaults to `None`; strict exclusion accepts only `Some(false)`. This
fix qualifies the policy decision, not whole-container PII removal or native
adapter postconditions.

## Final interpretation preset bypass (2026-10-27)

The review preset previously accepted a final `Question::Boundary` even when
metadata, memory, range or output requirements were configured. `PolicyDecoder`
can publish rows after adoption without another question. Adoption now refuses
these unproven obligations; provisional discovery remains permitted. Native
JPEG regression: `native_final_boundary_cannot_bypass_unproven_preset_requirements`.

## Known Bugs: array arguments in source signature cards (2026-10-27, fixed)

The guide's source scanner treated an array type's semicolon as a declaration
terminator, truncating YcbcrToRgbF32::new after `[CodePlane;`. It now tracks
parentheses, array brackets and generic arguments, including const blocks. The
pinned converter snapshot was regenerated from identical source hashes; existing
assertions remain, with a regression for arrays and const-generic return types.

## Completion admission regressions (2026-10-28)

The independent policy review reproduced header publication from an empty
`StagedEncode` without invoking its resolver. Completion now refuses before
draining when no frame has been admitted. Replacing the deprecated preset
after accepted input also refuses on the next push or finish, before final
destination publication. Empty-container admission is not implemented.

A controlled decoder could also return `Done` dimensions or format inconsistent
with its adopted interpretation while all sink calls matched. `Fused` now
rejects that successful completion with `InconsistentFacts` and remains terminal,
matching the coroutine helper. This does not roll back already delivered rows
or a sink finish callback. Regression controls live in the two testkits'
`policy_completion_review` and `policy_final_facts_review` targets.

## Known Bugs: compiler catalog omits inherent dyn methods (2026-10-28, fixed)

The site omitted all three `SourceEncodingDetails::codec_details` trait-object
forms because neither rustdoc JSON nor cargo-public-api emitted them. An explicit
source supplement now preserves their declarations, docs and links; drift guards
refuse stale supplementation. Actual Rust consumer tests cover all three forms.
Existing compiler-surface and browser assertions remain intact. See
`benchmarks/source_catalog_2026-10-28.json`.

## Paired precision CI integration (2026-10-28)

The first paired-precision CI run passed platform tests and native JPEG/TIFF/JXL
and AV1 tests, but failed strict linting on a deprecated zenmeta test import and
an enlarged inline conversion-plan enum. The import now uses `zenpixels::hdr`;
the validation harness deliberately keeps its plan inline rather than adding
an unadmitted heap allocation. No behavioral test expectation changes.

Semver-checks' isolated current-crate placeholder does not inherit the workspace
patch for unpublished zenpixels. `scripts/check-paired-semver.py` generates the
current rustdoc in the actual workspace, then compares it against the unchanged
registry baseline. Both feature configurations and testkit remain checked;
this does not establish registry package availability. Publish the paired
pixel dependency before publishing a crate that requires it.

Local strict workspace and AV1 Clippy plus all three semver configurations pass.
Both rustdoc inputs use Rust 1.98.1, whose JSON format is supported by the
installed semver-checks. The approved 0.2.0 bump is semver-major: the tool skips
compatibility lints; a green version check is not a backward-compatibility claim.
Receipts and the two failed orchestration attempts are retained in
`benchmarks/precision_ci_2026-10-28.json`.

The testkit's former semver gate used heuristic feature inclusion. Both current
and published testkit manifests are featureless; the new default comparison
fails closed if either acquires features or optional dependencies, requiring an
explicit gate update rather than silently reducing coverage. Baseline metadata
also rejects inherited non-registry dependencies and records the graph resolved
for this run; it does not claim to reconstruct a historical lockfile.

## Workspace admission deadline (2026-10-28)

`PolicySet::check_workspace` exposes the existing shared workspace predicate for
adapters to run before native preparation. Unknown refuses every configured
limit; named `UnmetRequirements` survives typed error erasure. Success neither
clears other requirements nor permits bare final adoption. Core admission,
binding, strict Clippy and guide controls pass; the actual native call-order and
metadata-completeness controls belong to the retained publication adapter and
remain a separate integration gate. See `docs/spec.md` and
`benchmarks/workspace_admission_2026-10-28.json`.


## Known Bugs: final publication cancellation (2026-10-28)

Source review found that cancellation raised by the final patch write could
still be followed by cursor restoration and flush in `row_encode`; the
multipass experiment also lacked its post-finish and pre-flush checks.
Fixed with checks at those boundaries and adversarial writer controls, retaining
confirmed byte accounting. Both pre-fix controls failed as expected; three
production patch controls, 33 public multipass controls and strict Clippy pass.
The receipt preserves an initial stale shared-target artifact failure; independent
target rebuilding qualified the corrected source.


## Known Bugs: preparation workspace admission (2026-10-28, fixed)

The planned row driver previously called `plan_encode` before refusing an
unknown strict workspace bound. It now checks both operation-entry bound and
legacy requirements before source declaration or native planning. The same
requirements remain enforced at final selection. Typed and erased controls
refuse before any planning event or managed allocation and retain named
`Workspace(Unknown)` causes. See `benchmarks/row_preparation_2026-10-28.json`.

The code-catalog generator also assumed every workspace used its local target
folder. It now reads Cargo metadata's target directory, honoring configured
and environment overrides. Regenerated catalogs and the site source checks pass.

## Replay fixture portability (2026-10-28)

CI on Windows x64/ARM64 found the new raw-disk replay fixture assumed `HOME`.
It now also accepts `USERPROFILE` and closes its owned file before removal.
The three raw-disk behavioral checks remain enabled with unchanged assertions.
Windows runtime qualification awaits the CI rerun; Linux controls already passed.

## Full Rustdoc reference links (2026-10-28)

The new all-feature eight-crate HTML reference exposed seven unresolved short
links in media output module documentation. They now use explicit
`crate::output` targets; signatures and behavior are unchanged. Strict Rustdoc
warnings remain enabled; final reference generation verifies the correction.

## Fixed reference navigation overflow (2026-10-28)

Adding the Rustdoc reference link exposed a 768 px header overflow. The main
navigation now wraps at all widths and the header grows with its contents.
The existing browser no-overflow assertions remain unchanged.

## Known Bugs: integration guide identity (2026-10-29, fixed)

Repinning the native JPEG harness to zenpipe e5cd3f9a without recapturing
docs/integration-api.json made the guide build refuse its stale revision.
Recapture from the pinned commit preserves the signatures and identity guard;
the full source/browser gates pass. Receipt: benchmarks/guide_snapshot_2026-10-29.json.

## Fixed: video workspace requirements after native effects

PolicyDecoder previously reached native configure/send/receive before refusing
an unknown required workspace bound. The regression proves configure returned
success. Both bound and legacy requirements now guard every native operation;
try_new admits a factory before creation. Wrapping a preconstructed decoder
cannot cover allocations already made. See docs/video-workspace-admission.md.

## Fixed: cancellation during video factory construction

The new PolicyDecoder::try_new initially checked only before its factory. A
constructor that signalled timeout could return an admitted decoder. The added
post-success poll drops that unpublished owner; native constructor errors stay
primary. Regression and scope: docs/video-workspace-admission.md.

## Fixed: staged workspace requirements after mux preparation

StagedEncode::with_staging initially prepared its mux before checking required
workspace at frame/finish policy. The new gate precedes configuration reads,
track creation and mux preparation. Supplied object construction remains outside
that boundary; explicit unknown bounds refuse. See docs/video-workspace-admission.md.

## Fixed collector capacity panic (2026-10-29)

`CollectSink::begin` formerly allocated decoder-supplied dimensions with `vec!`,
which panicked when a representable usize exceeded Vec's capacity limit. It now
checks row/total arithmetic and reserves fallibly before committing state.
The regression covers refusal and successful reuse; this is not a RAM budget.

## Coroutine finalization panic (2026-10-29, fixed on PR177)

A panic in sink.finish could leave the coroutine resumable. Finalization now
poisons state before sink callbacks; caught panic remains terminal. Pending owned
completion details are discarded on failure/cancellation/unwind and cannot be
retrieved. Native streaming detail production is still a separate qualification.

## Default media testkit build (2026-10-29)

The default workspace all-target gate exposed std-only output/staged imports in
`zencodec-media-testkit/tests/policy_completion_review.rs`. The testkit now enables
media/std via its dev dependency, so default tests execute those controls without
changing production defaults. No assertions or target coverage were removed.
See `benchmarks/release_workspace_2026-10-29.md`.

## Collector completion integrity (2026-10-29)

CollectSink formerly accepted missing rows at finish and returned writable rows
after finish. Fallible coverage storage now distinguishes unique supplied rows
from rows_written (which intentionally counts duplicates). Out-of-order and
overlapping strips remain valid; missing coverage and terminal writes refuse.
Three independent failing-first controls and the full I/O feature suite pass.
See `benchmarks/collector_coverage_2026-10-29.md`.

## Controlled destination failures (2026-10-29)

Fused previously guarded interpretation refusals but could accept Done after a
backend swallowed an underlying sink error. The guard now retains the typed
sink cause, blocks further sink effects and keeps the decoder terminal. All four
sink hooks have an adversarial control. Before/after root-package exports sharing
a Cargo target can reuse stale executables: separate targets or force a package
rebuild, and verify compilation paths before accepting evidence. See
`benchmarks/controlled_sink_errors_2026-10-29.md`.

## Channel publication after cancellation (2026-10-29, fixed)

The Tokio bridge checked Pulse only after a non-ready channel attempt. A cancelled
strip sink could publish Begin into an available channel. The helper now checks
before every attempt; the failing-first driver control asserts zero publication.
Backoff caps requested sleep, not wall-clock cancellation latency.

## Collection/render workspace admission (2026-10-29)

CheckedCollection opening and subsequent native calls, plus direct rendering,
previously reached effects before refusing unknown required workspace. Bound
and legacy workspace checks now precede factory/discovery/decode/transform and
color preparation. Request-local refusal preserves the collection owner;
unconstrained rendering stays available. Three failing-first controls, existing
renderer controls and core tests qualify the fix. See
[validation](benchmarks/render_workspace_2026-10-29.md).

## Incremental testkit pre-cancellation gap (2026-10-29)

The trip-after-two probe could complete before cancellation was exercised.
The shared incremental helper now independently requires pre-cancellation and
pre-timeout to preserve their reasons, invoke no sink hooks and remain terminal.
Mutation controls catch ignored stops, early sink effects and reason collapse;
how-far-really diagnostics also detect a swallowed stop. This is testkit coverage,
not proof of native in-flight responsiveness. See
`benchmarks/testkit_precancellation_2026-10-29.md` and `just testkit-pulse-check`.

## Retained preparation cancellation precedence (PR183)

A successful `prepare_output` could signal a stop and return mismatching facts;
required-output admission returned a descriptor error before its post-call Pulse
check. The check now precedes comparison, retaining Cancelled/TimedOut and native
failure priority. The regression also checks no reconstruction and exact owner
cleanup. See docs/decode-descriptor-admission.md for the boundary contract.

## Retained preparation conformance gap (2026-10-29)

The cross-path testkit did not invoke the new optional output preparation hook.
It now checks typed and erased prepared owners against ordinary decoding for
exact pixels/descriptors, attached color context and metadata. Known facts must
match actual output; unknown facts do not excuse changed results. Reference
known-fact and minimal unknown-fact paths, mutation controls and native PNG
RGB/RGBA parity qualify this scope. Preparation allocations, all preferences and
arbitrary late metadata remain separate; see zencodec-testkit/README.md.

## Compile observer descendant lifetime limit (2026-10-29)

Published zenbench 0.1.11 Confirm qualifies the resident CPU control but marks
all 15 owned short-lived-child invocations invalid. Its live-descendant accounting
cannot establish cleanliness for this Cargo-like workload. Preserve those flags;
do not rerun the old Iterate plan and call it confirmation. The control, portable
plans and exact counts live in benchmarks/compile_observer_2026-10-29.md.
