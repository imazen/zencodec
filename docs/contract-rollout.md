# Shared-contract rollout

Observed 2026-10-28. Merging these changes does not publish a crate or establish
release completeness. The [implementation record](ideal-api.md) and
[multipass gates](../perf/multipass-validation/README.md#production-promotion-gates)
retain the remaining design and qualification work.

## Verified merges

| Repository | Reviewed change | Tested source | Remote main contains it |
| --- | --- | --- | --- |
| zencodec | [PR 168](https://github.com/imazen/zencodec/pull/168) | `f552d338fc5af8dbc623129f3040b65c9f074a71` | Merge `f02a17ec6001a9efcad1267f92a3ea928ed06e57`; ancestry verified |
| zencodec | [PR 174](https://github.com/imazen/zencodec/pull/174), shared pull rows | `426804c8ef4840edc0ed5e7f6b1fe5e76bf11908` | Merge `9f9bd3cecf05c3b2cdcd7ee70a6a6da4f8b923df`; ancestry verified |
| zenextras | [PR 54](https://github.com/imazen/zenextras/pull/54) | `3e5267e363e339a2af86c6bb838e3ab0c4a7f0b2` | Merge `635609fba4bf073ff68a1e1ccb55fa3dd170b1d8`; remote comparison verified |
| zenbitmaps | [PR 34](https://github.com/imazen/zenbitmaps/pull/34) | `9ad4582bc51b6259829a421fe1e6564f617d31ce` | Merge `9cab06f86eb6582d2253410285f38a76185ba39b`; remote comparison verified |
| zenjxl | [PR 31](https://github.com/imazen/zenjxl/pull/31) | `f2c0177e153c9308042bfd5fe51a4be1ebac11b0` | Merge `bf3a4a801cd293bb59b13ecf3791c50c0acf9059`; remote comparison verified |

Before merging, every job passed in zencodec's
[core CI](https://github.com/imazen/zencodec/actions/runs/40767258138),
[native/guide qualification](https://github.com/imazen/zencodec/actions/runs/40767258147)
and [fuzz build](https://github.com/imazen/zencodec/actions/runs/40767258149),
zenextras' [CI](https://github.com/imazen/zenextras/actions/runs/40766323747),
zenjxl's [CI](https://github.com/imazen/zenjxl/actions/runs/40429930768),
and zenbitmaps' [CI](https://github.com/imazen/zenbitmaps/actions/runs/40624740502).
The native/guide workflow skips deployment on a PR; that is distinct from its
successful source, browser and native-oracle checks. Core semver acceptance is
for the intended breaking version, not a backward-compatibility certificate.

## Paired pixels stack

[zenpixels PR 95](https://github.com/imazen/zenpixels/pull/95), tested at
`283655c5424e0ad7ebc6d2acd94670fac1e29158`, passed
[its manually dispatched complete CI](https://github.com/imazen/zenpixels/actions/runs/40668717147),
including API snapshots, feature powerset, i686 and Windows/macOS ARM.
It merged into the `output-plan` review branch at
`9dbbdde8a4971d66deda43d6a06ed70022ac7f22`. The combined
[PR 92](https://github.com/imazen/zenpixels/pull/92) passed every job in
[CI on that exact head](https://github.com/imazen/zenpixels/actions/runs/40769496153)
and merged to main as `7d28c4b73ab7af01d79079af30a623b46bd4041d`.
Remote comparison verifies the tested head is its ancestor. No release was
requested or performed.

## Dependency alignment remains a gate

Merged implementations can still resolve different trait crates. Inspect every
standalone consumer graph outside inherited local path patches before claiming
one contract. Confirm source IDs and features, not just package names or equal
source files. Keep the chosen zencodec family and paired zenpixels revision
consistent across direct and transitive edges.

Zenpipe's current full API profile still pins AVIF `a7638f6e`, whose animation
signature takes `Stop` where the shared contract requires `Pulse`.
[AVIF PR 75](https://github.com/imazen/zenavif/pull/75) at `b25f7604` matches and
is qualified by the core AV1 harness. Consumer adoption must move its direct
parse member with all AVIF patch entries and account for the corresponding
rav1d revision. The old and new parse trees are identical but their Cargo source
IDs differ. The existing full-profile failure stays visible until that graph
and its native controls pass.

Zenextras main `a5e792612145e0d544fa0243677d935fbcd487f0` replaces the TIFF, IVF
and WebM direct shared git edges with registry requirements and coordinated
root patches. Its root plus five fuzz graphs have one source per shared crate.
Default/all-feature tests, strict Clippy, public API snapshots and all five fuzz
target compilations pass locally. The new graph requires the paired canonical
P3 ICC test expectation; exact carrier and sample checks remain.
[Portable graph evidence](https://github.com/imazen/zenextras/blob/a5e792612145e0d544fa0243677d935fbcd487f0/docs/shared-contract-graph.md)
records the preserved initial failures. Its [platform CI](https://github.com/imazen/zenextras/actions/runs/40774551950) passed all twelve jobs, including Windows ARM64, macOS ARM64 and i686.

The shared pull-row extension is on main after [core CI](https://github.com/imazen/zencodec/actions/runs/40774823759), [native/guide checks](https://github.com/imazen/zencodec/actions/runs/40774823490) and [fuzz builds](https://github.com/imazen/zencodec/actions/runs/40774823515). Retained native multipass preparation and movable sessions passed focused native controls and static checks; cross-revision byte qualification and retained-job integration remain outstanding. These do not yet qualify the
single-policy production multipass driver or asynchronous row ownership.

## PNG review stack

[PNG PR 47](https://github.com/imazen/zenpng/pull/47) at `b5e67cf3` passed
[all-platform CI](https://github.com/imazen/zenpng/actions/runs/40664772244),
including ImageIO iDOT compatibility. It merged into the `checked-color`
review branch as `c81f04cff3780d4be99dd43496d383e348ce987b`. The combined
[PR 46](https://github.com/imazen/zenpng/pull/46) passed [combined CI](https://github.com/imazen/zenpng/actions/runs/40773313696)
and merged to main as `b536fce16fdac478cb1b318ee5ae0e52b9c51b33`. The tested
head is contained in that merge; publishing remains a separate release gate.
