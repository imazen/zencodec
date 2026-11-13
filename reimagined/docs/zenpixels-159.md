# zenpixels PR 159 adaptation

This review is stacked on [zencodec PR 211](https://github.com/imazen/zencodec/pull/211).
It targets [zenpixels PR 159](https://github.com/imazen/zenpixels/pull/159) at
`240ed47585e7a9c2c94d7cd46a8c44f52d9129f5`, including the owned-planar foundation
in its base. The older root API and its dependency cohort are unchanged.

## Changes

- All three prototype crates and the next-side comparison types use one pixel
  revision. The older side of the comparison keeps its original pixel source;
  its types do not cross into the prototype.
- `PlanarSlice::planes()` now lends an iterator. The collector iterates directly
  and the fixture encoder takes indexed loans; neither builds a temporary
  plane-descriptor vector.
- The standalone dependency graph is tracked in `dependencies.lock`.
  `just prepare` restores an absent lock and refuses a differing existing lock.
  The native and synthetic comparison locks are updated as well.
- The prototype has its own CI workflow. Root workspace CI does not reach a
  nested Cargo workspace. The new workflow covers native Linux x64/aarch64,
  Windows x64/ARM64, macOS ARM64, i686 through cross, strict lint/docs,
  WASM/freestanding no_std and the declared Rust 1.88 minimum.

## Interoperability controls

`zencodec-io/tests/pixel_foundation.rs` exercises the actual collector, encoder
and decoder:

- `ColorContext` retains one current authority. Opaque ICC survives whole-frame
  delivery with the same allocation, while file `Metadata` can still retain
  both ICC and CICP. `PixelEncoding` borrows that context; copying it does not
  change the Arc reference count.
- Tight, cache-aligned, whole-pixel-aligned and custom sample-aligned RGB rows
  produce identical files and decoded samples. A source with only the final
  row's active extent works without a padding read or panic.
- An owned `PlanarBuffer` lends independently strided, odd-sized 4:2:0 planes
  into the encoder. Decoding retains every active sample and the declared
  current color. Source sample addresses and padding remain unchanged.

Existing round-trip, transcode, borrowed-input, optional-Pulse, cancellation,
error-cause and native QOI/BMP/JPEG trial controls remain enabled. No prior
assertions, thresholds or feature gates were relaxed. The existing collector
still supports its original U8/U16/F32 plane storage; this change does not claim
support for every native sample type available upstream.

## Verification

Qualified code: `7375a9e3963fc9e4c147c63ea4da1ecd25e5e332`.
Migration code: `68049bae7a2f`.

Local gates pass:

```sh
cd reimagined
just prepare
just gates
cargo test --workspace --locked
cargo +1.88.0 check --workspace --all-targets --all-features --locked
PROBE_TARGET=/path/to/isolated/output just probe-check
```

Run builds through the workspace's required resource wrapper and use an
isolated `CARGO_TARGET_DIR`. The bitmap trial export still verifies against
its original native source, with only the two documented accessors.

All eight jobs in the first standalone workflow pass, including Windows ARM64,
macOS ARM64, i686 and MSRV:
[Reimagined contract run 43416758667](https://github.com/imazen/zencodec/actions/runs/43416758667).

Commands, source identity, log hashes and resource lines are preserved in
[the receipt](../../benchmarks/zenpixels_pr159_2026-11-13.json). Full logs are
under `/mnt/v/output/zencodec/pixel159-adaptation-2026-11-13/` on r7900x, with
an archive mirror. The initial new-test compilation errors are retained too;
they were corrected without changing any established expectation.

The final documentation head also passes all eight standalone jobs:
[run 43417304883](https://github.com/imazen/zencodec/actions/runs/43417304883).
The older workspace's Rust 1.100 Clippy rejected an index-only loop in a
pre-existing gain-map test fixture. The equivalent iterator preserves every
fixture value and assertion; all 14 focused gain-map tests and strict Clippy
pass under local Rust 1.100 beta. This changes no production metadata API.
The older native-guide jobs retain their documented dependency-resolution
failure and are not qualified by the prototype workflow.

## Limits

The codec contract still delivers borrowed planar strips. Upstream owned and
mutable carriers now exist; an owned delivery hook and retained planar
remainder buffering are not implemented here. `Option<&dyn Pulse>` and worker
`Option<SharedPulse>` remain unchanged.

This is correctness/compilation qualification, not a timing rerun. Prior
performance receipts keep their original sources and must not be relabeled as
measurements of this pixel pin. Complete native color/metadata conformance,
collection/ROI browsing and concurrent InPlace fairness remain separate work.
