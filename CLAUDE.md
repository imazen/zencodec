# zencodec

Shared traits and types for zen* image codecs.

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

- API comparison (2026-10-25): `FileReadAt` and `BufferedReadAt` stringify native
  I/O failures into `SourceError::Io`, which exposes no cause. The native
  missing-file control in `just io-contract-probes` observes NotFound directly
  but cannot find `std::io::Error` through FileReadAt. Driver error signatures
  are unchanged; see `docs/io-contract-proposal.md#native-error-provenance-2026-10-25`.

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

- Freestanding policy qualification (2026-10-25): the root zenpixels dependency
  enables its default `std` feature. Disabling that edge in an export exposes
  12 `powf`/`floor`/`round` calls in `src/gainmap.rs` that fail on
  `thumbv7em-none-eabihf`. WASM compilation with std available does not certify
  a std-free graph. Tracked in [issue 165](https://github.com/imazen/zencodec/issues/165).
  `perf/policy-check` distinguishes full-crate WASM integration from the isolated
  freestanding resolver check. Root production code is unchanged.

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
