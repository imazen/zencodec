# Canonical sample encoding ablation — 2026-10-30

Candidate based on `a51d4119`; source replaces the duplicate core encoding and
error with zenpixels `f23ac753`'s checked type. This is the first structural cut,
not qualification of the complete frame/plane redesign or native codec rollout.

## Commands and results

`just frame-ablation-check` runs core library and focused mapping/replacement
controls; media and media-testkit libraries/integration tests; and all-target
strict Clippy for those three packages. All passed in an exported tree outside
workspace path overrides, with the preceding qualified export's Cargo.lock.
The log explicitly compiled core/media from the new export path.

`run-heavy: done rc=0 6s | peak-RSS 0.39GiB | min-avail 13450MiB | peak-load 0.28`

The exhaustive test retains all U8/U16 depth/shift descriptions and F32 refusal
inputs. Accepted U16 descriptions additionally compare strided native and
canonical conversion views against independently masked source words. U8 views
check source-byte identity. These controls are not codec or performance claims.

The eight-crate compiler catalog regenerated successfully:
`run-heavy: done rc=0 10s | peak-RSS 0.26GiB | min-avail 13362MiB | peak-load 0.32`.
Its nightly rustdoc emitted existing redundant-link warnings; this is not a
strict Rustdoc qualification claim.

Two failed attempts are retained: the first export omitted the ignored lockfile
and `--locked` refused before compilation; a migration edit then dropped a type
name in one replacement test, and compilation rejected it. Neither attempt ran
the full gate. The repaired test preserves its original refusal assertion.

Logs and exact export: `~/tmp/zencodec-frame-ablation-20261030/`. No executable
archives were created. Native AV1 fixture/bridge sources are migrated, but native
adapters in other repositories and their graphs are not yet qualified. Prior
package/platform results do not qualify this breaking candidate.

The matching eight-crate HTML Rustdoc build and all 27 guide/source/catalog
checks passed on `c3a4aae514f4` in a second isolated export:
`run-heavy: done rc=0 7s | peak-RSS 0.34GiB | min-avail 13414MiB | peak-load 0.23`.
Browser interaction tests were not rerun; no UI structure changed.
