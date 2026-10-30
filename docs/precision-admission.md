# Precision evidence before encoder startup

```text
Canonical 10-bit code 9 -> nearest-scaled U16 577
Pinned AVIF U16 577     -> truncate to code 9 -> replicated U16 576
```

These arithmetic paths disagree even if AV1 preserves every coded value. The
four controls in `just precision-grid-check` execute this counterexample,
full-range grid non-nesting, narrow-color versus full-scale alpha, and strided
padding exclusion. They do not run a native encoder or decoder.

The reviewed sources are [zenpixels PR 95 at 0bdc0db5](https://github.com/imazen/zenpixels/blob/0bdc0db524cb61fa2289abcd32999af7aed0b4ee/zenpixels/src/sample/exact.rs)
and [zenavif at a7638f6e](https://github.com/imazen/zenavif/blob/a7638f6e0655914fb15fe4633ac26425533756c8/src/convert.rs).
This review describes that pinned graph; it does not establish current native
round-trip results or authorize changing its existing reconstruction behavior.

## Keep three questions separate

```rust,ignore
// Review API: retain the assertion through planning and declared startup:
declaration.with_exact_depth(source_claim);

// Verify actual samples before relying on the assertion:
source_claim.holds_for(&actual_rows)?;

// Neither statement proves that a codec's transforms and reconstruction
// preserve these samples. That requires the actual retained native route.
```

An `ExactDepth` tag can be asserted without reading samples. Merely retaining
that tag on an immutable raster does not prove its truth. A verifier must examine
active samples, distinguish alpha from color, ignore padding, and reject
unsupported layouts. A stable measured raster can supply preflight evidence;
a replay scan requires a stable source or snapshot. Planning itself must not
silently read pixels, add a pass, or allocate an image.

The review API transports optional evidence through `EncodeDeclaration`
and the row source's `Description`, preserving it from `Raster` and checking it
before forwarding rows. It retains source revalidation and typed terminal
failures. No public `CodedSamples::depth_exact` boolean or new policy enum is
proposed for landing: canonical grid membership alone cannot certify codec
fidelity, channel preservation, color conversion or native reconstruction.

## A late contradiction still has effects

```text
plan -> begin/header -> check strip 0 -> check strip 1 -> reject strip 2
```

The rejected strip must never reach the encoder. A live route may already have
published earlier bytes; its error must report those effects without retrying at
another depth. Finish-first staging can prevent final-destination publication
on this failure. It does not prevent private native startup or establish a
workspace bound. Before-start proof of every sample requires a separately
admitted scan over stable retained data.

Qualification must cover typed and erased planning, false initial claims,
changing source declarations, late bad samples, stride and RGBX padding,
narrow-color/full-alpha separation, sample-changing processing, and both
publication schedules. Native controls must compare actual decoded canonical
samples and independently inspect coded depth, range, matrix and subsampling.
Keep existing tolerance tests; add exact assertions for exactness claims.

The shared verifier and zencodec transport have the qualification below. Native
AVIF exact-route qualification remains open under
[the existing fidelity issue](https://github.com/imazen/zencodec/issues/104).

## Integration controls on the paired verifier

The implementation uses [zenpixels 283655c5](https://github.com/imazen/zenpixels/tree/283655c5424e0ad7ebc6d2acd94670fac1e29158),
whose [CI run](https://github.com/imazen/zenpixels/actions/runs/40668717147)
passed all 13 jobs. This is upstream verification; the separate zencodec and JPEG controls are
recorded below. It does not qualify a native codec's lower-depth selection.

```rust,ignore
// None: do not assert a reduced canonical grid.
let source = Description::new(width, height, descriptor);

// Some: an assertion the driver must check, even if the codec ignores it.
let source = source.with_exact_depth(claim);
```

`Some` is strict. The paired verifier supports U8/U16 gray, gray-alpha, RGB,
RGBA and BGRA, including undefined RGBX/BGRX padding. An unsupported layout
refuses before encoder startup. Attaching even a full-storage CMYK claim
therefore changes admission. `None` does not prohibit CMYK or require a
lower-depth encoding. `Raster` preserves an attached claim; planning never
silently discards one after a policy or codec could have relied on it.

An application that intentionally makes no claim can construct another borrowed
view without copying samples:

```rust,ignore
let mut unclaimed = PixelSlice::new(
    pixels.as_strided_bytes(), pixels.width(), pixels.rows(),
    pixels.stride(), pixels.descriptor(),
)?;
if let Some(context) = pixels.color_context() {
    unclaimed = unclaimed.with_color_context(context.clone());
}
let source = Raster::new(unclaimed);
```

The row driver verifies context-free row views, then attaches the original
color context before push. This avoids cloning the color-context `Arc` for
each verification row; no speedup has been measured. Verification adds a sample
scan when evidence is supplied. It adds no source pass or pixel-buffer copy.

## Qualification and migration

The paired pixel field increases `DecodeOutput` from 360 to **368 bytes on
x86_64**. Its exact size guard was updated with user approval. This is object
layout, not a measured heap or process-memory increase. The isolated dependency
resolution passes all 14 graphs, including the migrated native IMI harness,
with one zencodec, zenpixels and how-far identity in each.

Executed controls cover structural admission before startup, typed and erased
forwarding, false initial and later claims, source/policy/writer mutation,
per-row cancellation, strided U16, narrow color with full-scale alpha, RGBX
padding, and preservation of inherited startup refusals. The entire bad strip
is withheld from the encoder. Earlier Live output remains recorded in the
failure. A disk finish-first spool may retain private written bytes on error;
zero final-destination publication is not rollback of source reads, native work
or caller-owned spool contents.

The native JPEG control compares claim versus no-claim using raw, typed-job and
erased-job entry points in baseline and progressive modes. All six configured
routes preserve encoded bytes and independent `djpeg` output. JPEG remains
lossy; this establishes evidence transport without a configuration change,
not preservation of the original uncompressed samples or automatic depth
selection. Native AVIF narrowing/reconstruction remains unqualified.

The full workspace all-target/all-feature tests, doctests and focused strict
Clippy also pass. Commands, source identities and resource receipts are in
[the qualification record](../benchmarks/precision_cohort_2026-10-28.json).
Those build/test receipts are not throughput or native-memory measurements.

`EncodeDeclaration::new` and `Description::new` default to no assertion.
`EncodeDeclaration` is non-exhaustive. Existing exhaustive row-source
`Description` literals must add `exact_depth: None` (or propagate a justified
claim); constructor users need no change. No codec method signature changed.
The paired zenpixels verifier is pinned and unpublished; release qualification
must use the final published dependency graph.

## Shared sample-description migration boundary (2026-10-30)

Core still uses `zencodec::frame::SampleEncoding` in frame interpretation and
plane APIs, while conversion uses `zenpixels::sample::SampleEncoding`. The
compatibility type is intentionally retained for the current release; it is
not a separate sample normalization scheme.

`just sample-encoding-compat-check` compares every `u8` depth/shift pair for
both U8 and U16 storage (131,072 descriptions), matching accepted fields and
all three typed refusal categories. Another 65,536 descriptions verify F32
refusal. The legacy `U8` constant matches canonical `new(U8, 8, 0)`; its
`bytes_per_sample()` convenience has no corresponding method on the pinned
canonical type. Error display text differs and is not asserted identical.
The two differential tests and strict Clippy pass against paired pixels
`f23ac753`, using an isolated export based on core `7cde6916` plus the new test.
Wrapper: `rc=0 3s | peak-RSS 0.29GiB | min-avail 13540MiB | peak-load 0.05`.
These are validation resources, not conversion performance.

The native AV1 validation harness currently reconstructs the canonical
three-field description in `coupled_route.rs` and `prepared_conversion.rs`.
Consolidating the public types would remove that reconstruction, but still
requires migration of frame/plane signatures and native adapters, plus an
explicit choice for the two conveniences. This gate establishes constructor
compatibility only: it does not qualify that migration, all storage variants,
actual sample extraction, range conversion or codec exactness. In particular,
it does not resolve the 577-versus-576 reconstruction counterexample above.
