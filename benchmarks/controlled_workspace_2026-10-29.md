# Controlled decode workspace deadline — 2026-10-29

Base: `a3b8928adf2d296e38df6a3ad129e04a26a2babd`.

The new native-coroutine regression fails on the base: its first empty-input
step returns `Progress { consumed: 0, event: Need }` despite a mandatory unknown
workspace bound. Before-fix receipt: rc101, 20s, peak RSS 0.71 GiB, minimum
available 13096 MiB. This witnesses late admission, not a measured allocation.

The fix checks both bound and legacy requirements before native polling and
retains `UnmetRequirements` as a typed error. The same test covers limits 0 and
u64::MAX, both policy placements, no callback/decision record, zero native decoder
constructors/batches, zero sink effects, released session storage and terminal
subsequent calls. Existing native final-boundary controls still cover every
requirement; workspace now requires earlier refusal, all other cases retain their
original discovery/final-decline assertions.

`just controlled-workspace-check` passes in an isolated export: 10 native control
tests, all zencodec-io feature tests/doctests, and strict Clippy for the native
harness and IO all-target/all-feature surface. Wrapper: rc0, 44s, peak RSS
0.72 GiB, minimum available 13215 MiB, peak load 1.53. Both exports force an IO
package rebuild to prevent stale root-package artifacts from a shared target.

This is not a total-memory measurement. Native construction occurs before the
wrapper receives the decoder; its admission remains an unresolved factory
contract. Metadata discovery remains available without a workspace requirement.
No whole-file privacy or arbitrary deferred-obligation guarantee is added.
