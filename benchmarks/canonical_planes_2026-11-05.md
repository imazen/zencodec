# Canonical plane migration qualification — 2026-11-05

Core source: `85a7c8fe514b57938ec139ac53ea670104938612`.
zenpixels and zenpixels-convert: `fde59b96632557592cffa9b7301acd8db96ed078`.
Standalone git export outside workspace path overrides; aarch64 Linux.

Commands, serialized under run-heavy 3G/jobs2 with a 900-second outer timeout:

```sh
cargo check --workspace --all-targets --all-features --keep-going
cargo test --workspace --all-targets --all-features
cargo test --workspace --doc --all-features --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
```

All commands passed. Aggregate test/doctest summaries: 1451 passed, zero failed,
39 existing ignored cases. These counts are not completeness evidence. The four
new `plane_layout` controls cover mixed-depth alpha, F32 scale/offset and exact
NaN carriage, odd-width CbCr/CrCb component addressing, and contradictory owner
encoding refusal. Pointer checks cover sample-buffer borrowing, not total
allocation freedom.

Wrapper receipt: `rc=0 183s | peak-RSS 1.10GiB | min-avail 26304MiB | peak-load 7.96`.
This is correctness qualification, not a performance benchmark.

Earlier attempts failed at dependency resolution or compilation (old ICC
helpers, converter convenience and benchmark constructor). They ran no affected
runtime controls; logs were retained separately. No behavioral assertion was
relaxed. Subsequent floor corrections require zenpixels >=0.3.1 and <0.5.0;
the tested resolved dependency remains identical.

## Remaining gates

Renderer, native AV1/spatial adapters, generated API catalogs and site controls
are separate gates. Native AV1 still needs its plane mappings migrated; JXL
pins exclude the converter's archmage prerelease and use removed orientation
helpers. Core success does not qualify those integrations or every platform.

## Renderer

Source `427390a4` passed no-default-feature check, all-target strict Clippy,
unit tests and doctest after targeted archmage resolution to 0.9.31-beta.2.
Receipt: `rc=0 24s | peak-RSS 0.37GiB | min-avail 26921MiB | peak-load 6.52`.
The initial stale-lock resolution failure is retained separately. The lock delta
is limited to zenpixels/convert revisions, archmage/macros/magetypes prerelease
versions and the pinned garb/linear-srgb source replacements; no unrelated
package versions changed. Native spatial oracles were not run.
