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

## Release Requirements

**CI MUST pass before any crates.io release.** This includes:
- All tests pass on Linux, Windows, macOS
- WASM build succeeds (wasm32-wasip1)
- Clippy clean (no warnings)
- Format check passes
- MSRV 1.93 check passes
- `cargo-semver-checks` passes (no unintended breaking changes)

**Before publishing:**
1. Verify README.md reflects current API
2. Run `cargo semver-checks check-release` locally
3. Bump version in Cargo.toml
4. Get explicit user approval
5. `cargo publish`

## Known Issues

Three bugs verified during the cross-codec color/metadata scenario-matrix
research (2026-06-01). The first is in this crate; the other two are recorded
here as cross-repo findings (do NOT edit those repos from here — flag to the
owner). Full design context: [`docs/color-emit-model.md`](docs/color-emit-model.md).

1. **Double-rotation hazard — FIXED (this crate, `src/metadata.rs`).** When a
   decoder bakes orientation upright it sets `Metadata::orientation = Identity`
   while the EXIF blob still carries the original `Orientation` tag (e.g. `6`); a
   consumer that re-applied the tag would rotate twice. `Metadata::filtered` now
   reconciles them — it rewrites the embedded tag to match the authoritative
   `orientation` field via `helpers::set_exif_orientation` (offset-preserving,
   fires only on a mismatch so the matched case keeps the zero-copy `Arc` clone).
   Regression: `filtered_reconciles_baked_orientation_tag`.

2. **AVIF descriptor-CICP override — FIXED (zenavif main, `b3be82a`,
   2026-06-10).** `apply_descriptor_color` no longer overrides a
   caller-supplied CICP from `Metadata`; landed with the zencodec 0.1.21
   color-emit adoption ("AVIF nclx now sole-safe").

3. **Missing signal-range conversion kernels (zenpixels-convert) — HARDENED
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
