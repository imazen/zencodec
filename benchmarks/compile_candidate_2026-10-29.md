# Compile comparison: PR175 increment

Compared pre-merge main `87eff5f162042aa46c50df3f8abd3745da0001b8` with merged
candidate `aa30ebba1aef5eadfd0b1f82a8c308e1e9e3a8cd`. This is not a crates.io comparison.

| Build | Before median seconds | Candidate median seconds | A/A median seconds |
|---|---:|---:|---:|
| dev/default | 2.94 | 2.96 | 2.93 |
| dev/std | 3.05 | 3.07 | 3.05 |
| release/std | 4.35 | 4.40 | 4.36 |

All 45 builds completed, five blocks per case with A/A controls. These are
exploratory observations, **not confirmed performance evidence**: all 45 invocations
have CPU-load flags and three have heavy-process flags. No observations were removed;
the cause of those flags has not been established. Both arms used identical resolved
locks. Scope is a core/io/media re-export consumer, excluding codecs, rendering/CMS
and application monomorphization; fresh targets, warm OS cache, prefetched dependencies,
two jobs. It does not measure encoder memory or whole-application compile cost.

Wrapper: rc=0, 178s, peak RSS 0.31 GiB, minimum available 14578 MiB, peak load 1.87.
[Machine-readable receipt](compile_candidate_2026-10-29.json) preserves source identities,
commands, hashes, unit timings and compiler RSS. Full packet and logs remain at the
paths in that receipt. Completed target directories were deleted only after retaining
Cargo timing reports. No compiled executables were archived.
