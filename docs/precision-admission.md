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
// Proposed transport, not yet added to zencodec's declaration:
declaration.with_exact_depth(source_claim);

// Paired zenpixels verifier under validation:
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

The next core prototype transports optional evidence through `EncodeDeclaration`
and the row source's `Description`, preserving it from `Raster` and checking it
before forwarding rows. It must retain source revalidation and typed terminal
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

The shared verifier is being qualified in the paired zenpixels release. The
zencodec transport and native AVIF exact-route qualification remain open under
[the existing fidelity issue](https://github.com/imazen/zencodec/issues/104).
