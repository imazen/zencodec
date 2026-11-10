# Native CMYK through explicit LCMS

This isolated graph uses the root pixel/converter76005fc9 cohort with the
opt-in Rust LCMS backend. It changes no core signature or product default.
The earlier af19 graph and failing Mox receipts remain historical evidence;
they are not relabeled as current-cohort measurements.

Run `just spatial-lcms-check <fresh-artifacts> <independent-C-oracle>` from an
export outside inherited workspace overrides. Build the independent C oracle
with `perf/spatial-validation/build_color_oracle.py`.

The LCMS test retains the native JXL source ICC, inverted F32 polarity, strided
staging, metadata/source preservation, owner accounting and bit-exact
finalizer/direct-backend checks. Every RGB component must satisfy the unchanged
strict absolute error <0.0013 against independent LCMS relative+BPC output.
The build script enables this additional backend gate in the shared test source.
Its focused runtime gate and strict all-target Clippy pass on the current cohort.

The Mox test separately retains direct-backend parity and explicitly reproduces
the known independent accuracy discrepancy, as approved by the owner. Its JSON
report marks accuracy unqualified; no higher error bound is accepted. See the
[current gate split](../spatial-validation/README.md#current-shared-cohort-and-cmyk-gate-split-2026-11-10)
and [preserved original measurements](../../benchmarks/native_cmyk_finalization_2026-11-08.md).
