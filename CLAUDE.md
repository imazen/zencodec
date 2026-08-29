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
