# Verified documentation cleanup before structural review

Base: PR208 `23ca16ff9af0a9fe92fb8c00853d998013f880eb`.
Qualified source: `c3d96458650d7d7bf177000c94dc93685bed7701`.

Changes correct the nonexistent per-format encode-trait description, identify
CodecSet's registry conveniences, narrow the prelude description, remove the
unqualified minimum-memory claim for push decode, distinguish required versus
preferred output in the guide, and document existing copying-allocation panics.
The new root example uses actual imports and the existing two-call pixel flow.
It does not claim automatic descriptive/container metadata retention.

Public signatures, visibility, defaults, executable Rust and tests are unchanged.
A diff check verifies that all changed Rust lines are documentation comments.
There are no proposed names presented as new importable types.

## Checks

The complete documentation wrapper exits rc0 in 98s: peak RSS 0.40 GiB,
minimum available 13447 MiB, peak load 2.80. The separate doctest wrapper exits
rc0 in 8s, peak RSS 0.40 GiB. These are qualification receipts, not performance
comparisons.

On r7900x, each command uses run-heavy with a 4G hard cap and two jobs; scratch
is disk-backed. Full logs/export/receipt remain in the external run directory
named `zencodec-pre-review-docs-20261110`.

```sh
python3 scripts/qualify-core-stack.py NEW_RUN --revision CANDIDATE \
  --target OWN_TARGET --docs --docs-only --browser-port FREE_PORT
# In the resulting isolated source export:
cargo test --workspace --all-features --doc --locked
```

- All eight zenutils public API snapshot sets match without regeneration.
- Strict Rustdoc generation passes for all eight crates.
- The regenerated build-only catalog, 29 source checks and the full guide browser
  checks pass, including the existing exact clipboard, signature, keyboard and
  390/768/1440px layout assertions.
- Workspace doctests pass: 34 passed, 39 existing ignored examples, including
  the added compiled root example. Ignore applicability is unchanged.
- Whitespace checks pass. No native runtime/benchmark rerun is claimed or needed
  to establish behavior of documentation-only edits.

The structural proposals in the architectural critique remain unimplemented:
no renames, root visibility changes, narrowed imports, trait split, new policy
composition behavior or checked-facade default was silently applied here.
