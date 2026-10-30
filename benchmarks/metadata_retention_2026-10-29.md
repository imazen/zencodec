# Native mastering metadata retention witness

Core base: `a8d03fbea9488a097025ad6490988a10d07fe429`.
Native graph: `perf/jpeg-policy-validation/dependencies.lock`, unchanged.
JPEG `7c5a9501270210d6ef534c23221ff2660508b7cf`, PNG
`7b6d1436db5d771fc43226c146847fc76305afde`, pixels
`f23ac7535be1144a22d0dc6b71f866a08b151f34`.

97×65 RGB8 fixture; PreserveExact; empty controls and independently supplied
CLL(1000,400), HDR10_REFERENCE mastering display, and both. Six successful
encodes; only PNG's CLL-only route retains all supplied fields on native readback.
The witness exits rc1, explicitly preserving five unqualified routes. It does
not reinterpret arbitrary encode errors as approved refusals.

Retained encoded artifacts: `/mnt/v/output/zencodec/metadata-retention-20261029/`.
SHA256 groups (each member has identical bytes):

- `jpeg-empty.jpeg`, `jpeg-cll.jpeg`, `jpeg-mastering.jpeg`, `jpeg-both.jpeg`:
  `17dbc129a35482c06357b86ff6e95fc2162de37e6390a9c9c5836bf5722d8cf9`
- `png-empty.png`, `png-mastering.png`:
  `d23d5e3d7483722e0294ece2444f7b1f54db87a09a98dcc84ecbdf1074e15694`
- `png-cll.png`, `png-both.png`:
  `6f5ab491cc3649aff1caa399bd29fa3dcabc360afb2a292616e877186223dfcb`

The negative identical-byte controls do not depend on how a decoder interprets
metadata. The positive PNG CLL claim is native readback only; independent
third-party retention qualification is not claimed. No native library, policy,
capability or existing assertion was changed. The experiment is an example
command, not an ignored test or a substitute passing conformance check.

ARM64 Rust 1.96.0, run-heavy 3G/jobs2, timeout900:

- Initial non-artifact run: rc1, 54s, peak RSS 0.69 GiB, minimum available
  13980 MiB, peak load 1.17.
- Artifact-retaining run: rc1, 22s, peak RSS 0.69 GiB, minimum available
  14031 MiB, peak load 0.77; same six observations.
- Initial strict example Clippy: rc0, 23s, peak RSS 0.56 GiB, minimum available
  14160 MiB, peak load 0.69.
- Final artifact-writing source strict Clippy, eight compiler catalogs, strict
  eight-crate Rustdoc reference and 26 site source checks: rc0, 96s, peak RSS
  0.56 GiB, minimum available 14197 MiB, peak load 1.14. No browser layout
  qualification is claimed; the native diagnostic still exits rc1 as above.

These receipts measure validation resources, not encoder performance or memory
bounds. `just metadata-retention-probe FRESH_DIRECTORY` reproduces the observation;
existing artifact files are never overwritten. Logs and runners remain at
`~/tmp/metadata-retention-20261029/`. No compiled executable was archived.

[Contract and proposed replacement](../docs/metadata-retention-admission.md).
