# Native CMYK through explicit LCMS

This isolated graph compiles the existing spatial controls against zenpixels
`af19bcdab06b0df55745288931a1bdfaa6232665` and its opt-in Rust LCMS backend
`55b59476cf64920345e9ca32eb3a4678bf0bbece`. It does not repin the frozen spatial
harness, change a core signature, or select a new product default.

Run `just spatial-lcms-check <fresh-artifacts> <independent-C-oracle>` from the
repository root, outside inherited workspace overrides. Build the C oracle with
`perf/spatial-validation/build_color_oracle.py --help` (the pinned C 2.20 source
used in the receipt is independent of the production Rust implementation).

The same native JXL image, source ICC, inverted F32 polarity, strided staging,
metadata/source preservation, owner accounting, direct-backend parity and
strict absolute error <0.0013 controls apply to both backends. The build script
only enables the additional LCMS test in the shared test source. The frozen
manifest cannot accidentally enable an unavailable LCMS dependency through
`--all-features`.

The original Mox accuracy test remains enabled and fails on both known graphs;
a full run here is not expected to be green. The focused LCMS command is an
explicit backend qualification, not a workaround asserting that Mox meets the
bound. See [the numerical receipt](../../benchmarks/native_cmyk_finalization_2026-11-08.md).
