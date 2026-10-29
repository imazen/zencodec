# Checked encode orientation qualification

Core source 777ea11f9fe18dd9718899a65536d27d94b7fd2b; native AVIF source
d07d51618970185875df4b7bde69cf0d17724a98. Consumer fixture graph is zenpipe
9e2a0350, with local source replacements for this exact core/native pair and
unchanged pixels f23ac753. Native tests are the complete orientation_bake.rs,
compiled as a consumer integration target with only its feature guard mapped to
avif-decode/avif-encode. No assertion or existing default control changed.

Serial run-heavy --mem 3G --jobs 2, timeout 900, completed rc0 in 22s;
peak-RSS 0.46GiB, min-avail 11469MiB, peak-load 0.56. These wrapper observations
are not benchmark comparisons or native workspace bounds.

- Core library: 674 passed.
- Core doctests: 26 passed, 37 existing ignored across the two rustdoc batches.
- Generic caller example: compiled; strict core Clippy passed.
- Native orientation target: all three tests passed. Matrix covers eight
  stored transforms, strided pixels, conflicting native overrides, typed/dyn/
  ordinary registry/Send registry paths, exact stored sample and byte parity,
  coded geometry, default bake parity, and typed animation-bake refusal.
- Strict consumer Clippy passed; all eight public API catalogs regenerated.

Earlier failures retained: missing export lock (no compile), two omitted core
capability-match arms, moved PixelSlice test views, and missing codec identity
in the new animation error. The final error wraps native Error::UnsupportedOperation
before CodecError::of; it preserves the original typed operation without panic.
No unrelated native algorithm or existing test expectation changed.

Native per-frame baking, independent reader conformance, retained orientation
plan evidence, memory bounds, and other codec implementations are not qualified
by these controls. Zero zenpixels API changes were required.
