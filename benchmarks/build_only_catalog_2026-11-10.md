# Build-only signature catalog

Qualified source: `7db9734e` (runtime/library API source unchanged from `63f136f0`).
The subsequent CI-only two-job cap matches the local qualification budget.

- Removed the entire committed `docs/code-api/` catalog: 2,235,177 bytes,
  including 1,480,283 bytes of JSON.
- Canonical API review now uses zenutils-apidoc 0.1.1 across all eight crates:
  24 default/feature/internal text snapshots, totaling 487,864 bytes.
- All 16 former per-crate raw signature/documentation payloads are byte-identical
  to the newly generated cache. The manifest gains source/tool/cache identity.
- The site builds its richer catalog in ignored `docs/.cache/code-api/` and
  publishes the browser data in `_site/code-api/`. Nothing in that cache is
  committed. Invalid prior caches are preserved beside it before regeneration.
- Cache checks cover source/manifests/READMEs, lockfiles, generator/compiler/tool
  identity, host platform and payload hashes. Mutation controls reject stale
  documentation, changed tools, missing payloads and corrupted data.
- The final isolated run passes zenutils snapshot checking, all-eight-crate
  strict Rustdoc, all site source tests and the complete browser suite.
  A second catalog invocation reports a valid cache hit without rebuilding.
- Actionlint v1.7.12 accepts the updated documentation workflow. CI installs
  the pinned nightly/CLI, checks the text snapshots explicitly, and caches only
  the generated browser catalog. Codec/runtime checks are unchanged.

Reproduce from an isolated export, through the normal resource wrapper:

```sh
python3 scripts/qualify-core-stack.py OUTPUT --revision REV --target TARGET --docs --docs-only
# Deliberate snapshot updates additionally use --refresh-api-snapshots.
```

Final wrapper receipt: `run-heavy: done rc=0 91s | peak-RSS 0.32GiB | min-avail 15973MiB | peak-load 1.15`
under a 4G cap with two jobs. This describes documentation qualification, not
codec performance. Initial migration qualification also passed (100s,
peak RSS0.40GiB). Prior snapshot/catalog source remains recoverable in Git.
