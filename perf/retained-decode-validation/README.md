# Retained decoder output preparation (review candidate)

`Decode::prepare_output(&mut self)` optionally reports the exact descriptor of
that configured decoder before raster/coefficient reconstruction. Default `None`
preserves existing codecs. `DecodeRequest` calls it only for required output,
refuses a known mismatch, then checks actual decoded output regardless. This
avoids an extra caller query or a second independently configured decoder.
Native errors, retained cancellation and cleanup survive both refusal paths.

The native patch uses JPEG's existing descriptor selection and full metadata
scanner, including late/split ICC. It does not implement a second descriptor
formula. Preparation can still allocate metadata; it is not a workspace bound.
Only ordinary 8-bit gray/RGB input is qualified. Crop, CMS, transform, gain-map,
non-default native target, forced float IDCT, XYB and other input precision
return unknown. Required output still receives the final check on these routes.

## Reproduce

Use an isolated export outside workspace Cargo overrides. Supply a local JPEG
git checkout containing the pinned revision; the script never edits it.

```sh
python3 perf/retained-decode-validation/prepare.py "$CORE_REPO" "$CORE_REV" "$JPEG_REPO" "$EXPORT"
cd "$EXPORT/core"
# Review native graph/lock changes before execution; only the local JPEG family
# substitution is intended. Preserve the committed baseline dependencies.lock.
just decode-output-admission-check
JPEG_POLICY_ARTIFACTS="$ARTIFACTS" cargo test --manifest-path perf/jpeg-policy-validation/Cargo.toml --features retained-output --locked --lib
cargo clippy --manifest-path perf/jpeg-policy-validation/Cargo.toml --features retained-output --locked --all-targets -- -D warnings
```

The script and commands require the normal serialized resource wrapper.
The `retained-output` feature enables an explicit native witness; never run it
against the unmodified native pin and interpret a default unknown as success.

## Evidence and costs

[Measured cells](../../benchmarks/retained_decode_2026-10-29.json) retain source,
binary and input identities. At 1024x768 RGB8, unsupported RGB16 refusal changed
peak heap from 2,820,159 to 282,563 bytes for baseline JPEG and from 5,243,597 to
272,384 bytes for progressive JPEG. These are whole-process debug heaptrack
observations, not limits a policy may rely on. Equal-length executable and fixture
paths remove the observed argument-storage difference between earlier runs.

Successful required output kept the same peak heap, but made **four additional
allocation calls**: 74→78 baseline, 120→124 progressive. The prototype repeats
header/metadata work during execution; runtime cost has not been measured. All
six controlled-path cells retained identical peak heap and allocation counts.
Pixel checksums match across all successful paths within each fixture.

The native matrix also compares repeated preparation against actual output and
unprepared execution for seven layouts, three ICC placements and two scan modes.
A crop control requires unknown and successful execution. The complete native
suite passes 61 tests; two core tests cover early refusal, unknown fallback,
lying facts, retained native error causes, cancellation and owner cleanup.
All 682 core library tests and strict Clippy on both graphs pass. These witnesses do not qualify all JPEG
configurations or prove other codecs can prepare without decoding.

This remains a review API and a native patch experiment. Native adoption, API
review, full workspace/target checks are required
before landing. Do not publish a release from this experiment.
