# Retained preparation reuse experiment

This is a tested native candidate on PR183, **not adopted JPEG behavior**.
The preceding [ARM confirmation](../../benchmarks/retained_decode_arm_2026-10-29.json)
found a small progressive-image regression from repeated preparation work.

`jpeg.patch` retains the configured decoder's completed metadata inspection and
selected descriptor. Repeated preparation checks cancellation before returning
that descriptor. The full metadata scanner accepts the already-validated header;
execution reuses its structural facts instead of reparsing the header again.
The complete primary-image scan and late/split ICC checks remain. No new public
API or caller query is introduced beyond PR183's existing proposal.

After output selection, execution drops the scanned carrier buffers before
native reconstruction. Actual decode still owns output metadata and native extras;
prepared facts do not manufacture a replacement result. Preparation retains its
original dimension/progressive checks before the full scan. Unsupported native
configurations still return unknown and receive final output validation.

## Reproduce

First use PR183 `9efb21077dc446ba2285eb7d71efca80324130e1`'s
`perf/retained-decode-validation/prepare.py` to create an isolated prepared export
from the core and native repositories. That script checks the JPEG/YUV source
substitution against the frozen lock. Then, from current zencodec:

```sh
just retained-metadata-reuse-apply "$EXPORT"
```

The applicator verifies all three input source hashes, dry-runs both patches
before mutation, leaves the Cargo lock unchanged and records resulting identities.
An already-patched or different source refuses. Patch reproduction was checked
byte-for-byte against the tested source; repeated application refuses.

In the export's `core` directory, through the normal serialized resource wrapper:

```sh
JPEG_POLICY_ARTIFACTS="$ARTIFACTS" cargo test --manifest-path perf/jpeg-policy-validation/Cargo.toml --features retained-output --locked --lib
cargo clippy --manifest-path perf/jpeg-policy-validation/Cargo.toml --features retained-output --locked --all-targets -- -D warnings
```

## Qualified behavior and limits

All 62 native harness controls and strict all-target Clippy pass on ARM64.
The existing preparation matrix retains seven layouts, three ICC placements,
baseline/progressive input, repeated preparation, crop unknown fallback and exact
sample/metadata equality. Additional assertions compare native retained segment
bytes and types. Eight cancellation cases stop after successful preparation,
then require the original Cancelled/TimedOut category from cached preparation
or decode. Existing assertions and feature coverage were not relaxed.

The final wrapper reports rc0/9s/peakRSS0.32GiB/minavail14453MiB/load1.00 under
3GiB/two jobs/timeout900. Initial correctness passed in80s/peakRSS0.67GiB;
explicit carrier-release refinement passed in29s/peakRSS0.71GiB. These are build
and validation resources, not decoder memory measurements. Full logs remain
at `~/tmp/retained-reuse-20261029/{check,check2,check3}.log`.

**Runtime and allocation improvements remain unmeasured.** The inline cached
owner changes decoder layout; retaining full metadata between preparation and
execution can change peak memory. Early release is not a numeric bound. Before
native adoption, repeat matched heap profiles and the unchanged five-process,
30-round, 1% runtime protocol; compare ordinary decode too. Do not infer that
removing two parses eliminates the observed regression. Native main checkouts
and their unrelated work remain untouched.
