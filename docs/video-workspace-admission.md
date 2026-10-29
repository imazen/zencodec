# Video admission before native work

```rust,ignore
let decoder = PolicyDecoder::try_new(
    Av1VideoDecoder::new,
    PolicySet::new(),
    None,
)?;
```

Default policy permits construction and checks each presentation before returning
its frame. Add a required workspace bound only when refusal is acceptable:

```rust,ignore
let decoder = PolicyDecoder::try_new(
    Av1VideoDecoder::new,
    PolicySet::new().with_workspace_limit(64 << 20),
    None,
)?;
```

This generic adapter has no qualified native workspace evidence. The second
example therefore refuses with a typed `UnmetRequirements` naming Workspace /
Unknown, before calling the factory. Even `u64::MAX` does not turn unknown into
proof. A callback cannot waive bound requirements. This provides predictable
refusal, not a successful bounded native decoder implementation.

Keep native preparation inside the factory. Capturing an already-created decoder
cannot undo its allocations. The existing `PolicyDecoder::new` remains useful for
wrapping an existing session; its policy governs subsequent operations only.
Both paths check bound and legacy workspace requirements before configure, send,
receive, end_input and reset. Refusal drops the retained native owner and is
terminal. No fabricated choice or callback decision is recorded for this early
failure; its named cause remains available through the error chain. Cancellation
is checked first without an extra poll in the adapter.

Other late frame/metadata decisions remain at publication. This workspace check
does not certify metadata completeness, opaque-data privacy or native references'
memory cost. Supporting success under a strict bound still needs trustworthy
native preparation, reference-storage and conversion accounting for the same
retained owner.

## Qualification

The before-fix regression at95caa13d fails because native configure succeeds.
The repaired controls exercise all five entry points, legacy and bound policies,
zero native/callback calls on refusal, owner cleanup, terminal state, named
unknown evidence, default factory success, factory errors and cancellation.
The late-refusal optional-Pulse control retains its assertions using descriptive
metadata; workspace refusal now has its separate earlier-effect matrix.

On edb9f787, the full media testkit and strict all-target Clippy pass. Two native
AV1 controls pass: strict unknown workspace never invokes the rav1d constructor;
default admission constructs/configures the real decoder and preserves a typed
native failure. The existing erased native error/trace control also passes.
Receipt: rc0,18s,peak RSS0.97GiB,minimum available10468MiB,peak load0.80.
This is validation resource usage, not a decoder workspace measurement.

Commands: `just media-workspace-admission-check` and
`just media-native-workspace-check`. Logs and exact before/after source exports:
`/home/lilith/tmp/zencodec-video-admission-20261029/`. The initial missing-lock
attempts and missing-std test-runner failure remain retained. No test was skipped
or behavioral threshold weakened.

API snapshots and the eight-crate compiler-derived catalog regenerate and pass
freshness checks. The migrated native guide example compiles. All24 source/site
checks and the complete browser suite pass (rc0,42s,peak RSS0.33GiB); all eight
Rustdoc crates build with warnings denied (rc0,6s,peak RSS0.32GiB). The first
site attempt lacked Node dependencies and built HTML; those prerequisites were
installed/built before rerunning the unchanged checks.
